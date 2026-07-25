#!/usr/bin/env bash
# Native PipeWire DSP support for recover-motu-audio.sh. This file is sourced
# by the main script; its caller supplies MOTU_SINK and LOOPBACK_NODE.

native_node_json() {
  pw-dump | jq -ce --arg node "$MOTU_SINK" '
    [ .[]
      | select(.type == "PipeWire:Interface:Node" and .info.props["node.name"] == $node)
      | {
          id,
          state: .info.state,
          input_ports: .info["n-input-ports"],
          channels: .info.props["audio.channels"],
          port_config: (.info.params.PortConfig[0] // {}),
          props_present: ((.info.params.Props // []) | length > 0),
          soft_mute_present: ((.info.params.Props[0] // {}) | has("softMute")),
          soft_volumes_present: ((.info.params.Props[0] // {}) | has("softVolumes")),
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

set_native_node_param() {
  local node_id="$1" param_id="$2" param_json="$3"

  # pw-cli echoes parsed parameter objects to stderr. Hide that structured dump
  # while retaining asynchronous errors for the caller to report when readback
  # does not prove that the requested configuration took effect.
  pw-cli set-param "$node_id" "$param_id" "$param_json" 2>&1 | awk '
    /^Object: size [0-9]+, type Spa:Pod:Object:Param:/ { dumping_param = 1; next }
    dumping_param && /^[[:space:]]/ { next }
    { dumping_param = 0; print > "/dev/stderr" }
  '
}

native_dsp_node_json_has_ports() {
  local node_json="$1"

  jq -e '
    (.channels == 128)
    and (.input_ports == 128)
    and (.port_config.direction == "Input")
    and (.port_config.mode == "dsp")
    and (.port_config.format.mediaType == "audio")
    and (.port_config.format.mediaSubtype == "raw")
    and (.port_config.format.format == "F32P")
    and (.port_config.format.channels == 128)
  ' <<< "$node_json" >/dev/null
}

native_dsp_ports_are_configured() {
  local node_json

  node_json="$(native_node_json)" || return 1
  native_dsp_node_json_has_ports "$node_json"
}

wait_for_native_dsp_ports() {
  local attempts="${1:-20}"

  for _ in $(seq 1 "$attempts"); do
    if native_dsp_ports_are_configured; then
      return 0
    fi
    sleep 0.2
  done

  echo "848 native DSP ports were not created" >&2
  return 1
}

configure_native_dsp_ports() {
  local node_json node_id channel_count
  local diagnostics=""
  local set_status=0

  if native_dsp_ports_are_configured; then
    return 0
  fi

  node_json="$(native_node_json)" || return 1
  node_id="$(jq -r '.id' <<< "$node_json")"
  channel_count="$(jq -r '.channels' <<< "$node_json")"
  if ! [[ "$channel_count" =~ ^[1-9][0-9]*$ ]] || [ "$channel_count" -ne 128 ]; then
    echo "Expected a 128-channel native 848 PipeWire node, got: $channel_count" >&2
    return 1
  fi
  if ! native_node_matches "$node_id" "$channel_count"; then
    echo "848 PipeWire node changed before DSP port configuration" >&2
    return 1
  fi

  # WirePlumber may replace the adapter node while applying PortConfig. In that
  # case pw-cli reports an error for the retired object even though the new node
  # has the requested ports, so the bounded readback is authoritative.
  diagnostics="$(set_native_node_param "$node_id" PortConfig \
    '{ direction = "Input", mode = "dsp", monitor = true, control = false }' 2>&1)" \
    || set_status="$?"
  if wait_for_native_dsp_ports; then
    return 0
  fi

  if [ -n "$diagnostics" ]; then
    echo "pw-cli PortConfig diagnostics:" >&2
    printf '%s\n' "$diagnostics" >&2
  fi
  if [ "$set_status" -ne 0 ]; then
    echo "pw-cli PortConfig command exited with status $set_status" >&2
  fi
  return 1
}

native_dsp_volume_bypass_is_ready() {
  local node_json

  node_json="$(native_node_json)" || return 1
  native_dsp_node_json_has_ports "$node_json" || return 1
  jq -e '
    (.props_present == true)
    and ((.soft_mute_present | not) or (.soft_mute == false))
    and (
      (.soft_volumes_present | not)
      or (
        (.soft_volumes | type == "array")
        and (.soft_volumes | length == 128)
        and (.soft_volumes | all(.[]; . == 1))
      )
    )
  ' <<< "$node_json" >/dev/null
}

native_loopback_links_are_active() {
  pw-dump | jq -e \
    --arg motu "$MOTU_SINK" \
    --arg loopback "$LOOPBACK_NODE" '
    [ .[] | select(
      .type == "PipeWire:Interface:Node"
      and .info.props["node.name"] == $motu
    ) ] as $motu_nodes
    | [ .[] | select(
      .type == "PipeWire:Interface:Node"
      and .info.props["node.name"] == $loopback
    ) ] as $loopback_nodes
    | if ($motu_nodes | length) == 1 and ($loopback_nodes | length) == 1 then
        ($motu_nodes[0].id) as $motu_id
        | ($loopback_nodes[0].id) as $loopback_id
        | [ .[] | select(
          .type == "PipeWire:Interface:Port"
          and .info.props["node.id"] == $motu_id
          and .info.props["port.direction"] == "in"
          and .info.props["port.id"] == 0
          and .info.props["port.name"] == "playback_1"
        ) ] as $motu_left
        | [ .[] | select(
          .type == "PipeWire:Interface:Port"
          and .info.props["node.id"] == $motu_id
          and .info.props["port.direction"] == "in"
          and .info.props["port.id"] == 1
          and .info.props["port.name"] == "playback_2"
        ) ] as $motu_right
        | [ .[] | select(
          .type == "PipeWire:Interface:Port"
          and .info.props["node.id"] == $loopback_id
          and .info.props["port.direction"] == "out"
          and .info.props["port.id"] == 0
          and .info.props["port.name"] == "output_FL"
          and .info.props["audio.channel"] == "FL"
        ) ] as $loopback_left
        | [ .[] | select(
          .type == "PipeWire:Interface:Port"
          and .info.props["node.id"] == $loopback_id
          and .info.props["port.direction"] == "out"
          and .info.props["port.id"] == 1
          and .info.props["port.name"] == "output_FR"
          and .info.props["audio.channel"] == "FR"
        ) ] as $loopback_right
        | [ .[] | select(
          .type == "PipeWire:Interface:Link"
          and .info.props["link.output.node"] == $loopback_id
        ) ] as $links
        | ($motu_left | length) == 1
          and ($motu_right | length) == 1
          and ($loopback_left | length) == 1
          and ($loopback_right | length) == 1
          and ($links | length) == 2
          and any($links[];
            .info.state == "active"
            and .info.props["link.input.node"] == $motu_id
            and .info.props["link.input.port"] == $motu_left[0].id
            and .info.props["link.output.port"] == $loopback_left[0].id
          )
          and any($links[];
            .info.state == "active"
            and .info.props["link.input.node"] == $motu_id
            and .info.props["link.input.port"] == $motu_right[0].id
            and .info.props["link.output.port"] == $loopback_right[0].id
          )
      else
        false
      end
  ' >/dev/null
}

wait_for_native_loopback_links() {
  local attempts="${1:-20}"

  for _ in $(seq 1 "$attempts"); do
    if native_loopback_links_are_active; then
      return 0
    fi
    sleep 0.2
  done

  echo "VirtualSink.output did not establish two active DSP links to the 848" >&2
  return 1
}
