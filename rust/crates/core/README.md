# mailslate-core

Domain engines and infrastructure adapters for MailSlate; the single owner of
mail state (ARCHITECTURE.md §3).

## Structure

- `src/lib.rs` — crate root; `domain/` and `infra/` modules land with M1/M2.

## Key files

- `src/lib.rs` — public surface of the crate.

## Consumes / produces

- Consumes: `mailslate-protocol` (IPC types, from M2).
- Produces: engines and storage used by `mailslate-daemon`; nothing in this
  crate is exposed to Dart directly.

## Tests

- `cargo test -p mailslate-core` — unit tests colocated in modules.
- GreenMail integration tests (M2+, `--features greenmail`) use the JAR
  fetched by `tool/fetch_greenmail.sh`; no Docker anywhere (AGENTS.md §10).
