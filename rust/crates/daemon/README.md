# mailslate-daemon

Headless sync daemon — the only writer of the MailSlate database
(ARCHITECTURE.md §1).

## Structure

- `src/main.rs` — binary entry and CLI contract (runtime lands with M2).

## Key files

- `src/main.rs`

## Consumes / produces

- Consumes: `mailslate-core` engines and `mailslate-protocol` for the IPC
  listener (M2).
- Produces: the `mailslate-daemon` binary shipped with every installer; OS
  notifications once M5 lands.

## Tests

- `cargo test -p mailslate-daemon` — CLI contract tests.
