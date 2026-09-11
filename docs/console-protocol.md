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
The fresh snapshot must match every previous value before any setter is sent.
No-op writes are removed. Unknown controls, missing inventory members, duplicate
controls, malformed values and stale prior values fail before the first write.
The server reports acknowledged count on partial failure and never retries.
The browser follows every attempted batch with a fresh snapshot and reconciles
applied routes, leaving unsuccessful drafts for review.

## State refresh and meter continuity

A read-only check on 2026-09-10 confirmed that an empty `...:01` request on
an already initialized session returns a fresh 198-page inventory plus its
terminal empty response. Meter `...:04` requests can be interleaved in that
chain. The next state request acknowledges the preceding **state** sequence,
not the intervening meter sequence; all requests share a wrapping sequence
counter. Both complete inventories contained the same 9,307 record keys.

The read-only worker now serves console, line-input and output refreshes over
that connection, polling meters between state pages when 20 milliseconds have
elapsed. Reads retain the 256-page bound and also have one total deadline
covering queue time and page collection. A failed read reconnects the proxy
without closing the SSE feed. Explicit vendor writes still use their existing
fresh-session lifecycle and can briefly interrupt metering.

Live verification of the release server completed 20 inventory GETs in 42.1
seconds while a single SSE connection delivered 702 meter updates. All reads
succeeded, with no SSE closure, revision reset or reported device error; the
largest observed gap was 62.2 ms (99th percentile 60.3 ms). `/api/get` also
returned device status 200. This verifies read continuity, not setter behavior
or every network failure condition.

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

## Remaining validation

The installed-client and read-only evidence justifies the implemented property
mapping; it does not prove every firmware setter or stereo propagation behavior.
A controlled listening session should validate new writes and concurrent
external-controller changes. Polling remains the recovery mechanism. No
standard notification registration, preset commands, A/B/C routing, DSP-effect
controls, AVB stream connection management, or link editing is added here.
