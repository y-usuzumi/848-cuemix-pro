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

Discovery sends the native mDNS query over IPv4 and, on Linux, over IPv6
link-local multicast on every up, multicast-capable non-loopback interface.
It does not require Avahi. IPv6 link-local answers retain their interface scope
in discovery output, for example `fe80::1%eth2`; pass that form in brackets to
a command, such as `[fe80::1%eth2]` or `[fe80::1%eth2]:17221`.

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

To observe unsolicited proxy traffic while you make a known CueMix Pro change,
extend this passive-only window (1–30,000 ms; the default is 250 ms):

```sh
cargo run -- avdecc-probe 192.168.1.50 --listen-ms 15000
```

This still sends no AVDECC or device-control command. It only records bytes
that the proxy sends spontaneously during the bounded interval; it does not
establish that notifications are registered or available.
Complete v0 frames captured during this window include `received_ms`, measured
from the start of the listen interval. ADP `ENTITY_AVAILABLE`,
`ENTITY_DEPARTING`, and `ENTITY_DISCOVER` payloads are decoded into protocol,
entity-ID, and available-index fields; other traffic remains a bounded hex
preview.
On the tested 848, changing and restoring a Mic label in CueMix Pro during a
15-second passive window produced only the five-second ADP heartbeat. This
does not establish registered-notification behavior; it only rules out a
passive control update for that test.

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

On Windows, CueMix Pro also exposes a dedicated `MOTU Pro Audio v2 Ethernet`
virtual adapter. A passive capture on that adapter showed its 848 traffic as
IPv6 link-local TCP to port `17221`, carrying v0 `avdecc_from_apc` frames. The
normal controller-to-848 traffic is a fixed vendor-poll command with protocol
ID `00:01:f2:00:00:04`; this alone is not a mapping.

The 848 front-panel LED changes when CueMix changes a Monitor Group, proving
that this is hardware state rather than host-local state. With USB removed,
CueMix Pro re-established the proxy over the physical Realtek adapter. A full
passive Wireshark capture then found two event-specific 47-byte
`avdecc_from_apc` AECP Vendor Unique Commands using protocol ID
`00:01:f2:00:00:03`, with seven vendor-data bytes beginning
`13:94:00:00:02:00`. Each received a matching 40-byte Vendor Unique Response
with the same sequence number. Controlled transitions establish Monitor Group
membership in the final two vendor-data bytes as a big-endian 16-bit bitset:
`0003` for Line Out 1+2, `0005` for 1+3, `0009` for 1+4, `000a` for 2+4,
`010a` after adding Line Out 9, and `090a` after adding Line Out 12. This
validates the documented Line Out 1–12 range and the ordinary `1 << (n - 1)`
bit position. The preceding `02` field's semantics remain unmapped. This
identifies an acknowledged property `0x1394` command path for current-group
membership, but does
not map the remaining A/B/C enable, selection, or routing actions. Keep all
write controls disabled until those actions have equally direct evidence.

Controlled B-to-A, A-to-B, B-to-C, and C-to-A+B+C transitions map active
selection property `0x13b6` to Vendor Unique data `13:b6:00:00:01:<mask>`:
A=`01`, B=`02`, C=`04`, and A+B+C=`07`. The app writes the combined mask
directly. CueMix does not expose two-selection combinations in its UI, though
the 848 front panel can select them; do not infer unobserved `03`, `05`, or
`06` writes from this app-only evidence or expose a control.

With A+B+C set before a passive capture, deselecting C on the 848 front panel
sent no app-originated setter. The subsequent protocol-`00:01:f2:00:00:01`
device-to-app state response carried `13:b6:00:00:01:03`, directly confirming
the front-panel A+B mask and providing the passive state-update path for
front-panel-only controls. Values `05` and `06` remain unobserved.

Disabling A/B/C monitoring sends the same acknowledged `0x13b6` command with
mask `00` and no other event-specific command. CueMix therefore represents
enablement as a nonempty selection mask rather than a separate enable property.

For the front-panel-only MUTE control, passive captures with A+B enabled show
no app setter and no selection (`0x13b6`) change. The protocol-`...:01`
device-to-app state response reports property `0x139b` as `01` after mute and
`00` after unmute, identifying the mute-state latch for this configuration.
Property `0x07d7=01` is also emitted on both button presses but does not toggle;
its meaning remains unmapped. With A/B/C disabled, a second MUTE capture
produced the same `0x07d7=01` and `0x139b=01` reports and no output-target
identifier. The target is therefore resolved by device monitor context, not the
observed MUTE state message. The user reports that the current Monitor Group is
silenced in this state; keep that as hardware-use evidence until a controlled
output observation confirms it.

These front-panel reports revise only the vendor-state-push conclusion, not the
standard AVDECC one. CueMix maintains a protocol-`00:01:f2:00:00:01`
request/re-arm chain, and the 848 returns changed property records through that
chain after front-panel actions. This is notification-like vendor state delivery
(possibly a long-poll lifecycle), not a standards-defined registration or a
proven unsolicited event stream. It has not been reproduced for a second
controller; keep HTTP polling as recovery and map the request token, re-arm
sequence, controller lifecycle, and property scope passively before considering
any client implementation.

An idle virtual-adapter capture maps the normal renewal cadence: about every
five seconds CueMix sends a protocol-`...:01` request whose two-byte vendor
payload equals the sequence number of the preceding renewal, and the 848 sends
an empty response with the new sequence. A front-panel event instead arrives as
a property record using the outstanding request's sequence; CueMix then starts
the same acknowledgement/re-arm chain. This is source-backed lifecycle
evidence, not authorization to reproduce it outside CueMix's established
controller session.

The CueMix **DISCOVERY** screen is below that boundary: a capture that began
with an existing control socket and then closed CueMix ended with that socket's
clean FIN exchange at 2.112 seconds. Reopening CueMix and leaving it on the
DISCOVERY screen produced no new TCP 17221 SYN, HTTP `CONNECT`, or vendor
re-arm traffic. Discovery therefore finds the 848 without opening a control
session; the actual device-open lifecycle still needs a separate passive
capture.

That device-open capture now maps the lifecycle without replaying it. After
the TCP handshake, CueMix sends CONNECT at about 10.5 ms and receives HTTP 200,
performs the proxy identity exchange and read-only descriptor discovery, then
starts protocol 00:01:f2:00:00:01 at 248.6 ms. Its first vendor request has no
vendor payload. The 848 replies with an initial state snapshot as 198 non-empty
pages (9,307 self-delimiting records); each observed record is u16 property_id,
u16 index, u8 value_size, and that many value bytes. CueMix chains each
following request to the prior response sequence until the terminal empty
response at about 302 ms, then begins the five-second renewal cadence described
above. This validates the observed snapshot grammar and lifecycle, but is
still not permission to send those vendor messages.

`avdecc-probe` now decodes any passively delivered `...:01` state record as
`vendor_state` JSON. It recognizes A/B/C selection (`0x13b6`), Monitor Group
membership (`0x1394`), MUTE (`0x139b`), and the unknown front-panel event
(`0x07d7`) when their bounded property records are well formed. It does not
send a token, acknowledgement, re-arm, registration, or vendor-control command,
so such records will appear only if the proxy delivers them without that
unimplemented lifecycle.

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
