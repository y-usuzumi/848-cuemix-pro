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

Discovery sends the native IPv4 mDNS query from every active,
multicast-capable local IPv4 address on Linux and Windows. This keeps a
multihomed Windows host from sending the query only through its default adapter
instead of the MOTU network. On Linux it also sends scoped IPv6 link-local
multicast on every eligible interface; Windows IPv6 interface enumeration is
still pending. No Avahi installation is required. IPv6 link-local answers retain
their interface scope in discovery output, for example `fe80::1%eth2`; pass
that form in brackets to a command, such as `[fe80::1%eth2]` or
`[fe80::1%eth2]:17221`. A Windows hardware run confirms that this
per-interface IPv4 query discovers the tested 848 at `192.168.4.166` and
returns its full AVDECC DNS-SD TXT record set.

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

Open the local discovery landing page:

```sh
cargo run -- serve
```

It performs a read-only mDNS scan at startup and lists each discovered 848.
Opening a device preserves an allow-list of the advertised, usable control
addresses for that server session. Restart `serve` to rescan.

To skip discovery and start a server scoped to one known device instead:

```sh
cargo run -- serve 192.168.1.50
```

Then visit:

```text
http://127.0.0.1:8480
```

The browser server intentionally binds only to a numeric loopback address, and
each launch issues its own session token for write requests. Fixed-host mode
accepts only its configured host. Discovery mode accepts only the advertised
control addresses captured at startup; it does not become an arbitrary-host
browser proxy.

This is a browser-origin safeguard, not authentication against hostile local
processes. Anyone with local access to the machine may also be able to reach
the device's HTTP control service directly.

The UI is divided into **Inputs**, **Outputs**, **Mixer**, and **Diagnostics**
tabs. Inputs contains live Mic 1-4 controls for preamp name, gain, 48 V, pad,
and polarity, plus gain and polarity controls for Line Inputs 5-12. Outputs
contains line-output gain controls plus the headphone outputs advertised by
the connected device. Inputs and Outputs show live per-channel signal meters;
Phones meters retain separate L/R lanes. Mixer contains the capture-validated
faders and full read-only meter diagnostics, while Diagnostics keeps the raw
read, write, and probe controls for the remaining datastore surface. The
selected tab is retained in the URL fragment.

Input and line-output strip headings are not fixed display strings. The UI
reads `ch/<index>/name` from Mic input bank `0`, Analog input bank `1`, and
Analog output bank `0`, and uses `Mic / Inst N`, `Line In N`, or `Line Out N`
only when the corresponding device label is blank. Renamed labels therefore
appear on the control strips and mapped meters. Mic, Line In, and Line Out
labels share the same inline editor: click the displayed name, type the
replacement, then press Enter or click away to save; Escape cancels. Saves use
the corresponding bank's `/ch/<index>/name` datastore path. Channel strips
follow CueMix's vertical layout, with a gain fader beside the level meter. Mic
48 V, Pad, and Polarity and the Line Input Polarity control are latching on/off
buttons.

### Line inputs

The tested 848's HTTP compatibility datastore advertises an eight-channel
`Analog` input bank at `/datastore/ext/ibank/1`, with a `0:20` dB gain range.
Its live gains `[20,20,0,0,0,0,0,0]` exactly match the bounded vendor-state
snapshot's eight one-byte property-`0x13b2` records at indices `0` through `7`.
The HTTP bank does not expose a line-input phase field. The adjacent vendor
property `0x13b3` supplies eight boolean values at the same indices, and the
installed CueMix Pro binary models `kLineInGain` and `kLineInPhase` as distinct
controls. These indices are shown as the 848's physical Line Inputs 5-12.

The Inputs tab discovers the sorted gain and polarity record sets and requires
their indices to match before exposing either control. Gain is limited to
integer `0` through `20` dB and polarity to boolean values. Like Mic gain, Line
Input gain updates its displayed value immediately, debounces writes while the
slider moves, writes `/datastore/ext/ibank/1/ch/<index>/trim`, and uses the
750-millisecond HTTP refresh to recover changes from other controllers. The
vendor snapshot is retained only for the polarity state that HTTP omits.

One explicit polarity change writes only a freshly discovered index through
protocol `00:01:f2:00:00:03`, using
`13:b3:<u16 index>:01:<0|1>`. Rendering, refreshing, polling, and automated
verification never write polarity. This exact write and concurrent
external-controller behavior remain to be confirmed with a controlled user
action; close CueMix Pro before changing polarity.

### Line output trims

The compatibility HTTP output bank advertises 12 analog channels and a
`-99:0` trim range, but its trim values do not track changes made in CueMix Pro.
The bounded vendor-state snapshot instead exposes physical line-output trim as
one-byte property `0x1388`. A controlled read-only comparison matched Line Out 1
at -35 dB to index `0` value `35` and Line Out 4 at -42 dB to index `3` value
`42`; Line Outs 1-2 were members of the monitor group during that comparison.

