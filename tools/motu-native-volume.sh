#!/usr/bin/env bash
# Native PipeWire volume support for recover-motu-audio.sh. This file is not
# intended to be invoked directly; its caller supplies MOTU_SINK.

NATIVE_ACTIVATOR_SAMPLE_COUNT="480000"
NATIVE_ACTIVATOR_TIMEOUT_SECONDS="12"
SILENT_ACTIVATOR_PID=""

native_node_json() {
  pw-dump | jq -ce --arg node "$MOTU_SINK" '
    [ .[]
      | select(.type == "PipeWire:Interface:Node" and .info.props["node.name"] == $node)
      | {
          id,
          state: .info.state,
          channels: .info.props["audio.channels"],
          soft_mute: (.info.params.Props[0].softMute // null),
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
  # pw-cli echoes the parsed Props object to stderr even if the node later
  # rejects it. Keep other stderr lines so command failures remain actionable;
  # the readback below is the authoritative result in either case.
  pw-cli set-param "$node_id" Props "{ softMute = false, softVolumes = [ $channel_volumes ] }" 2>&1 | awk '
    /^Object: size [0-9]+, type Spa:Pod:Object:Param:Props / { dumping_props = 1; next }
    dumping_props && /^[[:space:]]/ { next }
    { dumping_props = 0; print > "/dev/stderr" }
  '
}

native_soft_volumes_are_full_scale() {
  local node_json channel_count

  node_json="$(native_node_json)" || return 1
  channel_count="$(jq -r '.channels' <<< "$node_json")"
  [[ "$channel_count" =~ ^[1-9][0-9]*$ ]] || return 1
  jq -e --argjson channel_count "$channel_count" '
    (.soft_mute == false)
    and (.soft_volumes | type == "array")
    and (length == $channel_count)
    and all(.[]; . == 1)
  ' <<< "$node_json" >/dev/null
}

native_node_state() {
  local node_json

  node_json="$(native_node_json)" || return 1
  jq -r '.state' <<< "$node_json"
}

native_node_is_active() {
  case "$(native_node_state)" in
    running|idle) return 0 ;;
    *) return 1 ;;
  esac
}

wait_for_native_full_scale_volume() {
  local attempts="${1:-20}"
  local observed_full_scale=false

  for _ in $(seq 1 "$attempts"); do
    if native_soft_volumes_are_full_scale; then
      observed_full_scale=true
    elif [ "$observed_full_scale" = true ]; then
      echo "848 native soft volumes did not remain full scale" >&2
      return 1
    fi
    sleep 0.2
  done

  if [ "$observed_full_scale" = true ]; then
    return 0
  fi
  echo "848 native soft volumes did not become full scale" >&2
  return 1
}

start_silent_motu_activator() {
  # A suspended ALSA node accepts Props syntax but discards dynamic softVolumes.
  # Feed the MOTU sink zero-valued PCM directly. Waking VirtualSink is not
  # sufficient because its output stream is deliberately passive while idle.
  timeout --signal=TERM --kill-after=1s "$NATIVE_ACTIVATOR_TIMEOUT_SECONDS"s \
    pw-cat --playback --raw --format s16 --rate 48000 --channels 2 \
      --target "$MOTU_SINK" --volume 1 \
      --sample-count "$NATIVE_ACTIVATOR_SAMPLE_COUNT" /dev/zero >/dev/null 2>&1 &
  SILENT_ACTIVATOR_PID="$!"
}

stop_silent_motu_activator() {
  if [ -n "$SILENT_ACTIVATOR_PID" ]; then
    kill "$SILENT_ACTIVATOR_PID" 2>/dev/null || true
    wait "$SILENT_ACTIVATOR_PID" 2>/dev/null || true
    SILENT_ACTIVATOR_PID=""
  fi
}

wait_for_active_native_path() {
  local attempts="${1:-20}"

  for _ in $(seq 1 "$attempts"); do
    if ! kill -0 "$SILENT_ACTIVATOR_PID" 2>/dev/null; then
      wait "$SILENT_ACTIVATOR_PID" || true
      SILENT_ACTIVATOR_PID=""
      echo "The silent MOTU activation stream exited before the 848 became active" >&2
      return 1
    fi
    if native_node_is_active; then
      return 0
    fi
    sleep 0.2
  done

  echo "The silent stream did not activate the 848 playback path" >&2
  return 1
}

wait_for_silent_motu_activator() {
  [ -n "$SILENT_ACTIVATOR_PID" ] || return 1
  if ! wait "$SILENT_ACTIVATOR_PID"; then
    SILENT_ACTIVATOR_PID=""
    echo "The silent MOTU activation stream exited unexpectedly" >&2
    return 1
  fi
  SILENT_ACTIVATOR_PID=""
}

wait_for_native_node_to_settle() {
  local attempts="${1:-25}"
  local previous_state=""
  local state

  for _ in $(seq 1 "$attempts"); do
    state="$(native_node_state)" || state=""
    case "$state" in
      running|idle|suspended)
        if [ "$state" = "$previous_state" ]; then
          return 0
        fi
        previous_state="$state"
        ;;
      *) previous_state="" ;;
    esac
    sleep 0.2
  done

  echo "848 playback node did not reach a stable usable state after the silent activation stream" >&2
  return 1
}
