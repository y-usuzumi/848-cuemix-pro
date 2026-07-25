#!/usr/bin/env bash
set -euo pipefail

MOTU_SINK="alsa_output.usb-MOTU_848_848AFEB9E2-00.multichannel-output"
RULE_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/wireplumber/wireplumber.conf.d"
RULE_FILE="$RULE_DIR/90-motu-848-volume-lock.conf"
PREVIOUS_RULE_FILE="$RULE_FILE.pre-cuemix-848"
MANAGED_MARKER="# Managed by cuemix-848: MOTU volume lock"
LEGACY_MARKER="# The 848's physical knob owns monitor level. Its 128-channel PipeWire adapter"

transaction_active=false
transaction_had_rule=false
transaction_backup=""
temporary_rule=""
previous_rule_created=false

usage() {
  cat <<'EOF'
Usage: enable-motu-volume-lock.sh [--check|--install|--remove]

Installs a MOTU-only WirePlumber rule that prevents desktop volume clients
from changing the 848 Multichannel sink. Installation or removal restarts
WirePlumber and briefly interrupts audio streams.

Options:
  --check    Report whether the rule is installed and active (default).
  --install  Write the rule, restart WirePlumber, and verify the live node.
  --remove   Remove the rule and restart WirePlumber.
  -h, --help Show this help.
EOF
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

restart_wireplumber() {
  if ! systemctl --user restart wireplumber; then
    return 1
  fi
  systemctl --user is-active --quiet wireplumber
}

rule_is_managed() {
  [ -f "$RULE_FILE" ] || return 1
  grep -Fqx "$MANAGED_MARKER" "$RULE_FILE" \
    || grep -Fqx "$LEGACY_MARKER" "$RULE_FILE"
}

live_node_has_volume_lock() {
  pw-dump | jq -e --arg node "$MOTU_SINK" '
    [ .[] | select(
      .type == "PipeWire:Interface:Node"
      and .info.props["node.name"] == $node
    ) ]
    | length == 1
      and .[0].info.props["channelmix.lock-volumes"] == true
      and .[0].info.props["state.restore-props"] == false
  ' >/dev/null
}

wait_for_live_volume_lock() {
  local attempts="${1:-25}"

  for _ in $(seq 1 "$attempts"); do
    if live_node_has_volume_lock; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

restore_transaction_file() {
  if [ "$transaction_had_rule" = true ]; then
    if [ ! -f "$transaction_backup" ]; then
      echo "Cannot restore the previous rule: transaction backup is missing" >&2
      return 1
    fi
    temporary_rule="$(mktemp "$RULE_DIR/.90-motu-848-volume-lock.restore.XXXXXX")"
    cp -p "$transaction_backup" "$temporary_rule"
    mv -f "$temporary_rule" "$RULE_FILE"
    temporary_rule=""
  else
    rm -f "$RULE_FILE"
  fi
  if [ "$previous_rule_created" = true ]; then
    rm -f "$PREVIOUS_RULE_FILE"
  fi
}

cleanup_transaction() {
  local status="$?"

  if [ -n "$temporary_rule" ] \
    && { [ "$transaction_active" != true ] || [ "$temporary_rule" != "$transaction_backup" ]; }; then
    rm -f "$temporary_rule"
  fi
  if [ "$transaction_active" = true ]; then
    if restore_transaction_file; then
      if restart_wireplumber; then
        echo "Interrupted; restored the previous WirePlumber rule" >&2
      else
        echo "Interrupted; restored the previous rule on disk but could not restart WirePlumber" >&2
      fi
      commit_transaction
    else
      echo "Interrupted; automatic rule restoration failed" >&2
    fi
  fi
  return "$status"
}

begin_transaction() {
  if [ -f "$RULE_FILE" ]; then
    temporary_rule="$(mktemp "$RULE_DIR/.90-motu-848-volume-lock.rollback.XXXXXX")"
    cp -p "$RULE_FILE" "$temporary_rule"
    transaction_backup="$temporary_rule"
    transaction_had_rule=true
  fi
  transaction_active=true
  temporary_rule=""
}

rollback_transaction() {
  local restart_status=0

  if ! restore_transaction_file; then
    return 1
  fi
  if ! restart_wireplumber; then
    echo "Could not restart WirePlumber after restoring $RULE_FILE" >&2
    restart_status=1
  fi
  commit_transaction
  return "$restart_status"
}

commit_transaction() {
  transaction_active=false
  if [ -n "$transaction_backup" ]; then
    rm -f "$transaction_backup"
    transaction_backup=""
  fi
}

trap cleanup_transaction EXIT
trap 'exit 128' HUP INT TERM

write_rule() {
  local destination="$1"

  cat >"$destination" <<EOF
$MANAGED_MARKER
# The 848's physical knob owns monitor level. Its 128-channel PipeWire adapter
# must not accept desktop sink-volume changes, which can silence DSP playback.
monitor.alsa.rules = [
  {
    matches = [
      {
        node.name = "~alsa_output[.]usb-MOTU_848_.*[.]multichannel-output"
      }
    ]
    actions = {
      update-props = {
        channelmix.lock-volumes = true
        state.restore-props = false
      }
    }
  }
]
EOF
}

mode="check"
if [ "$#" -gt 1 ]; then
  echo "Expected at most one option" >&2
  usage >&2
  exit 2
fi
case "${1:---check}" in
  --check)
    ;;
  --install|--remove)
    mode="${1#--}"
    ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    echo "Unknown option: $1" >&2
    usage >&2
    exit 2
    ;;