The Outputs tab discovers every `0x1388` record rather than assuming the 848's
12-output inventory. It displays attenuation `100` as negative infinity and
otherwise converts the byte to its negative integer dB value. One explicit
slider change forms a protocol-`...:03` record
`13:88:<u16 index>:01:<attenuation>` for only the freshly discovered output.
The exact write is not exercised by automated verification. No line-output
write is sent while rendering, refreshing, polling, or testing.

### Headphone outputs

Headphone volume is not present in the HTTP compatibility datastore. A bounded,
read-only initial vendor-state snapshot on the tested 848 instead exposes
property `0x13b7` as four one-byte values at indices `0` through `3`. With the
848 front panel reporting Phones 1 at negative infinity and Phones 2 at -50 dB,
those values are `[100, 100, 50, 50]`: attenuation `100` is the negative-infinity
sentinel. The 848's static AEM strings independently name four headphone
channels: `Headphones 1 L/R` and `Headphones 2 L/R`. Property `0x1388` is not
used for this control: its 12 indexed values match the 848's line outputs, while
the superficially similar four-value property `0x139d` remains zero at the known
headphone settings. The installed CueMix Pro binary also models
`kHeadphoneTrim` separately from `kLineOutTrim` and applies the same output-trim
conversion to each.

The UI derives its headphone list from the `0x13b7` records rather than a model
name. It requires an even number of consecutive channel indices and groups each
pair into one linked-stereo **Phones** control. This produces two controls on the
848 and is intended to produce two on the 10pre and one on the 16A, matching the
published hardware inventories in their MOTU user guides. Unsupported devices
show no phone controls instead of receiving guessed indices.

A user slider change sends both members of only the freshly discovered stereo
pair through vendor property protocol `00:01:f2:00:00:03`. Each record is
`13:b7:<u16 index>:01:<attenuation>`, where the one-byte attenuation is the
positive magnitude of a `-99` through `0` dB value or `100` for negative
infinity. The UI serializes requests against the local meter session and never
sends a headphone write automatically. While Outputs is visible, a five-second
read-only snapshot poll recovers front-panel or other-controller changes until
the vendor notification lifecycle is implemented. Close CueMix Pro before
changing an output gain: concurrent external-controller behavior has not been
mapped. Line and headphone reads share the same bounded snapshot. The exact
`0x13b7` write is not exercised by automated verification; its record envelope
and attenuation conversion are derived from the installed CueMix model and
generation-compatible output-trim traffic.

### Mixer faders

The **Validated Mixer Faders** section is an opt-in AVDECC vendor-control path,
not a compatibility-datastore write. Close CueMix Pro before using it: concurrent
controller behavior is not mapped. Each click opens the capture-validated proxy
session, drains the bounded initial vendor-state exchange, sends one fader
command, and requires its matching acknowledgement. The browser never sends a
fader command automatically. It serializes fader clicks and briefly waits after
an acknowledgement before allowing the next session. If the 848 rejects a
session anyway, the server retries once using a fresh session and the same
idempotent requested fader value.

Only these capture-validated controls are available: `Main 1-2 / Host 11-12`,
`Headphone Mix / Host 11-12`, and `Main 1-2 / Line In 5-6`, at `-12 dB` or
`-60 dB`. The captured encoding is `00:40:4d:e6` for `-12 dB` and
`00:00:41:89` for `-60 dB`; do not infer intermediate values, other strips, or
other buses until they are separately captured.

### Live channel meters

The Inputs, Outputs, and Mixer tabs share one local, read-only AVDECC meter
session. It uses the capture-observed protocol `00:01:f2:00:00:04`, receives
its two meter pages, and exposes both the packed words and decoded channels.
The browser opens `/api/mixer/meters/events` as a Server-Sent Events stream;
each completed device snapshot is pushed immediately and rendering is
coalesced to the next animation frame. `/api/mixer/meters` remains available
as a one-shot snapshot endpoint. Every big-endian 16-bit word contains two
independent one-byte attenuation values: the high byte is the first/left
channel and the low byte is the second/right channel. Values `0x00` through
`0xfe` are positive attenuation magnitudes in 0.5 dB steps, and `0xff` is
silence. For example, `0x6a6f` is −53 dBFS on the first channel and −55.5
dBFS on the second. This supersedes the earlier Q8.8 interpretation of the
whole word and the initial one-byte/one-dB assumption.

The tested 848 maps `0x138c:0` to Mic / Inst channels 1-4 and `0x13ac:0` to
Line Inputs 5-12. The bounded initial state also advertises routing-aware meter
paths rather than requiring fixed output assumptions. Each `0x93ac` line-output
record contains `u16 meter property / u8 record index / u8 channel index`; its
12 indices correspond to the 12 discovered line outputs. Each `0x13b4`
headphone record has the same path format and identifies the first channel of
that Phones output, so the UI renders that channel and the following channel as
separate L/R lanes. Meter paths are optional: an unsupported device shows an
empty meter instead of receiving a guessed mapping.

