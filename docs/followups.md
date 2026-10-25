# Follow-ups

Out-of-scope findings parked during feature work. One line each, with the file
location. Every entry is also tracked as a Plane work item in the MAILSLATE
project (global `AGENTS.md` §4 and §13).

## Open

- MAILSLATE-44 — M2: `restart_actor` spawns the replacement while the old
  actor may still be mid-cycle, interleaving `account_status` events and
  duplicating work; gate reconnect respawns on actor termination (await the
  ack or check `current_state()`) (`rust/crates/daemon/src/dispatch.rs:495-517`).
- MAILSLATE-45 — M2: re-sync never deletes vanished `message_locations`
  and a recorded `uidvalidity` change is never acted on, so local membership
  drifts from the server across `sync_now` runs; add expunge/UIDVALIDITY
  handling (ties into MAILSLATE-36)
  (`rust/crates/core/src/infra/db/writer.rs:399-419`).
- MAILSLATE-46 — M2: a failed SELECT (including the 30s timeout) silently
  skips a mailbox with no surfaced event; publish a `sync_progress` error
  phase so the GUI can tell "empty" from "failed"
  (`rust/crates/core/src/domain/sync/initial.rs:127-139`).
- MAILSLATE-47 — M2: `ThreadCountersRecompute` runs after every mailbox
  (O(mailboxes×threads)); recompute once after the mailbox loop
  (`rust/crates/core/src/domain/sync/initial.rs:213`, op at
  `rust/crates/core/src/infra/db/writer.rs:691-704`).
- MAILSLATE-48 — M2: `add_imap_account`/`add_oauth_account` store the
  secret under a freshly reserved id before the row write; a later DB
  failure orphans the keyring entry — sweep unreferenced secrets at
  startup (`rust/crates/core/src/domain/accounts.rs:99,145`).
- MAILSLATE-49 — M2: dispatch handlers lock `ctx.sync`/`ctx.oauth` with
  `.lock().expect` (11 sites); a panic while holding either poisons the
  lock and every later dispatch panics its connection task — map poisoned
  locks to bridge errors (`rust/crates/daemon/src/dispatch.rs`).
- MAILSLATE-50 — M2: the IMAP/SMTP TLS connector hardcodes the webpki root
  store, so the `Tls`/`StartTls` arms cannot be integration-tested against
  a local fake CA; add a trust-store injection seam
  (`rust/crates/core/src/infra/imap/tls.rs:17`).
- MAILSLATE-51 — M2: minor test gaps in sync/writer/mime paths —
  `refresh_oauth` failure sub-branches (`actor.rs:100-124`), reauth park
  ignores SyncNow (`actor.rs:324-329`), event-stream garbage-frame recycle
  (`events.rs:172-177`), `MessageUpsert` soft-fail and gmail fresh-INSERT
  (`writer.rs:491-509,600-602`), probe `read_reply_line` EOF
  (`probe.rs:160-163`), mailbox `flag_string` arms (`mailbox.rs:102-113`),
  mime address-group flattening (`mime.rs:134-140`).
- MAILSLATE-53 — M2: the accept loop enforces MAX_CONNECTIONS=8 with same-uid
  slot accounting but has no per-connection read timeout or idle cutoff, so a
  same-uid peer can exhaust slots (resource exhaustion, CWE-400); harden
  accept-loop slot lifecycle before M2 widens the surface. Pre-existing code,
  not introduced by M2 — track only, do not fix in this branch
  (`rust/crates/daemon/src/server.rs:30`).
- MAILSLATE-52 — M3: subject-only thread clustering merges unrelated
  conversations that share a normalized subject; replace with JWZ threading
  in the M3 slice (`rust/crates/core/src/infra/db/writer.rs:510-522`).
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

- MAILSLATE-55 — M3: search feature shipped test-less (PR #13): no SearchBloc
  unit test, no SearchScreen widget tests, FakeFfiClient has no `search()`
  stub; also `_debounceTimer` is never assigned (all cancels are no-ops, the
  real debounce is the 150ms `Future.delayed` guarded by `emit.isDone`) —
  handle all four in a search-test pass
  (`lib/features/search/search_bloc.dart:132,148,153`,
  `test/support/fake_ffi_client.dart`).
  FIXED 2026-10-25 in `test/search-feature-tests` (46 new tests, fake
  `search()` stub, dead timer removed, mount-time empty queries guarded,
  epoch-stamped stale-emits guards on the query and filter paths) —
  close on merge.
- MAILSLATE-56 — M3: CI rust job red on the `feature/m3-reading-search` tip
  since PR #9/#13: `dispatch::tests::spawn_on_add::search_dispatch_returns_hits_for_account`
  panics at dispatch.rs:2334 (pre-existing; fails every M3 PR run, e.g.
  40055924645 and PR #14's 40097171641) — investigate and fix before M3
  close (`rust/crates/daemon/src/dispatch.rs:2334`).
- MAILSLATE-63 — M3: search retry semantics — `Try again` dispatches
  SearchCleared, so the typed text survives in the controller and the
  clear affordance stays visible while the placeholder lead shows
  (`lib/features/search/search_screen.dart:354-355`, pinned by the
  retry widget test in `test/search_screen_test.dart`) — decide
  whether retry should re-run the query instead of discarding it, and
  align the USER_FLOW §7 states-matrix copy.
- MAILSLATE-68 — M3: search states-matrix copy deviations pinned by tests
  but untracked until now: the loading state shows a spinner with
  `Searching "…"` copy while USER_FLOW §7 mandates no skeleton, and
  every AppFailure renders the error state while the spec says
  `Search unavailable` only for DB errors — align the copy or amend
  the spec (`lib/features/search/search_screen.dart`,
  `specs/USER_FLOW.md:284`).

## Acknowledged test deferrals

Trivial error-variant branches deliberately left untested in M1 (verified by
the round-3 QA certification; no Plane item — revisit only if these branches
gain logic):

- `LoggingError::Io` arm — `rust/crates/daemon/src/logging.rs:16-17`.
- `MigrationError::Version` arm —
  `rust/crates/core/src/infra/db/migrations.rs:27-28`.
- `FrameError` Display strings — `rust/crates/protocol/src/framing.rs:31-42`.

M2 accepted constraints (recorded by the round-3 QA review; no Plane item —
revisit only if these gain decisions or logic):

- The `id_token` email is read without JWT signature validation — acceptable
  per OIDC §3.1.3.7 because the token arrives directly over TLS and is used
  for display only; validate against the provider JWKS if it ever drives
  decisions (`rust/crates/core/src/infra/oauth/grant.rs:123-135`).
