#!/usr/bin/env bash
set -euo pipefail

MOTU_CARD="alsa_card.usb-MOTU_848_848AFEB9E2-00"
MOTU_PROFILE="output:multichannel-output+input:multichannel-input"
MOTU_SINK="alsa_output.usb-MOTU_848_848AFEB9E2-00.multichannel-output"
VIRTUAL_SINK="VirtualSink"
LOOPBACK_NODE="VirtualSink.output"

CHECK_ONLY=false
DISABLE_PULSE_DEVICE_RESTORE=false
DISABLE_WIREPLUMBER_ROUTE_RESTORE=false

usage() {
  cat <<'EOF'
Usage: recover-motu-audio.sh [--check] [--disable-pulse-device-restore]
                             [--disable-wireplumber-route-restore]

Restores the intended VirtualSink loopback and verifies that the 848 PipeWire
sink remains at 100%. It does not reset the 848 card profile by default.

Options:
  --check                         Report the 848 profile, USB mixer, and
                                  PipeWire sink state without changing audio.
  --disable-pulse-device-restore  Reload PipeWire Pulse's device-restore
                                  module with volume restoration disabled for
                                  this PipeWire Pulse session. This affects all
                                  Pulse devices until pipewire-pulse restarts.
  --disable-wireplumber-route-restore
                                  Disable WirePlumber device-route restoration
                                  for this WirePlumber session. This affects all
                                  devices until WirePlumber restarts.
  -h, --help                      Show this help.
EOF
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

card_details() {
  pactl list cards | awk -v card="$MOTU_CARD" '
    /^Card #/ {
      if (in_card) exit
      in_card = 0
    }
    /^[[:space:]]*Name: / {
      in_card = $2 == card
    }
    in_card { print }
  '
}

alsa_card_index() {
  card_details | awk '/^[[:space:]]*api\.alsa\.card =/ {
    value = $3
    gsub(/"/, "", value)
    print value
    exit
  }'
}

profile_is_available() {
  card_details | awk -v profile="$MOTU_PROFILE" '$1 == profile ":" { found = 1 } END { exit !found }'
}

active_profile() {
  card_details | awk '/^[[:space:]]*Active Profile: / { print $3; exit }'
}

profile_is_active() {
  [ "$(active_profile)" = "$MOTU_PROFILE" ]
}

sink_exists() {
  pactl list sinks short | awk '{ print $2 }' | grep -Fxq "$1"
}

sink_is_full_scale() {
  local sink="$1"

  pactl get-sink-volume "$sink" | awk '
    {
      for (field = 2; field <= NF; field += 1) {
        if ($(field - 1) == "/" && $field ~ /^[0-9]+%$/) {
          saw_volume = 1
          if ($field != "100%") bad_volume = 1
        }
      }
    }
    END { exit !saw_volume || bad_volume }
  '
}

sink_volume_summary() {
  local sink="$1"

  pactl get-sink-volume "$sink" | tr '\n' ' ' | sed 's/[[:space:]]\+/ /g'
}

sink_is_unmuted() {
  local sink="$1"

  pactl get-sink-mute "$sink" | awk '/^Mute: no$/ { found = 1 } END { exit !found }'
}

wait_for_full_scale_sink() {
  local attempts="${1:-20}"

  # The faulty route briefly accepts 100% before restoring zero. Check over a
  # few seconds so a transient write is not mistaken for a recovered sink.
  for _ in $(seq 1 "$attempts"); do
    if ! sink_is_full_scale "$MOTU_SINK"; then
      echo "848 PipeWire sink returned below 100%: $(sink_volume_summary "$MOTU_SINK")" >&2
      echo "Try --disable-wireplumber-route-restore if WirePlumber is restoring this route." >&2
      return 1
    fi
    sleep 0.2
  done
}

usb_audio_out_is_full_scale() {
  local card="$1"
  amixer -c "$card" sget 'Audio Out' | awk '
    /Playback [0-9]+ \[[0-9]+%\] .*\[(on|off)\]/ {
      saw_channel = 1
      if ($0 !~ /\[100%\].*\[on\]/) bad_channel = 1
    }
    END { exit !saw_channel || bad_channel }
  '
}

sink_input_ids_with_property() {
  local key="$1"
  local value="$2"

  pactl list sink-inputs | awk -v key="$key" -v value="$value" '
    /^Sink Input #/ {
      if (id != "" && block ~ key " = \"" value "\"") print id
      id = $3
      sub(/^#/, "", id)
      block = ""
    }
    {
      block = block $0 "\n"
    }
    END {
      if (id != "" && block ~ key " = \"" value "\"") print id
    }
  '
}

sink_id() {
  local sink="$1"

  pactl list sinks short | awk -v sink="$sink" '$2 == sink { print $1; exit }'
}