The primary `0x13ad:0` record has 32 packed stereo words for the 64-channel Mix
In bank. Its sixth word is Host 11-12, its seventh is Mic / Inst 1-2, and its
eleventh is Line In 5-6. The same Mic / Inst word appears at the still-unmapped
`0x13ad:0x20` and `0x13ad:0x40` stages. The remaining raw records stay
available under **All captured meter channels**. The UI uses a −127 to 0 dBFS
visual range and retains decoded and raw values in hover text. Meters rise
vertically beside their channel controls, with adjacent L/R bars for Phones.
The low-level region is compressed so the visible marks at −∞, −48, −36, −24,
−12, −6, −3, and clip remain evenly spaced and readable. A thin peak marker
jumps to every new louder sample and holds that position for one second after
the live level falls. A vendor control or
inventory refresh first closes the local meter session, avoiding concurrent
vendor sessions; the event stream reconnects after that bounded operation and
resumes metering. Empty startup snapshots preserve the last displayed values,
so a bounded restart does not flash every meter to zero. The device request
loop targets a 5-millisecond interval; actual cadence is bounded by the time
needed to receive both device pages.

The **Capture meter baseline** and **Compare to baseline** helper performs no
hardware write. Use it to map the remaining vendor groups: capture while a
known source is quiet, introduce that source alone (or change one fader with a
steady source), then compare. It reports individual channel changes of at
least 0.5 dB, including the vendor property/bank and source name where known.

## Linux Audio Recovery

The 848 USB playback path is intended to remain at full scale, with monitor
level controlled by the 848's physical knob. If a Linux volume control leaves
the `848 Multichannel` PipeWire sink at zero while the 848 USB `Audio Out`
mixer remains at full scale, first inspect the two layers without changing
audio:

```sh
tools/recover-motu-audio.sh --check
```

Once the native DSP path is configured, require its volume lock, PortConfig,
and direct links in the same read-only check with
`tools/recover-motu-audio.sh --check --native-dsp`.

The normal mode restores the USB and PipeWire sink volumes to 100% and routes
the existing `VirtualSink.output` loopback to the 848 at full scale. It leaves
the 848 card profile, default sink, and application stream routing alone. This
is a live audio operation, so stop playback first:

```sh
tools/recover-motu-audio.sh
```

The script is tailored to this workstation's USB card name
`alsa_card.usb-MOTU_848_848AFEB9E2-00` and an already-created `VirtualSink`
with a `VirtualSink.output` loopback stream. Update the variables at the top of
the script if a different 848 or PipeWire graph uses different names.

On this system, WirePlumber currently has `device.restore-routes=true`; that
policy can restore the 848 route to zero whenever the device node is recreated.
To disable route restoration for the current WirePlumber session while running
recovery, use the explicit switch below. It affects every audio device until
WirePlumber restarts, but does not save the setting:

```sh
tools/recover-motu-audio.sh --disable-wireplumber-route-restore
```

### Protecting the Native DSP Path

The 848 exposes 128 USB playback channels but only 16 UAC playback-volume
controls. The KDE `848 Multichannel` slider can silence playback even after the
native DSP links are established: PipeWire keeps those links active, but the
adapter stops producing audible output. The per-node rule below sets PipeWire's
`channelmix.lock-volumes` property and disables WirePlumber property restoration
only for the MOTU playback node. The 848's physical knob remains the monitor
level control, while desktop clients can no longer change this physical sink.
Plasma will continue to display the sink at 0% and keep its slider there because
the native DSP adapter has no Pulse-compatible volume array; that display is not
the 848's audible level.

Installation restarts WirePlumber and briefly interrupts every audio stream.
The adapter property is consumed while the device node is created:

```sh
tools/enable-motu-volume-lock.sh --install
```

To inspect or remove the rule later, use `--check` or `--remove`. The older
`tools/enable-motu-soft-mixer.sh` experiment is retained for diagnostics but is
not part of the native DSP recovery path.

After installing the lock, configure and verify the native DSP path:

```sh
tools/recover-motu-audio.sh --disable-wireplumber-route-restore --native-dsp
```

The 848's PipeWire node advertises 128 native playback channels while its
Pulse-compatible sink exposes only 32. The native DSP path configures the adapter's
session-scoped 128-channel F32P input ports, then verifies that the full-scale,
unmuted `VirtualSink.output` stream has two direct links to the 848. In this
mode the adapter exposes no node-level `softVolumes`, so the stale 0% value in
the 32-channel Pulse view is bypassed instead of written back to the hardware.

`--native-volume` remains available as a compatibility alias for
`--native-dsp`.

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
