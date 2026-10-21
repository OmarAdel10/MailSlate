# Follow-ups

Out-of-scope findings parked during feature work. One line each, with the file
location. Every entry is also tracked as a Plane work item in the MAILSLATE
project (global `AGENTS.md` §4 and §13).

## Open

- MAILSLATE-43 — M2: the account settings dialog offers 30d/90d/1y sync
  windows but not "all" (PRD F2.2) — the `update_account` wire carries a
  plain i64 where NULL means "unchanged", so the DB's NULL-everything state
  is not expressible; extend protocol + daemon + bridge, then add the choice
  (`lib/features/accounts/account_settings_dialog.dart`).
- MAILSLATE-8 — Docs: FAST-gate cargo commands in `specs/DEVELOPMENT_ENVIRONMENT.md`
  (§12.1) and root `AGENTS.md` (§3/§4) are written as bare `cargo ...` but assume
  the working directory `rust/`; make the cwd explicit. Found during `chore/ci-setup`.
- MAILSLATE-9 — M1: `.github/workflows/ci.yml` build-matrix is Linux-only and the
  windows/macos legs of `.github/workflows/cd.yml` auto-skip until the Windows/macOS
  Flutter scaffolds are committed (`flutter create` on each OS); restore the legs then.
- MAILSLATE-10 — M1: reopened after `01aeacd` rescoped the codegen-staleness check
  (`7a2e7be` failed on any dirty path due to untracked `macos/Flutter/`); keep open
  until a green PR run proves the scoped check still catches real FRB drift
  (`.github/workflows/ci.yml`).
- MAILSLATE-11 — M2: the CI rust job smoke-fetches the GreenMail JAR
  (`.github/workflows/ci.yml`) but cannot run `--features greenmail` integration
  tests until setup-java is wired (`specs/DEVELOPMENT_ENVIRONMENT.md` §8).
- MAILSLATE-12 — M2: fastforge packaging (`linux/packaging/`, `windows/packaging/`,
  `macos/packaging/`) ships the Flutter GUI only; bundle the `mailslate-daemon`
  binary with the installers once it is real.
- MAILSLATE-13 — M5: the `.github/workflows/cd.yml` shorebird job stays skipped
  until `shorebird init` lands `shorebird.yaml` and the `SHOREBIRD_TOKEN` secret is
  set; verify the full release end-to-end at the first `vX.Y.Z` tag.
- MAILSLATE-14 — Docs: `specs/DEVELOPMENT_ENVIRONMENT.md` §10 does not yet match the
  shipped `cd.yml` (appdmg 0.6.6, deb `installed_size`/`license` fields, Inno Setup
  6.7.1 pin, actionlint v1.7.12 pin); sync the spec text.
- MAILSLATE-15 — M1: the shell renders the platform font stack; DESIGN.md §4
  specifies Inter (UI) and JetBrains Mono (code) — bundle both and set the
  TextTheme families (`lib/core/theme/theme.dart`).
- MAILSLATE-16 — M1: minimum window size 1024x640 (USER_FLOW.md §2) is not yet
  enforced; wire the desktop runners once the scaffolds land (MAILSLATE-9;
  `lib/main.dart`, `linux/`, `windows/`, `macos/`).
- MAILSLATE-18 — M2: `is_compatible` in `rust/crates/protocol/` is never called;
  wire the protocol compatibility check and the update-required screen when the
  daemon earns its first wire change (ARCHITECTURE.md §5.2; `lib/features/shell/`).
- MAILSLATE-20 — M2: `daemon_binary()` picks the first `is_file()` candidate
  without an executable-bit check and falls back to a bare PATH lookup
  (`rust/crates/core/src/infra/ipc.rs:145-166`); require `mode & 0o111` on
  unix candidates, keep PATH last.
- MAILSLATE-21 — M2: env-mutating tests set `MAILSLATE_DAEMON_PATH`,
  `MAILSLATE_DATA_DIR`, `XDG_RUNTIME_DIR` while sibling tests run in the same
  binary (`rust/crates/core/src/infra/ipc.rs:368-433`,
  `rust/crates/bridge/src/api/system.rs:208-277`); serialize them or document
  the single-suite constraint before the suites grow.
