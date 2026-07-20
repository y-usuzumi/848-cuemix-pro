# cuemix-848

Linux-first control/probing app for the MOTU 848 / CueMix Pro generation.

The first goal is not to pretend the 848 protocol is fully known. It gives us a
native Linux tool that can:

- probe likely CueMix Pro and older MOTU AVB HTTP endpoints
- read arbitrary device paths
- send conservative `set` updates to arbitrary paths
- run a local browser control/probe UI

This is intentionally dependency-free Rust so it can build on a plain Linux box.

It discovers AVDECC devices through mDNS and speaks the HTTP datastore
compatibility layer present on current 848 firmware. Datastore writes use the
firmware-required raw JSON form body: `json={"value":...}`.

## Build

```sh
cargo build --release
```

## Try it against an 848

Replace `192.168.1.50` with the 848's IP address.

Discover any 848 advertising the standard AVDECC mDNS service:

```sh
cargo run -- discover
```

Discovery uses a native IPv4 mDNS query and also merges results from the
standard Linux `avahi-browse` utility when it is installed. Avahi is currently
needed to discover IPv6-only advertisements.

Validate an advertised AVDECC Proxy endpoint without sending an AVDECC or
device-control command:

```sh
cargo run -- avdecc-probe 192.168.1.50
```

This only opens the proxy's HTTP `CONNECT` tunnel, listens briefly for data,
and prints a bounded JSON summary. It decodes complete version-0 proxy frames
when present, but preserves all received bytes as a hex preview. The current
848 advertises DNS-SD `Version=1` and answers a v0 envelope with a nonzero
reserved field.

For a standards-defined v0 compatibility check, request an ephemeral proxy
controller identity using the host interface's MAC address:

```sh
cargo run -- avdecc-probe 192.168.1.50 --request-entity-id eth2
```

This sends only the v0 `ENTITY_ID_REQUEST` APPDU; it does not control the audio
interface. A reply with `entity_id_reserved: 0` is a standard identity. The
848's nonzero result is printed as `entity_id_candidate`, not as a trusted
controller identity.

After a candidate has been observed, a narrowly scoped descriptor check can
request static metadata for the advertised target entity:

```sh
cargo run -- avdecc-probe 192.168.1.50 --request-entity-id eth2 \\
  --read-entity-descriptor 0001f2fffefeb9e2
```

This sends one standards-defined AEM `READ_DESCRIPTOR` command for entity
descriptor zero. It validates the target, candidate controller ID, and sequence
in the reply, and never sends a gain, phantom-power, routing, monitor, or
notification-registration command. It is still a request to the device, so use
it only while the 848 is available for diagnostics.

The 848 currently reports one active Configuration (`0`). Its static
descriptor-count table can be read independently:

```sh
cargo run -- avdecc-probe 192.168.1.50 --request-entity-id eth2 \\
  --read-configuration-descriptor 0001f2fffefeb9e2
```

For a descriptor type and index confirmed by that table, the generic diagnostic
form is available. For example, the 848 reports Audio Unit type `0x0002`, index
`0`, and Control type `0x001a`, index `0`:

```sh
cargo run -- avdecc-probe 192.168.1.50 --request-entity-id eth2 \\
  --read-descriptor 0001f2fffefeb9e2 0x0002 0
```

Each invocation sends one `READ_DESCRIPTOR` request and prints the complete
bounded descriptor payload. It validates the target, candidate controller ID,
sequence, command, type, and index before exposing a response. It remains
diagnostic-only and never sends a control command.

On the tested 848, the A/B/C monitor labels resolve to standalone Audio Clusters
`23` (`ABC Monitor L`) and `24` (`ABC Monitor R`). The only advertised standard
Control is an unrelated `IDENTIFY` control. Do not infer an A/B/C switching
command from those labels; its vendor-specific mapping remains unimplemented.

An 848 directly attached to this Linux machine may advertise only IPv6. In that
case, use bracket notation, for example `"[2604:4080:1503:8036::1]"`.

```sh
cargo run -- probe 192.168.1.50 --save probe.jsonl
```

Open a local control UI:

```sh
cargo run -- serve 192.168.1.50
```

Then visit:

```text
http://127.0.0.1:8480
```

The browser server intentionally binds only to a numeric loopback address. It
is scoped to the device host passed to `serve`, and each launch issues its own
session token for write requests. Start another server instance when you need
to control a different device.

This is a browser-origin safeguard, not authentication against hostile local
processes. Anyone with local access to the machine may also be able to reach
the device's HTTP control service directly.

The UI includes live Mic 1-4 controls for preamp name, gain, 48 V, pad, and
polarity, plus analog output gain controls for outputs 1-12. It also keeps raw
read, write, and probe controls for the remaining datastore surface.

Read a known or suspected path:

```sh
cargo run -- get 192.168.1.50 /apiversion
```

Capture a complete device subtree for inspection:

```sh
cargo run -- get 192.168.1.50 /datastore --save 848-datastore.json
```

For an older MOTU AVB datastore device, set one datastore value. Unquoted
numbers, booleans, and `null` are sent as JSON literals; other values are sent
as JSON strings. For `/datastore/...` paths, cuemix-848 writes to the datastore
root with the full key, which is required by current 848 firmware.

```sh
cargo run -- set 192.168.1.50 /datastore/ext/obank/2/ch/0/name "Main out"
```

Use `--method PATCH` only when a compatible device requires it:

```sh
cargo run -- set 192.168.1.50 /datastore/ext/obank/2/ch/0/name "Main out" --method PATCH
```

## Notes

Start with `probe` if a device is reachable but does not appear in `discover`.
The output is JSON Lines so we can collect evidence from the hardware and then
promote working paths into first-class controls.

Use `--timeout-ms` on any command if your device or network is slow.
