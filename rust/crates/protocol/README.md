# mailslate-protocol

IPC types and framing shared by the daemon and the GUI bridge
(ARCHITECTURE.md §5).

## Structure

- `src/lib.rs` — version handshake; the request/response/event envelope
  lands with M2.

## Key files

- `src/lib.rs` — `PROTOCOL_VERSION`, `Hello`.

## Consumes / produces

- Consumes: nothing (leaf crate).
- Produces: typed contract consumed by both `mailslate-daemon` and
  `mailslate-bridge`.

## Tests

- `cargo test -p mailslate-protocol` — compatibility tests for the
  handshake contract.
