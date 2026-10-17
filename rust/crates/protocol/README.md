# mailslate-protocol

IPC types and framing shared by the daemon and the GUI bridge
(ARCHITECTURE.md §5).

## Structure

- `src/message.rs` — request/response/event envelopes and `PingResult`.
- `src/error.rs` — `BridgeError` / `ErrorKind` taxonomy.
- `src/framing.rs` — length-prefixed JSON codec (u32 LE + JSON, 64 MiB cap).
- `src/methods.rs` — method-name constants for the implemented v1 surface.
- `src/async_io.rs` — async frame codec (`tokio` feature); same wire format
  and caps as `framing.rs`, used by the daemon and the IPC client.
- `src/lib.rs` — `PROTOCOL_VERSION`, `is_compatible`, re-exports.

## Key files

- `src/lib.rs` — `PROTOCOL_VERSION`, `is_compatible`.
- `src/framing.rs` — `write_json_frame`, `read_frame`, `read_json_frame`.

## Consumes / produces

- Consumes: nothing (leaf crate; serde + serde_json only).
- Produces: typed contract consumed by both `mailslate-daemon` and
  `mailslate_bridge`.

## Tests

- `cargo test -p mailslate-protocol` — envelope roundtrips, error taxonomy,
  framing caps and malformed-input behavior.
