# mailslate_bridge

The `flutter_rust_bridge` v2 surface the Flutter GUI links against
(ARCHITECTURE.md §1, §6).

## Structure

- `src/lib.rs` — crate root; declares the `api` module.
- `src/api/` — the exported surface: `mod.rs` and `system.rs` (`init_app`
  carries the `#[frb(init)]` hook; `ping`; `daemon_status` never fails —
  it reports `Unavailable`; `start_daemon` is connect-or-spawn plus ping)
  with the `DaemonError`, `DaemonState`, `DaemonStatus`, and `PingInfo`
  types.
- `src/frb_generated.rs` — codegen output, committed in the same change as
  the Rust source it was generated from (AGENTS.md §5).

## Key files

- `src/api/system.rs` — the whole Dart-facing API.
- Generated Dart lives in `lib/src/rust/` and the plugin in
  `rust/rust_builder/`; both are committed, never hand-edited.

## Consumes / produces

- Consumes: `mailslate-protocol` wire types and `mailslate-core`'s IPC
  client; the daemon IPC socket at runtime.
- Produces: a `cdylib` consumed by the `rust_builder` Flutter plugin and
  ultimately by Dart as `lib/src/rust/`.

## Versions

FRB is locked in step (AGENTS.md §5): the `flutter_rust_bridge_codegen` CLI,
the `flutter_rust_bridge` crate, and the Dart dependency must all read
2.13.0, bumped together in one commit.

## Tests

- `cargo test -p mailslate_bridge` — unit tests.
- CI regenerates the bindings and fails when the committed output drifts
  (codegen staleness job in `.github/workflows/ci.yml`).
