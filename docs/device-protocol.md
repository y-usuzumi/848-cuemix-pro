# Device protocol evidence

Inspected 2026-10-02 using the installed CueMix Pro executable identified in
[console-protocol.md](console-protocol.md). No vendor code or binary is
redistributed. These are independently implemented record mappings derived
from named client property/converter types, static serializers and read-only
848 snapshots. Live setter behavior has not been tested.

## Network controls

The transport is the existing bounded vendor initial-state lifecycle `...:01`
and setter protocol `00:01:f2:00:00:03`. Records are `u16 property, u16 index,
u8 length, value`; index is zero for each device-wide setting.

| Setting | Property | Wire value |
| --- | --- | --- |
| Sample Rate | `000a` | Four-byte big-endian Hz: 44100, 48000, 88200, 96000, 176400, 192000 |
| Clock Source | `000b` | One byte: Internal `00`, AVB input `04`, Word Clock `05`, Optical A `0c`, Optical B `0d` |
| Word Clock | `000c` | One byte: Out `01`, Thru `00` |
| AVB clock input selector | `1b5f` | One byte, zero-based input index; 848 streams 1–16 use 0–15 and Media Clock Input uses 16 |
| Advertised input-stream count | `1b5d` | Two-byte big-endian count; read-only |
| Input stream format inventory | `1b5b` | Six bytes per zero-based input index; read-only |
| IPv4 Address | `0005` | 16-byte text field; stop at first NUL, ignore trailing bytes; read-only in this UI |

Named `kSampleRate`, `kClockSource`, `kWordClockOut` and `kAVBClockStream`
PendingChange types link respectively to properties `000a`, `000b`, `000c`
and `1b5f`. In the installed client, the ClockSource PendingChange path at
`0x1404edf60` embeds `000b`; SampleRate at `0x1404ee520` embeds `000a`; and
AVBClockStream at `0x1404eb150` embeds `1b5f`.

The ClockSource serializer/converter at `0x140473040` maps model enum values
0–5 to wire bytes `00,04,05,08,0c,0d`. The label switch at `0x1402f0700`
identifies those enum values as Internal, AVB, Word Clock, S/PDIF, Optical A and
Optical B. S/PDIF is not offered for the 848. Do not confuse the model enum
ordinal with the wire byte. Setter record encoders at `0x1405eb390` and
`0x1405eb4d0` emit `000b` and `000c` with one-byte values; `0x1405f0030`
emits `1b5f`. The WordClockOut label branch at `0x140327814` selects Out for
true and Thru for false. Sample-rate converter `0x1404730d0` emits Hz as a
32-bit value; the existing vendor serializer supplies network byte order.

A fresh read-only `/api/console` check on the attached 848 returned:

- `000a:0000 = 00017700` (96000 Hz), matching the HTTP datastore.
- `000b:0000 = 00` (Internal).
- `000c:0000 = 00` (Thru).
- `1b5d:0000 = 0011` (17 inputs).
- `1b5f:0000 = 00` (first input selector, inactive while clock is Internal).
- `1b5b:0000` and `1b5b:000f = 000177000800`.
- `1b5b:0010 = 000177000100` (the final clock input).
- `0005:0000` decodes to `192.168.4.166` before its first NUL.

The UI sends clock-stream selection before AVB Clock Source and includes both
expected values, including an unchanged source. The server checks the entire
batch against a fresh snapshot before any setter, removes no-ops, bounds the
selector by the advertised count and format inventory, and verifies all written
bytes afterwards. This is two ordered setters, not an atomic hardware action;
partial/uncertain failure is reported and never retried. Rate, Word Clock and
clock-source setters share the same persistent worker as other tabs. Polling
remains the recovery mechanism. Only the explicit name edit continues to use
the firmware-required raw datastore root-key write.

IPv6 discovery independently of the connected address remains unresolved;
`0006` must not be assumed to be IPv6 simply because it follows IPv4. The UI
currently displays a literal connected IPv6 host when present.

## Windows driver settings

Buffer Size, Output Safety Offset, WDM input/output stereo-pair counts and the
multichannel flags are properties of named `WinDriverDevice` / `WinDriverDeviceImpl`
types, not `MOTU_VENDOR` network properties. The concrete implementation vtable
at `0x140c0a990` leads to local Windows handle/IOCTL code. This establishes why
those fields cannot be enabled through the Linux server's network connection.
It does not yet provide a complete, validated Windows driver backend.

The generic driver property function at `0x14068d3f0` calls Windows
`DeviceIoControl` (`0x140b117e0` import) with IOCTL `0x222017` and a 32-byte
input structure. Recovered x64 structure layout is:

| Offset | Field |
| --- | --- |
| 0 | u32 driver device index |
| 4 | u32 property ID |
| 8 | u32 input payload size |
| 12 | u32 output payload size |
| 16 | u64 input payload pointer |
| 24 | u64 output payload pointer |

The outer IOCTL call has no ordinary output buffer: payload results use the
embedded output pointer. Driver enumeration at `0x14068d7c0` uses IOCTL
`0x22200b`; initial device metadata reads property `0a` into an 80-byte result.
The observed property accessors include:

| Driver property | Access observed |
| --- | --- |
| `03` | Read 40 bytes, accessor ending at `0x140690a21` |
| `04` | Write 40 bytes, accessor ending at `0x140690b08` |
| `0c`, `0d` | Read 8 bytes, accessors ending at `0x140690be1`, `0x140690cc1` |
| `0e`, `0f` | Write 8 bytes, accessors ending at `0x14068e6e6`, `0x14068e8a6` |
| `12`, `13` | Read 8 bytes, accessors ending at `0x140690da1`, `0x140690e81` |
| `14`, `15` | Write 8 bytes, accessors ending at `0x14068ea46`, `0x14068ebe6` |

Exact association of each driver property, payload fields, units, capability
ranges and device-index-to-network-identity matching still needs tracing and a
read-only native Windows driver check. The 40-byte structure must not be guessed
as a single buffer-size integer. None of these IOCTLs were executed in this
investigation. Do not send those property IDs through AVDECC or silently
substitute PipeWire settings for the Windows-driver controls.

## Validation and remaining work

Synthetic Rust checks cover native encodings, advertised stream bounds,
missing controls, stale expected bytes, no-op handling, fixed device indices and
invalid values. Browser tests cover paired clock selection, native rate/Word
Clock writes, readback failures and focused-name polling. Existing persistent
session tests exercise acknowledgement, readback mismatch, lost replies,
partial batches and no retries. Automated hardware checks were GET-only.

Remaining: controlled hardware setter validation (including clock loss and
paired-stream partial failure), independent IPv6 mapping, and a native Windows
backend with precise driver payload/capability and identity checks.
