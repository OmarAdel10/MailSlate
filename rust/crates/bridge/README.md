# mailslate-bridge

The `flutter_rust_bridge` v2 surface the Flutter GUI links against
(ARCHITECTURE.md §1, §6).

## Structure

- `src/lib.rs` — the API Rust exposes to Dart; generated bindings live in
  `lib/src/rust/` and `rust/rust_builder/` once M1 lands.

## Key files

- `src/lib.rs`

## Consumes / produces

- Consumes: `mailslate-protocol` and `mailslate-core` (M1+); the daemon IPC
  socket (M2+).
- Produces: a `cdylib` consumed by the `rust_builder` Flutter plugin, and
  ultimately by Dart as `lib/src/rust/`.

## Tests

- `cargo test -p mailslate-bridge` — unit tests; the Dart-side smoke test
  lands with M1.