- MAILSLATE-22 — M2: the daemon accept loop has no per-connection read
  timeout or connection cap (`rust/crates/daemon/src/server.rs:169-193`); an
  idle same-uid peer can hold a task and force a 64 MiB declared-length
  allocation — harden before M2 widens the surface.
- MAILSLATE-23 — M2: the IPC `Client` is desynchronized after a request
  timeout — a late reply would be consumed by the next call and rejected by
  the id check (`rust/crates/core/src/infra/ipc.rs:70-98`); document the
  discard-after-error contract or poison the client.
- MAILSLATE-24 — M2: the single-instance guard is probe-then-bind (TOCTOU)
  (`rust/crates/daemon/src/server.rs:104-116`); concurrent starts race and
  the loser sees a raw io bind error instead of `AlreadyRunning` — use a
  pid-file lock or re-probe on bind failure.
- MAILSLATE-27 — M2+: the accept loop releases its connection slot with
  bare `active.fetch_sub(...)` after the handler task — a panic inside
  the task leaks the slot (`rust/crates/daemon/src/server.rs:244-260`);
  replace with an RAII drop guard.
- MAILSLATE-29 — M2+: `newestDaemonLog` walks with `listSync()` and
  `statSync()`, both of which follow symlinks — a planted symlink can
  win the newest-contest (`lib/core/logging/daemon_log.dart:15-28`);
  use `followLinks: false` plus a FileSystemEntity type check.
- MAILSLATE-30 — M2+: the paused-card `View log` action discards
  `open()`'s result and is a silent no-op on non-Linux or missing
  xdg-open (`lib/features/shell/shell_screen.dart:374`); surface the
  failure with a SnackBar.
- MAILSLATE-31 — M2+: the paused banner resolves the newest daemon log
  once per mount and never re-resolves when the bloc re-emits
  `DaemonUnavailable` (`lib/features/shell/shell_screen.dart:337-362`).
- MAILSLATE-32 — M2: `integration_test/daemon_e2e_test.dart:21-52` has
  no tearDown shutdown; local runs orphan the daemon and the scratch
  dir — kill via pid file in teardown and note the constraint in the
  file header.
- MAILSLATE-34 — CI: re-verify the e2e job apt list (gtk/ninja/
  pkg-config) and the Node-20 deprecation warnings after the Ubuntu-26
  runner flip (`.github/workflows/ci.yml`); starts 2026-10-19.
- MAILSLATE-36 — M3 (user-committed 2026-10-19): incremental sync via
  CONDSTORE CHANGEDSINCE + UIDVALIDITY wipe; SyncNow full-refetch is
  the M2 fallback (`rust/crates/core/src/domain/sync/initial.rs`;
  quirks detection ready in `rust/crates/core/src/infra/imap/quirks.rs`).
- MAILSLATE-37 — M3 (user-committed 2026-10-19): IDLE push with 29-min
  re-issue + 60s polling fallback; pull forward mid-M3 if dogfooding
  demands it (`rust/crates/core/src/domain/sync/actor.rs`).
- MAILSLATE-38 — M3 (user-committed 2026-10-19): sync-window filtering
  + body-fetch-on-open (body_state headers→full) — absorbed by the M3
  reading UI by necessity.
- MAILSLATE-39 — M3 (user-committed 2026-10-19): outbox drainer for
  server mutations (uid_store/uid_move, FIFO per target, retry jitter
  ≤60s) — absorbed by the M3 reading UI by necessity.

## Acknowledged test deferrals

Trivial error-variant branches deliberately left untested in M1 (verified by
the round-3 QA certification; no Plane item — revisit only if these branches
gain logic):

- `LoggingError::Io` arm — `rust/crates/daemon/src/logging.rs:16-17`.
- `MigrationError::Version` arm —
  `rust/crates/core/src/infra/db/migrations.rs:27-28`.
- `FrameError` Display strings — `rust/crates/protocol/src/framing.rs:31-42`.
