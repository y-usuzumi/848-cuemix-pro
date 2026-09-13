# Console protocol evidence

Inspected 2026-09-10 using installed CueMix Pro `1.1.12+07610a749` and
848 firmware `2.3.0+3a75d6526adc`. Executable SHA-256:
`85fc754eaa0c24cec88e4ee0cf84b21db30ae1a0b81830d1227bab455fe1f5a1`.
The implementation is independent of that executable; no vendor code or
binary is distributed with the project.

Evidence consists of the previously captured fader exchanges, a bounded
read-only initial vendor snapshot (9,307 records), HTTP route comparisons,
and static inspection of named C++ property/converter types in the installed
client. New routing, pan, mute/solo, master, and pre/post setter combinations
are implemented from this evidence but remain unverified by live writes.
Automated checks must not change audio hardware.

## Transport and write contract

Use the existing captured CueMix proxy identity and bounded `...:01` initial
state lifecycle, not standard AEM notification registration. Each state/write
record is `u16 property, u16 index, u8 size, value`, with big-endian integers.
Composite indices retain the wire property's `0x8000` bit. Setters use protocol
`00:01:f2:00:00:03`. Each request must receive the expected proxy address,
target, controller, sequence, protocol, successful AECP status and valid length.

`GET /api/console?host=...` returns `{records:[[property,index,hexValue],...]}`.
`POST /api/console/changes` requires the existing same-origin session token,
allowed host, and form field `changes`. An edit is
`operation:target:index:previousHex:value`; semicolons separate up to 32 edits.
Freshly synchronized state must match every previous value before any setter is sent.
No-op writes are removed. Unknown controls, missing inventory members, duplicate
controls, malformed values and stale prior values fail before the first write.
The server reports acknowledged count on partial failure and never retries.
The browser follows routing/mixing batches with a fresh snapshot and reconciles
applied routes, leaving unsuccessful drafts for review. All batches use the
persistent session and verify server readback before returning success; the
response includes acknowledged count and the current monitor snapshot.

## State refresh and meter continuity

A read-only check on 2026-09-10 confirmed that an empty `...:01` request on
an already initialized session returns a fresh 198-page inventory plus its
terminal empty response. Meter `...:04` requests can be interleaved in that
chain. The next state request acknowledges the preceding **state** sequence,
not the intervening meter sequence; all requests share a wrapping sequence
counter. Both complete inventories contained the same 9,307 record keys.

The worker serves console, line-input and output refreshes over that connection.
It continues the incremental re-arm chain between meter requests and also
starts an independent full inventory read after 500 ms since the last completed
inventory. Explicit HTTP refreshes and every slider/console pre-write/readback check always
read the inventory afresh. Full reads interleave meters after 20 ms of page
collection, retain bounds, and commit only a complete inventory. Events newer
than a property's inventory page are overlaid before that commit.
The two-byte acknowledgement is the last **state response**
sequence, including terminal empty responses; it is never a meter or setter
sequence. Matching events against the outstanding state sequence are applied
even when they arrive between meter pages or during a write acknowledgement.
Reads retain the 256-page bound and one total deadline covering queue time and
page collection. A failed read reconnects and reloads the full inventory
without closing the SSE feed. All sliders and console batches reuse the live
connection, including routing, mixer/aux/Reverb levels, pan, masters and monitor
controls. Legacy diagnostic presets and Line Input polarity retain their separate
session lifecycle. The server caches the target identity for each worker after
initial resolution, so repeated slider requests need no extra identity HTTP
connection; allowed-host and origin/token checks still precede writes.