esac

if [ "$mode" = "check" ]; then
  if ! rule_is_managed; then
    echo "MOTU 848 volume-lock rule is not installed"
    exit 1
  fi
  require_cmd jq
  require_cmd pw-dump
  if live_node_has_volume_lock; then
    echo "MOTU 848 volume-lock rule is installed and active: $RULE_FILE"
    exit 0
  fi
  echo "MOTU 848 volume-lock rule is installed but not active: $RULE_FILE" >&2
  exit 1
fi

require_cmd systemctl
mkdir -p "$RULE_DIR"

if [ "$mode" = "install" ]; then
  preserve_previous_rule=false

  require_cmd jq
  require_cmd pw-dump
  if [ -f "$RULE_FILE" ] && ! rule_is_managed; then
    if [ -e "$PREVIOUS_RULE_FILE" ]; then
      echo "Cannot preserve $RULE_FILE: backup already exists at $PREVIOUS_RULE_FILE" >&2
      exit 1
    fi
    preserve_previous_rule=true
  fi
  begin_transaction
  if [ "$preserve_previous_rule" = true ]; then
    temporary_rule="$(mktemp "$RULE_DIR/.90-motu-848-volume-lock.preserve.XXXXXX")"
    cp -p "$RULE_FILE" "$temporary_rule"
    previous_rule_created=true
    mv -f "$temporary_rule" "$PREVIOUS_RULE_FILE"
    temporary_rule=""
  fi
  temporary_rule="$(mktemp "$RULE_DIR/.90-motu-848-volume-lock.new.XXXXXX")"
  write_rule "$temporary_rule"
  mv -f "$temporary_rule" "$RULE_FILE"
  temporary_rule=""
  if ! restart_wireplumber; then
    echo "Could not restart WirePlumber; restoring the previous configuration" >&2
    rollback_transaction || true
    exit 1
  fi
  if ! wait_for_live_volume_lock; then
    echo "MOTU 848 volume-lock rule did not take effect; restoring the previous configuration" >&2
    rollback_transaction || true
    exit 1
  fi
  commit_transaction
  echo "Installed $RULE_FILE; the live MOTU sink now rejects volume updates."
  exit 0
fi

if [ ! -f "$RULE_FILE" ]; then
  echo "MOTU 848 volume-lock rule is not installed"
  exit 0
fi
if ! rule_is_managed; then
  echo "Refusing to remove unmanaged WirePlumber rule: $RULE_FILE" >&2
  exit 1
fi

begin_transaction
if [ -f "$PREVIOUS_RULE_FILE" ]; then
  temporary_rule="$(mktemp "$RULE_DIR/.90-motu-848-volume-lock.restore.XXXXXX")"
  cp -p "$PREVIOUS_RULE_FILE" "$temporary_rule"
  mv -f "$temporary_rule" "$RULE_FILE"
  temporary_rule=""
else
  rm "$RULE_FILE"
fi
if ! restart_wireplumber; then
  echo "Could not restart WirePlumber; restoring the managed rule" >&2
  rollback_transaction || true
  exit 1
fi
commit_transaction
rm -f "$PREVIOUS_RULE_FILE"
echo "Removed the MOTU 848 volume lock and restarted WirePlumber."
