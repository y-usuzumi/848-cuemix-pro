# cuemix-848 Agent Guide

## Project scope

`cuemix-848` is a dependency-free Rust control and discovery tool for the MOTU
848's HTTP compatibility datastore. The browser UI is intentionally local-first
and is served by the same binary.

## Layout

- `src/cli.rs`: command-line parsing and output.
- `src/avdecc.rs`: bounded AVDECC Proxy tunnel inspection.
- `src/avdecc_format.rs`: AVDECC probe JSON formatting.
- `src/avdecc_transport.rs`: AVDECC proxy address validation and TCP setup.
- `src/device.rs`: HTTP client, response decoding, form-body generation, and
  shared escaping helpers.
- `src/discovery.rs`: mDNS and Avahi AVDECC discovery.
- `src/probe.rs`: conservative read-only endpoint probing.
- `src/server.rs`: local browser proxy and API routes.
- `src/ui.rs` and `src/ui.html`: browser UI template and renderer.

## Hardware guardrails

- Current 848 firmware acknowledges a write only when `/datastore` receives a
  **raw** `json={...}` form body. Do not URL-encode that inner JSON or replace
  root-key writes with individual datastore-path writes without hardware proof.
- Treat preamp gain, phantom power, output trim, and routing changes as live
  hardware actions. Do not use non-no-op writes as automated verification.
- The compatibility datastore exposes analog output trim, but not a reliable
  A/B/C monitor-group mapping. Keep that work behind AVDECC descriptor mapping.
- Polling is the UI recovery mechanism until AVDECC unsolicited notifications
  are implemented. Do not remove it merely because a write endpoint responds.
- A v0 AVDECC Proxy identity reply with a nonzero reserved field is not a
  controller identity. Do not transmit AECP through that tunnel; retain the
  reply only as protocol evidence.
- The loopback UI protects against cross-site browser writes. It is not a
  security boundary against hostile processes running on the same machine,
  which may be able to contact the 848 directly.

## Verification

Run `cargo fmt --check`, `cargo test`, `cargo clippy -- -D warnings`, and
`cargo build --release` after Rust changes. For UI/server changes, also perform
a read-only `/api/get` check against the local server when the attached 848 is
available.

## Documentation

Update `README.md` and `TODO.md` whenever the supported controls, protocol
evidence, or AVDECC work plan changes.