sink_input_is_routed_to() {
  local input_id="$1"
  local sink="$2"
  local target_sink_id

  target_sink_id="$(sink_id "$sink")"
  [ -n "$target_sink_id" ] || return 1
  pactl list sink-inputs short | awk -v input_id="$input_id" -v target_sink_id="$target_sink_id" '
    $1 == input_id && $2 == target_sink_id { found = 1 }
    END { exit !found }
  '
}

sink_input_destination() {
  local input_id="$1"
  local destination_id

  destination_id="$(pactl list sink-inputs short | awk -v input_id="$input_id" '$1 == input_id { print $2; exit }')"
  [ -n "$destination_id" ] || return 1
  pactl list sinks short | awk -v destination_id="$destination_id" '$1 == destination_id { print $2; exit }'
}

sink_input_is_full_scale() {
  local input_id="$1"

  pactl get-sink-input-volume "$input_id" | awk '
    {
      for (field = 2; field <= NF; field += 1) {
        if ($(field - 1) == "/" && $field ~ /^[0-9]+%$/) {
          saw_volume = 1
          if ($field != "100%") bad_volume = 1
        }
      }
    }
    END { exit !saw_volume || bad_volume }
  '
}

sink_input_is_unmuted() {
  local input_id="$1"

  pactl get-sink-input-mute "$input_id" | awk '/^Mute: no$/ { found = 1 } END { exit !found }'
}

single_loopback_sink_input_id() {
  local loopback_ids

  loopback_ids="$(sink_input_ids_with_property "node.name" "$LOOPBACK_NODE")"
  if [ -z "$loopback_ids" ] || [ "$(printf '%s\n' "$loopback_ids" | wc -l)" -ne 1 ]; then
    echo "Expected exactly one VirtualSink loopback stream: $LOOPBACK_NODE" >&2
    return 1
  fi
  printf '%s\n' "$loopback_ids"
}

route_loopback_to_motu() {
  local input_id="$1"
  local previous_sink

  previous_sink="$(sink_input_destination "$input_id")"
  if [ -z "$previous_sink" ]; then
    echo "Could not determine the VirtualSink loopback destination" >&2
    return 1
  fi

  # This is the one graph edge owned by the recovery setup. Leave application
  # streams and the desktop default sink alone.
  if ! pactl move-sink-input "$input_id" "$MOTU_SINK" \
    || ! pactl set-sink-input-mute "$input_id" 0 \
    || ! pactl set-sink-input-volume "$input_id" 100%; then
    echo "Could not restore the VirtualSink loopback; returning it to $previous_sink" >&2
    pactl move-sink-input "$input_id" "$previous_sink" || true
    return 1
  fi
}

module_arguments() {
  local module_id="$1"

  pactl list modules | awk -v module_id="$module_id" '
    /^Module #[0-9]+/ {
      if (in_module) exit
      in_module = $2 == "#" module_id
      next
    }
    in_module && /^[[:space:]]*Argument:/ {
      sub(/^[[:space:]]*Argument:[[:space:]]*/, "")
      print
      exit
    }
  '
}

disable_pulse_device_volume_restore() {
  local module_id current_arguments new_module_id
  local -a current_argument_tokens=()
  local -a replacement_arguments=()
  local argument found_volume_setting=false

  module_id="$(pactl list modules short | awk '$2 == "module-device-restore" { print $1; exit }')"
  if [ -z "$module_id" ]; then
    pactl load-module module-device-restore restore_volume=false >/dev/null
    return
  fi

  current_arguments="$(module_arguments "$module_id")"
  if [ -n "$current_arguments" ]; then
    # pactl prints module arguments as plain key=value tokens. Preserve every
    # existing option except the one this diagnostic switch intentionally alters.
    read -r -a current_argument_tokens <<< "$current_arguments"
  fi
  for argument in "${current_argument_tokens[@]}"; do
    case "$argument" in
      restore_volume=*)
        argument="restore_volume=false"
        found_volume_setting=true
        ;;
    esac
    replacement_arguments+=("$argument")
  done
  if [ "$found_volume_setting" = false ]; then
    replacement_arguments+=("restore_volume=false")
  fi

  # Load before unloading the original module. A rejected replacement therefore
  # leaves the user's current Pulse restoration behavior intact.
  if ! new_module_id="$(pactl load-module module-device-restore "${replacement_arguments[@]}")"; then
    echo "Could not load PipeWire Pulse device restore with volume restoration disabled" >&2
    return 1
  fi
  if ! pactl unload-module "$module_id"; then
    echo "Could not replace the existing PipeWire Pulse device-restore module" >&2
    pactl unload-module "$new_module_id" || true
    return 1
  fi
}

disable_wireplumber_route_restore() {
  wpctl settings device.restore-routes false
}

