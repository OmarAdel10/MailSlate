# mailslate-core

Domain engines and infrastructure adapters for MailSlate; the single owner of
mail state (ARCHITECTURE.md §3).

## Structure

- `src/lib.rs` — crate root; `domain/` (engines) lands with M2.
- `src/infra/paths.rs` — per-OS data, socket, and log paths (§16).
- `src/infra/db/` — connection profile, forward-only migration runner, FTS5
  probe; `migrations/0001_init.sql` is schema v1.
- `src/infra/keyring.rs` — `KeyStore` trait, OS keyring implementation, and an
  in-memory fake for tests.
- `src/infra/ipc.rs` — async IPC client: `Client::connect`,
  `Client::connect_or_spawn` (daemon binary lookup: `MAILSLATE_DAEMON_PATH`,
  executable sibling, cargo `rust/target/{debug,release}`, then `PATH`;
  50 ms retry up to 10 s), `request`, and `ping`.

## Key files

- `src/infra/db/migrations/0001_init.sql` — schema v1; never edited in place,
  new versions ship as `000N_*.sql`.

## Consumes / produces

- Consumes: `rusqlite` (bundled SQLite with FTS5), `dirs`, `keyring`,
  `thiserror`, `tokio` (async codecs and the IPC client);
  `mailslate-protocol` for wire types and framing.
- Produces: engines and storage used by `mailslate-daemon`; nothing in this
  crate is exposed to Dart directly.

## Tests

- `cargo test -p mailslate-core` — unit tests colocated in modules; database
  tests run against temp SQLite files, IPC client tests against a stub
  server.
- GreenMail integration tests (M2+, `--features greenmail`) use the JAR
  fetched by `tool/fetch_greenmail.sh`; no Docker anywhere (AGENTS.md §10).
