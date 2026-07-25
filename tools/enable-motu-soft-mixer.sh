#!/usr/bin/env bash
set -euo pipefail

MOTU_CARD="alsa_card.usb-MOTU_848_848AFEB9E2-00"
RULE_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/wireplumber/wireplumber.conf.d"
RULE_FILE="$RULE_DIR/90-motu-848-soft-mixer.conf"

usage() {
  cat <<'EOF'
Usage: enable-motu-soft-mixer.sh [--check|--install|--remove]

Installs a per-device WirePlumber rule that keeps PipeWire volume control in
software for this MOTU 848. Installation or removal restarts WirePlumber and
briefly interrupts audio streams.

Options:
  --check    Report whether the rule is installed (default).
  --install  Write the rule and restart WirePlumber.
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
  systemctl --user restart wireplumber
  systemctl --user is-active --quiet wireplumber
}

restore_rule() {
  local backup_file="$1"

  if [ -n "$backup_file" ]; then
    mv "$backup_file" "$RULE_FILE"
  else
    rm -f "$RULE_FILE"
  fi
  if ! restart_wireplumber; then
    echo "Could not restart WirePlumber after restoring $RULE_FILE" >&2
  fi
}

write_rule() {
  cat >"$RULE_FILE" <<EOF
# The 848 has 128 playback channels but only 16 UAC playback-volume controls.
# This ACP device rule prevents PipeWire from driving that mismatched control.
monitor.alsa.rules = [
  {
    matches = [
      {
        device.name = "$MOTU_CARD"
      }
    ]
    actions = {
      update-props = {
        api.alsa.soft-mixer = true
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
  if [ ! -f "$RULE_FILE" ]; then
    echo "MOTU 848 soft-mixer rule is not installed"
  else
    echo "MOTU 848 soft-mixer rule is installed: $RULE_FILE"
  fi
  exit 0
fi

require_cmd systemctl
mkdir -p "$RULE_DIR"
backup_file=""
if [ -f "$RULE_FILE" ]; then
  backup_file="$(mktemp "$RULE_DIR/.90-motu-848-soft-mixer.XXXXXX")"
  cp -p "$RULE_FILE" "$backup_file"
fi

if [ "$mode" = "install" ]; then
  write_rule
  if ! restart_wireplumber; then
    echo "Could not restart WirePlumber; restoring the previous configuration" >&2
    restore_rule "$backup_file"
    exit 1
  fi
  rm -f "$backup_file"
  echo "Installed $RULE_FILE and restarted WirePlumber."
  exit 0
fi

if [ ! -f "$RULE_FILE" ]; then
  echo "MOTU 848 soft-mixer rule is not installed"
  exit 0
fi
rm "$RULE_FILE"
if ! restart_wireplumber; then
  echo "Could not restart WirePlumber; restoring the previous configuration" >&2
  restore_rule "$backup_file"
  exit 1
fi
rm -f "$backup_file"
echo "Removed $RULE_FILE and restarted WirePlumber."