preflight() {
  local require_virtual_sink="$1"
  local card

  card="$(alsa_card_index)"
  if [ -z "$card" ]; then
    echo "MOTU card not found: $MOTU_CARD" >&2
    return 1
  fi
  if ! profile_is_available; then
    echo "MOTU profile is unavailable: $MOTU_PROFILE" >&2
    return 1
  fi
  if [ "$require_virtual_sink" = true ] && ! sink_exists "$VIRTUAL_SINK"; then
    echo "Virtual sink not found: $VIRTUAL_SINK" >&2
    return 1
  fi
  if ! amixer -c "$card" sget 'Audio Out' >/dev/null; then
    echo "MOTU USB Audio Out control is unavailable on ALSA card $card" >&2
    return 1
  fi
}

check() {
  local card
  local status=0

  preflight false
  card="$(alsa_card_index)"
  echo "MOTU ALSA card: $card"
  echo "MOTU active profile: $(active_profile)"
  echo "MOTU PipeWire sink: $MOTU_SINK"
  if ! profile_is_active; then
    echo "MOTU profile: expected $MOTU_PROFILE" >&2
    status=1
  fi
  if usb_audio_out_is_full_scale "$card"; then
    echo "MOTU USB Audio Out: 100% and unmuted"
  else
    echo "MOTU USB Audio Out: not full scale or muted" >&2
    status=1
  fi
  if ! sink_exists "$MOTU_SINK"; then
    echo "MOTU PipeWire sink: unavailable" >&2
    status=1
  elif sink_is_full_scale "$MOTU_SINK" && sink_is_unmuted "$MOTU_SINK"; then
    echo "MOTU PipeWire volume: $(sink_volume_summary "$MOTU_SINK")"
    echo "MOTU PipeWire sink: 100% and unmuted"
  else
    echo "MOTU PipeWire volume: $(sink_volume_summary "$MOTU_SINK")"
    echo "MOTU PipeWire sink: not 100% and unmuted; recovery is needed" >&2
    status=1
  fi
  return "$status"
}

verify_recovery() {
  local card="$1"
  local input_id="$2"
  local status=0

  if ! profile_is_active; then
    echo "848 profile did not remain active: $(active_profile)" >&2
    status=1
  fi
  if ! usb_audio_out_is_full_scale "$card"; then
    echo "848 USB Audio Out did not remain full scale and unmuted" >&2
    status=1
  fi
  if ! sink_is_full_scale "$MOTU_SINK" || ! sink_is_unmuted "$MOTU_SINK"; then
    echo "848 PipeWire sink did not remain full scale and unmuted" >&2
    status=1
  fi
  if ! sink_input_is_routed_to "$input_id" "$MOTU_SINK"; then
    echo "VirtualSink loopback is not routed to the 848" >&2
    status=1
  fi
  if ! sink_input_is_full_scale "$input_id" || ! sink_input_is_unmuted "$input_id"; then
    echo "VirtualSink loopback did not remain full scale and unmuted" >&2
    status=1
  fi

  return "$status"
}

recover() {
  local card
  local loopback_id

  preflight true
  card="$(alsa_card_index)"

  if [ "$DISABLE_WIREPLUMBER_ROUTE_RESTORE" = true ]; then
    echo "Disabling WirePlumber device-route restoration for this session..."
    disable_wireplumber_route_restore
  fi

  if ! sink_exists "$MOTU_SINK"; then
    echo "848 PipeWire sink is unavailable; reconnect the device or select its profile through the system audio settings" >&2
    return 1
  fi
  loopback_id="$(single_loopback_sink_input_id)"

  if [ "$DISABLE_PULSE_DEVICE_RESTORE" = true ]; then
    echo "Disabling PipeWire Pulse device-volume restoration for this session..."
    disable_pulse_device_volume_restore
  fi

  echo "Restoring the 848 USB and PipeWire playback volume..."
  amixer -c "$card" sset 'Audio Out' 100% unmute >/dev/null
  pactl set-sink-mute "$MOTU_SINK" 0
  pactl set-sink-volume "$MOTU_SINK" 100%
  wait_for_full_scale_sink

  echo "Routing VirtualSink.output to the MOTU..."
  route_loopback_to_motu "$loopback_id"

  verify_recovery "$card" "$loopback_id"
  echo "Recovered and verified."
}

parse_args() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --check)
        CHECK_ONLY=true
        ;;
      --disable-pulse-device-restore)
        DISABLE_PULSE_DEVICE_RESTORE=true
        ;;
      --disable-wireplumber-route-restore)
        DISABLE_WIREPLUMBER_ROUTE_RESTORE=true
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
    shift
  done
}

main() {
  parse_args "$@"
  if [ "$CHECK_ONLY" = true ] && { [ "$DISABLE_PULSE_DEVICE_RESTORE" = true ] || [ "$DISABLE_WIREPLUMBER_ROUTE_RESTORE" = true ]; }; then
    echo "Recovery options require normal recovery mode, not --check" >&2
    exit 2
  fi
  require_cmd amixer
  require_cmd pactl
  if [ "$DISABLE_WIREPLUMBER_ROUTE_RESTORE" = true ]; then
    require_cmd wpctl
  fi

  if [ "$CHECK_ONLY" = true ]; then
    check
  else
    recover
  fi
}

main "$@"