Continuous output dragging exposed repeated timeouts in the older trim path,
which stopped the meter worker, opened a separate session, and retried setup
and the setter once. `/api/outputs/line-trim` and
`/api/outputs/headphone-trim` now enqueue onto the same worker as monitor writes.
Each request independently reads the inventory, resolves the advertised output
indices, skips no-ops, sends one `...:03` payload, and independently reads back
all selected bytes before reporting success. A headphone pair stays in one
payload. Queue time, inventory, acknowledgement and readback share one deadline;
expired work sends no setter and failed/uncertain setters are not retried.
Connection recovery only reopens for subsequent reads or new explicit requests.
Synthetic TCP tests cover repeated output updates on one connection, stereo
pairs with sparse indices, meter/ACK continuity, no-ops, expiry, rejection, lost
acknowledgements and readback mismatch. Server route tests require the existing
worker to remain alive and propagate verification failures. Hardware write
latency and concurrent-controller behavior still require controlled validation.

The all-slider regression alternates Mic/Line gains, physical line/headphone
trims, Main/aux/Reverb send levels and pans, stereo bus masters and monitor
level: 28 setters on one accepted TCP connection with a continuous request
sequence and state ACK chain. Each verified update is interleaved with meters.
Input/console tests also cover gain ranges, sparse or missing channels, no-ops,
expiry, rejected setters, lost acknowledgements and mismatched readback without
retry. HTTP route tests run without any device HTTP listener to ensure an
existing worker's identity and write queue are reused.

The earlier 2026-09-10 release verification completed 20 inventory GETs in 42.1
seconds while a single SSE connection delivered 702 meter updates. All reads
succeeded, with no SSE closure, revision reset or reported device error; the
largest observed gap was 62.2 ms (99th percentile 60.3 ms). `/api/get` also
returned device status 200. This verifies read continuity, not setter behavior
or every network failure condition.

On 2026-09-11, twenty read-only incremental re-arms after the initial inventory
returned empty responses in 0.21–0.86 ms. Release verification then delivered
345 SSE events, each containing all four monitor properties, over 20.7 seconds
while completing twenty inventory GETs. There were no reported errors or
monitor revision regressions; maximum event gap was 61.4 ms (p99 60.7 ms).
`/api/get` returned device status 200. This measures idle delivery and read
continuity, not end-to-end latency from moving the physical knob or a live
setter. Synthetic TCP tests cover interleaved monitor events, stale events,
same-session writes/readback, conflicts, rejection, deadlines and no retries;
browser tests cover queued drags, newer external state and device switches.

Subsequent live comparison exposed a missing-event case: both browser pages and
the worker's successful `/api/outputs` response remained at `1393:0=1f` (-31 dB),
while a separate 9,307-record device read returned `2d` (-45 dB), matching CueMix
Pro's visible monitor level. Empty incremental replies therefore do **not**
establish that cached state is current. The earlier idle cadence test did not
verify this. Independent inventory reconciliation now recovers even if every
incremental event is missed. TCP regressions deliberately return empty deltas
after an unannounced hardware change, require background recovery, reject a
write using the stale value, and verify post-write state without an event.
The cause of missing incremental events/controller scope remains unresolved.

After restoring independent reads, both existing browser pages changed from
-31 to -45 dB without a page reload. A separate device inventory and the server
both returned `2d`, agreeing with CueMix Pro. The release check completed 20
inventory GETs and `/api/get` status 200 while delivering 392 monitor/meter
events in 23.5 seconds, with no reported errors and a 66.6 ms maximum meter gap
(p99 60.8 ms). Most inventory GETs took 76–142 ms; the first took 1.14 seconds.
These are read-only checks; write verification remains synthetic.

## Routing

These records are routing properties, also usable as meter paths. Named
`PropertySurrogate<Router::Property,...kPro*BankPatch>` types distinguish
line, DSP, digital, network, host and headphone destinations.

| Destination | Wire property | Index |
| --- | --- | --- |
| Line output | `93ac` | linear channel |
| Mixer input | `93ad` | linear slot |
| Optical output | `93ae` | `(bank << 8) | channel`, 8 channels/bank |
| Network output | `93af` | `(stream << 8) | channel`, 8 channels/stream |
| Computer recording | `93b0` | linear channel |
| Headphones | `93b1` | `(phone << 8) | L/R channel` |
| ABC monitor shared input | `93b9` | 0 = left, 1 = right |

