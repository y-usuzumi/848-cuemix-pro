#!/usr/bin/env bash
set -euo pipefail

MOTU_CARD="alsa_card.usb-MOTU_848_848AFEB9E2-00"
MOTU_SINK="alsa_output.usb-MOTU_848_848AFEB9E2-00.multichannel-output"
VIRTUAL_SINK="VirtualSink"
LOOPBACK_NODE="VirtualSink.output"

CHECK_ONLY=false
DISABLE_WIREPLUMBER_ROUTE_RESTORE=false
USE_NATIVE_VOLUME=false

usage() {
  cat <<'EOF'
Usage: recover-motu-audio.sh [--check] [--disable-wireplumber-route-restore]
                             [--native-volume]

Restores the intended VirtualSink loopback and verifies that the 848 PipeWire
sink remains at 100%. It does not reset the 848 card profile by default.

Options:
  --check                         Report the 848 profile, USB mixer, and
                                  PipeWire sink state without changing audio.
  --disable-wireplumber-route-restore
                                  Disable WirePlumber device-route restoration
                                  for this WirePlumber session. This affects all
                                  devices until WirePlumber restarts.
  --native-volume                 Set every native PipeWire channel directly,
                                  bypassing the 32-channel Pulse view. This is
                                  intended for the 848's 128-channel sink.
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

active_profile() {
  card_details | awk '/^[[:space:]]*Active Profile: / { print $3; exit }'
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

native_node_json() {
  pw-dump | jq -ce --arg node "$MOTU_SINK" '
    [ .[]
      | select(.type == "PipeWire:Interface:Node" and .info.props["node.name"] == $node)
      | {
          id,
          channels: .info.props["audio.channels"],
          soft_volumes: (.info.params.Props[0].softVolumes // [])
        }
    ]
    | if length == 1 then .[0] else error("expected one matching PipeWire node") end
  '
}

native_node_matches() {
  local expected_id="$1"
  local expected_channels="$2"
  local node_json

  node_json="$(native_node_json)" || return 1
  [ "$(jq -r '.id' <<< "$node_json")" = "$expected_id" ] \
    && [ "$(jq -r '.channels' <<< "$node_json")" = "$expected_channels" ]
}

set_native_full_scale_soft_volume() {
  local node_json node_id channel_count channel_volumes

  node_json="$(native_node_json)" || {
    echo "Could not find the native 848 PipeWire node" >&2
    return 1
  }
  node_id="$(jq -r '.id' <<< "$node_json")"
  channel_count="$(jq -r '.channels' <<< "$node_json")"
  if ! [[ "$channel_count" =~ ^[1-9][0-9]*$ ]] || [ "$channel_count" -ne 128 ]; then
    echo "Expected a 128-channel native 848 PipeWire node, got: $channel_count" >&2
    return 1
  fi
  if ! native_node_matches "$node_id" "$channel_count"; then
    echo "848 PipeWire node changed before the native volume write" >&2
    return 1
  fi

  channel_volumes="$(printf '1.0 %.0s' $(seq 1 "$channel_count"))"
  pw-cli set-param "$node_id" Props "{ softMute = false, softVolumes = [ $channel_volumes ] }"
}

native_soft_volumes_are_full_scale() {
  local node_json channel_count

  node_json="$(native_node_json)" || return 1
  channel_count="$(jq -r '.channels' <<< "$node_json")"
  [[ "$channel_count" =~ ^[1-9][0-9]*$ ]] || return 1
  jq -e --argjson channel_count "$channel_count" '
    (.soft_volumes | type == "array")
    and (length == $channel_count)
    and all(.[]; . == 1)
  ' <<< "$node_json" >/dev/null
}

wait_for_native_full_scale_volume() {
  local attempts="${1:-20}"

  for _ in $(seq 1 "$attempts"); do
    if native_soft_volumes_are_full_scale; then
      return 0
    fi
    sleep 0.2
  done

  echo "848 native soft volumes did not remain full scale" >&2
  return 1
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

  if ! usb_audio_out_is_full_scale "$card"; then
    echo "848 USB Audio Out did not remain full scale and unmuted" >&2
    status=1
  fi
  if [ "$USE_NATIVE_VOLUME" = true ]; then
    if ! native_soft_volumes_are_full_scale; then
      echo "848 native soft volumes did not remain full scale" >&2
      status=1
    fi
  elif ! sink_is_full_scale "$MOTU_SINK" || ! sink_is_unmuted "$MOTU_SINK"; then
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

  # Link the established playback path before changing volume. The 848 node
  # otherwise stays suspended and discards dynamic software-volume parameters.
  echo "Routing VirtualSink.output to the MOTU..."
  route_loopback_to_motu "$loopback_id"

  echo "Restoring the 848 USB and PipeWire playback volume..."
  amixer -c "$card" sset 'Audio Out' 100% unmute >/dev/null
  pactl set-sink-mute "$MOTU_SINK" 0
  if [ "$USE_NATIVE_VOLUME" = true ]; then
    echo "Setting all native PipeWire soft playback channels..."
    set_native_full_scale_soft_volume
    wait_for_native_full_scale_volume
  else
    pactl set-sink-volume "$MOTU_SINK" 100%
    wait_for_full_scale_sink
  fi

  verify_recovery "$card" "$loopback_id"
  echo "Recovered and verified."
}

parse_args() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --check)
        CHECK_ONLY=true
        ;;
      --disable-wireplumber-route-restore)
        DISABLE_WIREPLUMBER_ROUTE_RESTORE=true
        ;;
      --native-volume)
        USE_NATIVE_VOLUME=true
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
  if [ "$CHECK_ONLY" = true ] && { [ "$DISABLE_WIREPLUMBER_ROUTE_RESTORE" = true ] || [ "$USE_NATIVE_VOLUME" = true ]; }; then
    echo "Recovery options require normal recovery mode, not --check" >&2
    exit 2
  fi
  require_cmd amixer
  require_cmd pactl
  if [ "$DISABLE_WIREPLUMBER_ROUTE_RESTORE" = true ]; then
    require_cmd wpctl
  fi
  if [ "$USE_NATIVE_VOLUME" = true ]; then
    require_cmd jq
    require_cmd pw-cli
    require_cmd pw-dump
  fi

  if [ "$CHECK_ONLY" = true ]; then
    check
  else
    recover
  fi
}

main "$@"
