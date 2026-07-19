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