The four-byte source value is `u16 bankProperty, u8 bank, u8 channel`:

| Source | Property | Bank/channel |
| --- | --- | --- |
| Mic/instrument | `138c` | bank 0, channel 0–3 |
| Line input | `13ac` | bank 0, channel 0–7 (physical 5–12) |
| Optical | `13ae` | bank 0/1, channel 0–7 |
| Computer playback | `13b0` | bank 0, channel 0–127 |
| Network | `13af` | stream 0–15, channel 0–7 |
| Mixer direct out | `13ad` | bank 0, slot 0–63 |
| Main / Mix Monitor | `13ad` | bank 1 / 2, channel 0–1 |
| Aux | `13ad` | bank 3, channel 0–25 |
| Reverb | `13ad` | bank 4, channel 0–1 |
| ABC speaker signals | `13b9` | bank 0/1/2 = A/B/C, channel 0/1 = L/R |
| Disconnected | `0000` | bank 0, channel 0 |

All selections require corresponding advertised records. Optical/host name
inventories (`8022/8023`, `802a/802b`) use packed bank/channel indices, while
network names (`8024/802c`) use linear indices. Host route indices and paths
are linear, but optical/network destination indices are packed. These
differences are covered by boundary tests (host 128, optical B8, network 16/8,
headphone 2R). Listener format `1b5b` supplies the network stream's audio channel
count; a zero-channel media-clock stream is excluded. Valid device names are
used with physical-name fallbacks for blank/invalid UTF-8 values.

## Mixer

| Control | Main | Aux | Reverb |
| --- | --- | --- | --- |
| Input level | `841a` | `83f8` | `842e` |
| Input pan | `842b` | `83f9` | `843f` |
| Master level | `0420` | `0403` | `0434` |
| Master mute | `0421` | `0404` | `0435` |
| Pre-fader | — | `0411` | `043c` |

Input level/pan indices are `(mixerSlot << 8) | busChannel`. The UI uses
channel 0 for Main/Reverb, and each aux's leading channel. Property `03e8`
defines input stereo links, `03e9` aux stereo links: a leading `01` groups
the following inventory member. Pairing itself is read-only. The input fader
uses the leading slot, matching the existing stereo fader capture. Input
mute/solo and master controls explicitly update both displayed pair members.

Input **solo is `03fa` and mute is `03fb`**, indexed by slot. Do not infer their
order from adjacency. Static `PendingChange<kiMixSolo>` instantiation at
`0x140578230` embeds `03fa`; `PendingChange<kiMixMute>` at `0x1405787f0` embeds
`03fb`. `koBusMute` at `0x14057b030` embeds `0404`, `koMainBusMute` at
`0x14057ccf0` embeds `0421`, and `koReverbBusMute` at `0x140579ef0` embeds `0435`.
`koPreFader` at `0x1404f35b0` embeds `0411`; `koReverbPreFader` at
`0x1404f2ff0` embeds `043c`. They use one-byte boolean values.

Faders encode linear gain as `floor(10^(dB/20) * 2^24)` and zero as silence.
`IFaderImpl` encoder at `0x1405367a0` and `OFaderImpl` encoder at `0x140553d20`
multiply by the double `16777216.0` at `0x140cde9c8`, then truncate. This
independently reproduces captured −12 dB (`00404de6`) and −60 dB (`00004189`),
and live −6 dB (`00804dce`) / unity (`01000000`) values. UI/API range is
−90 through +12 dB plus `-inf`; non-finite values are rejected.

`IPanImpl` at `0x140536610` uses `floor((pan + 1) * 0.5 * 2^24)` for pan
−1 through +1. `PreFader` converter at `0x14047e080` converts nonzero to true
without inversion. Input mute/solo are shared across buses; pre/post belongs
to the destination bus, not to an individual send.

The older note interpreting fader index `0x10` as a source-family selector is
superseded: the current router records place Line In 5–6 in mixer slots 16–17.
Fader addressing is by mixer slot. Meter records can represent distinct signal
stages, so this does not resolve every meter-stage mapping.

## ABC monitoring

The native Outputs tab and [MOTU's 848 guide](https://cdn-data.motu.com/manuals/pro-audio-v2/848_User_Guide.pdf)
(printed pages 31–33, 42) distinguish one Monitor Group from ABC speaker
selection. ABC uses one shared stereo input and three stereo output signals,
which can be routed to physical outputs; the main monitor knob controls their
shared level. Monitor Group membership applies when ABC is off. The browser
keeps these controls separate and stages all connection changes for review.

Earlier standard AEM reads identify Audio Clusters 23/24 as ABC Monitor L/R,
but the only standard Control is IDENTIFY. These labels are corroboration,
not a standard AEM setter. The implementation uses the captured vendor session.

| Setting | Property, index | Evidence / value |
| --- | --- | --- |
| ABC selection | `13b6`, 0 | Three-bit mask 0–7; captured Off/A/B/C/All = 0/1/2/4/7, pair combinations = 3/5/6 |
| Monitor Group members | `1394`, 0 | Captured big-endian u16 mask, bit n = Line Out n+1 |
| Shared monitor attenuation | `1393`, 0 | `kMonitorTrim`, same `OutputTrimConverter` as line/phones: 0–99 dB attenuation, 100 = silence |
| Monitor mute | `139b`, 0 | Captured front-panel 0/1 transitions; installed-client `kMuteEnable` setter and boolean serializer |
| Monitor mono | `139a`, 0 | Installed-client `kMonoEnable` setter and boolean serializer; 0 = stereo, 1 = mono |
| Talkback enable | `13a3`, 0 | Installed-client `kTalkbackEnable` setter and boolean serializer; 0 = off, 1 = on |
| Shared stereo source routes | `93b9`, 0/1 | `kProMonitorBankPatch`, four-byte source path |

Installed-client `PendingChange<kABCMonitorEnable>` at `0x1404eeae0` embeds
`13b6`; `kMonitorGroup` at `0x1404ef660` embeds `1394`; `kMonitorTrim` at
`0x1404efc30` embeds `1393`. The monitor trim IO type at RTTI file offset
`0xf95660` names the same output-trim converter as line/headphone trims.
The read-only 2026-09-10 snapshot reports `1393:0=1e`, matching the native
Outputs knob's −30 dB, `1394:0=0003`, and ABC off (`13b6:0=00`).

`PendingChange<kProMonitorBankPatch>` at `0x14061a5c0` embeds `13b9`.
Its serializer at `0x1405efb10` writes composite-index property `93b9`,
size 4, and the same big-endian source path used by existing routes. The
source-stream encoder at `0x140641a18` combines `13b9` with bank/channel;
the native Device source inventory enumerates ABC A/B/C, each L/R, while
the snapshot advertises only two shared destination records `93b9:0/1`.
Read-only meter inventory independently supplies `13b9` banks 0, 1 and 2,
each with exactly two channels (all silent while ABC is off/disconnected).
Both were disconnected (`00000000`); inspecting and implementing this
feature did not alter those routes or any other hardware settings.

The `02` in captured `13:94:00:00:02:<mask>` is the u8 value length, not an
additional group selector. Only one membership mask is exposed. Membership
writes require known line-output indices below 12 and reject unmapped bits
in both current and requested masks. All ABC controls and source choices
require the valid selection byte and exactly two advertised stereo input
records. The browser offers pair shortcuts A+B=3, A+C=5, and B+C=6 as one
selection write each. A+B=3 was observed in a front-panel state event; the user
also confirms simultaneous front-panel selection. Masks 5/6 are derived from
the independent bit assignments, not captured native-client setters. The new
pair writes are tested synthetically; live hardware verification remains
read-only. Values outside 0–7 are rejected.
ABC sources cannot be connected to the mixer or their own input through this
API. Mute uses the device's own context-dependent latch, documented below.

`monitor-select:monitor:0:<oldByte>:<mask>`,
`monitor-level:monitor:0:<oldByte>:<dB or -inf>`, and
`monitor-members:monitor:0:<oldU16>:<decimalMask>` use the existing console
batch API, including whole-batch conflict checks and acknowledgement handling.
When every edit is a monitor control, the worker reads a fresh inventory,
checks the entire batch against that state, sends sequential `...:03` setters,
then reads a fresh inventory again. The response includes
`{acknowledged,monitor:{records,revision}}`; monitor records also accompany each
meter SSE event. Revisions increase across worker reconnects and use a wall-clock
millisecond floor to survive normal server restarts without reloading the page.
No setter is
retried on timeout, conflict, rejection or readback failure. Other batches keep
their existing full-snapshot lifecycle.

The browser sends level changes during dragging with a 60 ms coalescing window,
one in-flight request, and one latest pending value per control. Pending edits
retain their original conflict bytes; an acknowledged, verified own write
supplies the expectation for the next queued value. Newer device revisions
take precedence over late responses, while pending values remain visible until
confirmed or rejected. State events update the existing controls without
replacing focused DOM nodes. Host changes cancel queued edits.

`route:monitor:<0 or 1>:<oldPath>:<newPath>` addresses the shared input.
`/api/outputs` includes a console snapshot from its existing state read, so
the panel adds no periodic vendor inventory request. It also includes the
monitor records and revision captured with that state read, allowing the
five-second poll to recover monitor state if browser SSE delivery pauses.

## Front-panel Mute and Talk

Static inspection on 2026-09-11 extends the earlier passive mute-state mapping.
`PendingChange<kMuteEnable>` has RTTI at file offset `0x10f1370` and vtable
`0x140c03430`. Its constructor's containing function embeds `139b` at
`0x140615d40` / `0x140615db0`; the pending-change construction at
`0x140615f59` references that vtable. Serializer `0x1405edbf0` emits
`13:9b:<u16 index>:01:<boolean>` for the non-composite form. This provides
setter evidence beyond observing a state transition; it does not prove that
the attached firmware accepts a browser-originated setter.

`PendingChange<kTalkbackEnable>` has RTTI at `0x107ab70` and vtable
`0x140be8270`. Setter `0x1404f4700` embeds `13a3`, constructs that pending
change at `0x1404f4ad9`, and serializer `0x1405ee0f0` emits
`13:a3:<u16 index>:01:<boolean>`. The separately named Latch setter uses
`13a2`; browser Talk toggles the enable latch without changing that preference.
Static neighboring mappings identify Talkback Level/Dim/Source at
`13a4`/`13a5`/`13a6`. Those setup controls are not exposed by this change.

A fresh 9,307-record device inventory reports `139b:0=00`, `13a3:0=00`,
`13a2:0=01`, `13a4:0=00000000`, `13a5:0=01000000`, `13a6:0=40`.
The native Home tab independently shows Talk off, Latch checked, Source None,
Level −∞ and Dim 0 dB. The [848 guide](https://cdn-data.motu.com/manuals/pro-audio-v2/848_User_Guide.pdf),
printed pages 9, 32 and 34, documents Mute targeting the current monitor context
and Talk using the configured talkback microphone and destinations.

`monitor-mute:monitor:0:<oldByte>:<0 or 1>` and
`monitor-talk:monitor:0:<oldByte>:<0 or 1>` use the existing persistent monitor
write path, fresh conflict checks, validated acknowledgements, and independent
full-inventory readback. Both require advertised one-byte 0/1 records and the
mapped monitoring inventory. They do not write output trims, speaker masks,
microphone configuration, routing, or talkback Latch. The browser provides
explicit click-on/click-off buttons; Talk remains enabled until switched off.
Both latches travel with the meter stream and Outputs recovery snapshots.

Synthetic tests cover exact property bytes, missing/malformed inventory,
whole-batch conflicts, no-op suppression, rapid toggles, failed writes, device
readback, and external state changes. Live checks remain read-only: neither
new setter has been automatically exercised on the 848. The live `/api/get`
check returned 200; submitting both already-off values through the authorized
API returned `acknowledged:0` with fresh off-state readback, confirming no
setters were sent. Both open browser pages were reloaded after the server
restart and display enabled Mute/Talk buttons matching the device. Verify both
directions and the audible targets during a user-controlled listening session.

### Mono

The installed client's `PendingChange<kMonoEnable>` RTTI is at file offset
`0x10f1140`, with vtable `0x140c04e18`. Setter `0x1406155c0` embeds `139a`
at `0x140615780` / `0x1406157f0` and constructs the named pending change
at `0x140615999`. Serializer `0x1405edab0` emits
`13:9a:<u16 index>:01:<boolean>`. This is separate from headphone source mono.
The fresh 2026-09-11 inventory advertises exactly `139a:0=00`.

`monitor-mono:monitor:0:<oldByte>:<0 or 1>` uses the same validated monitor
write path as Mute and Talk, including advertised boolean checks, whole-batch
conflict checks, acknowledgement validation and full-inventory readback. Mono
state joins both the meter stream and Outputs recovery snapshots. No routes,
levels or other latches are written when Mono changes.

MOTU's guide (printed page 32, linked above) states that Mono sums left/right
to both channels of the main output pair, or the ABC pairs when enabled.
Other Monitor Group channels are unaffected. The device applies its own
3 dB attenuation to the summed signal; the browser does not emulate the DSP.
The browser button shows On/Off and restores stereo on the next click.
Synthetic tests cover both values, invalid inventory, conflicts, failed writes,
readback and external changes. Live hardware verification remains read-only
or no-op; the audible Mono behavior still needs user-controlled validation.
The live `/api/get` check returned 200. Submitting the already-off Mono value
returned `acknowledged:0` and `139a:0=00`, sending no setter. Both refreshed
browser pages display Mono off. Simulator browser checks verified on/off and
external-state updates without changing the other front-panel controls.
Revision checks discard
output reads that overlap a write or device change. Browser testing used a
synthetic device for routing apply/readback, selection, membership and volume.

## Remaining validation

The installed-client and read-only evidence justifies the implemented property
mapping; it does not prove every firmware setter or stereo propagation behavior.
A controlled listening session should validate new writes and concurrent
external-controller changes. Polling remains the recovery mechanism. No
standard notification registration, preset commands, DSP-effect
controls, AVB stream connection management, or link editing is added here.

## Input gain sliders

The installed-client `PendingChange<kPreampGain>` RTTI at file offset
`0x10be7c0` leads to the constructor at VA `0x1404f1eb0`, which embeds property
`0x1389`. The serializer at VA `0x1405ecba0` emits `13:89`, the channel index,
size `01`, and one gain byte. The named Mic gain IO model uses
`InputTrimConverter`. `PendingChange<kLineInGain>` at file offset `0x10ecf30`
and its constructor at VA `0x1404f2a30` identify `0x13b2`; its IO model uses the
same converter. These are input gains, distinct from output attenuation.

A read-only comparison on 2026-09-12 through the running persistent session
found Mic `1389` indices 0–3 = `[45,45,0,0]` and Line `13b2` indices 0–7 =
`[20,20,0,0,0,0,0,0]`. Both arrays exactly matched their HTTP input banks;
HTTP advertised Mic `0:74` and Line `0:20` ranges. `/api/console` exposes these
mapped records for read-only inspection.

`POST /api/inputs/gain` accepts the existing token and allowed host, `bank=mic`
or `bank=line`, `input=<advertised channel index>`, and integer `gain_db` within
that bank's range. The worker validates the fresh one-byte inventory, sends
one `...:03` record only when necessary, and verifies the requested byte with
an independent inventory read. It shares the meter connection and one total
deadline with other slider work. No setter is retried automatically. HTTP
polling remains for input recovery and non-gain input controls keep their raw
`json={...}` datastore transport. Automated verification uses simulated peers;
the exact gain setters have not been exercised against live audio hardware.
