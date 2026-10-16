# MailSlate — Performance Guidelines

| Field | Value |
| --- | --- |
| Version | 1.0 |
| Date | 2026-10-16 |
| Status | Approved for implementation |
| Related | [PRD](./PRD.md) · [ARCHITECTURE](./ARCHITECTURE.md) · [USER_FLOW](./USER_FLOW.md) · [DESIGN](./DESIGN.md) · [DEVELOPMENT_ENVIRONMENT](./DEVELOPMENT_ENVIRONMENT.md) |

Performance budgets for MailSlate v1 are **contracts, not aspirations**: every number in §2 is
verifiable with the tools in §3, and a regression against a budget is treated as a bug (filed in
Plane, fixed before the next milestone). This document also collects the mandatory engineering
rules that keep the budgets reachable — for Flutter (§4), Rust (§5), SQLite (§6), and the
network/energy layer (§7).

---

## 1. Philosophy

1. **Measure, then optimize.** No performance work without numbers from §3 tooling; no
   speculative micro-optimization that adds complexity.
2. **The budget table is the acceptance test.** PRs that touch hot paths (list rendering, sync
   loop, search, IPC, startup) must include before/after measurements in the PR description.
3. **Perceived performance beats raw speed.** Skeleton rows, optimistic UI, and cached-first
   reads (USER_FLOW) count as performance work.
4. **One regression is a bug.** Budget violations get a Plane work item with `perf` label and
   measured evidence.

---

## 2. Budgets (v1)

| # | Metric | Budget | How verified |
| --- | --- | --- | --- |
| P1 | GUI window visible, cold start | **< 2 s** | `flutter run --profile`, DevTools startup timeline |
| P2 | Inbox interactive (scrolling from cache) | **< 500 ms** after window paint | DevTools; daemon pre-warmed at login |
| P3 | UI frame time — scrolling list of **10k+ conversations** | **≤ 16 ms** (60 fps) | DevTools frame chart; profile build |
| P4 | Local FTS search, p95 @ **100k messages** | **< 100 ms** | Criterion bench (`bench_search`) + in-app timing logs |
| P5 | Memory — GUI typical session | **< 300 MB** | DevTools Memory; macOS/Windows task managers |
| P6 | Memory — daemon idle @ 5 accounts | **< 80 MB** | `ps`/Task Manager after 24 h idle soak |
| P7 | IPC round-trip (GUI ↔ daemon), p95 | **< 10 ms** | Protocol bench (`bench_ipc`) |
| P8 | New mail → OS notification (IDLE active) | **< 2 s** | Manual stopwatch test on Gmail + Outlook |
| P9 | Initial sync — 5k messages, healthy network | **< 5 min** | Sync integration test vs GreenMail; manual vs Gmail |
| P10 | Finder/search-as-you-type first paint of results | **< 150 ms** after debounce | In-app instrumentation; FTS p95 covers the query |
| P11 | Startup SQLite open + first page query (GUI) | **< 200 ms** | Tracing log `MAILSLATE_LOG=debug` markers |
| P12 | Mark-read / flag write → visible UI change | **instant** (optimistic), < 300 ms round-trip reconcile | Manual + IPC bench |

Budgets P3, P4, P6, P7 are **hard gates for M3/M5 exit criteria**; P1, P8 are user-visible
ship criteria in PRD §9.

---

## 3. Measurement methodology

### 3.1 Flutter UI

```bash
flutter run --profile -d linux        # profile build — never judge perf in debug
```

- **DevTools Performance:** frame chart for scroll jank (P3), startup timeline (P1/P2),
  rebuild counts (see §4.5 on over-rebuilding).
- **DevTools Memory:** heap snapshots at 10k conversations (P5); look for retained message
  bodies — bodies must only be reachable from disk, not caches in RAM.
- **Trace import:** `flutter run --profile --trace-startup` for deterministic startup CSV.
- Record numbers in the PR that changes a hot path.

### 3.2 Rust / engine

- **Criterion benches** — `rust/crates/core/benches/`: `bench_threading` (JWZ-lite + denorm
  recompute), `bench_search` (FTS5 query set incl. prefix + bm25 at 100k rows), `bench_parser`
  (mail-parser on representative `.eml` fixtures), `bench_ipc` (length-prefixed JSON round
  trip over loopback socket). Run: `cargo bench -p mailslate-core`.
- **Sampling profilers:** `perf` on Linux (`perf record -g cargo test --release …`),
  Instruments on macOS, ETW/WPA on Windows — only when a budget is violated.
- **Timing markers:** sync phases and DB transactions emit `MAILSLATE_LOG=debug` markers with
  durations; these are the source for P9/P11 numbers in dev builds.

### 3.3 SQLite

```sql
.timer on
EXPLAIN QUERY PLAN SELECT …;   -- required for every new query on a hot path
```

Hot path = anything on: conversation list page load, thread open, search, counts refresh,
outbox drain. Every such query must be index-backed (ARCHITECTURE §7 index list); a query plan
containing `SCAN` beyond the FTS layer requires an index + rationale in the PR.

---

## 4. Flutter rules (mandatory)

### 4.1 Lists

- Conversation list, thread view, sidebar: **`ListView.builder` only** (lazy) — never
  `ListView(children: […])` for unbounded data.
- Keyset pagination (ARCHITECTURE §6): page size 50; prefetch next page 300 px before end.
- Wrap each conversation row in `RepaintBoundary`; rows must not repaint siblings when
  hover/selection changes.
- Avoid `Opacity`, `ClipRRect`, and `BackdropFilter` inside rows; the row design (DESIGN §7) is
  flat surfaces + hairlines — draw it that way, don't clip it.

### 4.2 Build-time discipline

- `const` constructors wherever possible (lint-enforced — `flutter analyze` fails on
  `prefer_const_constructors`).
- **No I/O, JSON parsing, date formatting, or regex in `build()`** — formats cached on the
  row model at data-layer time.
- No `MediaQuery.of(context)` full-dependency reads in rows; use `MediaQuery.sizeOf`/
  `textScalerOf` to avoid rebuild storms.
- `context.watch`/`BlocBuilder` on the **smallest scope**: rows observe a per-row
  `Equatable` view-model; the list body does not rebuild when one row's flags change
  (granular selectors; ARCHITECTURE §4 BLoC rules).

### 4.3 Images & icons

- Avatars: initials only in v1 (no network image downloads) — zero image cost in lists.
- Material Symbols Rounded icons: use the font glyphs, not SVGs (`material_symbols_icons`);
  avoid raster upscaling.
- Remote content in mail bodies is blocked (PRD F4); when user opts in, sanitized HTML renders
  via `flutter_widget_from_html_core` with image loading disabled except explicit allow-list
  senders — same as security posture.

### 4.4 Animation (ties to DESIGN §9)

- Only the documented transitions (150–200 ms color/opacity/small transforms). No layout
  animations on the 44px rows; selection uses color only.
- Indeterminate motion restricted to the sync progress bar (one per surface).
- Respect `MediaQuery.disableAnimations` (reduced motion → 0 ms).

### 4.5 Rebuild hygiene

- The event stream (ARCHITECTURE §5) emits **coalesced invalidations**; BLoCs must ignore
  events for non-visible mailboxes (e.g., new mail in account B while viewing account A updates
  counts only — no list rebuild).
- Counts (sidebar) update via a dedicated count stream — never by re-querying the full list.
- Widget tests include at least one "no rebuild when unrelated event" test per feature
  (counted in coverage for M3).

### 4.6 Dispose discipline

- All controllers (`ScrollController`, `TextEditingController`, `FocusNode`) disposed —
  enforced by analyzer lints; leaks catch the eye in DevTools (P5).

---

## 5. Rust / engine rules (mandatory)

### 5.1 Threading & async

- `rusqlite` calls run inside the **single writer actor** (writes) or a small **read pool**
  (reads, 4 connections) via `spawn_blocking` — never on the async reactor thread.
- Channels between GUI/IPC and engine are **bounded** (backpressure, not unbounded queues).
- One **tokio task per account** (ARCHITECTURE §9); cross-account work never serializes behind
  a slow account — errors/backoff are per-account.
- Cancellation: removing an account cancels its task and pending outbox ops for that account.

### 5.2 Batching & transactions

- Writer actor batches up to ~500 operations per transaction or flushes at 50 ms — whichever
  comes first (ARCHITECTURE §7). No per-message autocommit.
- IMAP: **batched `UID FETCH`** in envelope chunks (500 UIDs per round per ARCHITECTURE §9);
  never per-message round-trips.
- FTS5 index updates happen in the **same writer transaction** as the row write (manual
  external-content sync, no triggers — ARCHITECTURE §7) — one write path, no drift.

### 5.3 Backoff & retry

- Reconnect: exponential backoff + jitter, cap **60 s** (PRD F2). No tight retry loops; auth
  errors do not retry until user action (re-auth card — USER_FLOW §9).
- Outbox drain: per-account FIFO, sequential per message; a failing op backs off the queue,
  not the whole engine.

### 5.4 Memory discipline

- Raw RFC822 messages are streamed to disk (`raw/` content-addressed store) — the full body is
  never held in RAM beyond the fetch buffer; attachments likewise (blobs on demand, PRD F4).
- Reuse the MIME parser instance; avoid re-parsing on every UI open — parsed text body cached as
  `message_bodies.text_body` when the message is backfilled.
- No `Vec<String>` accumulation of whole mailboxes; keyset streaming only.

### 5.5 Parsing & threading hot paths

- `mail-parser` on envelope+full only; snippet extraction during initial sync uses the same
  parse pass (single pass per message).
- JWZ-lite threading (ARCHITECTURE §10) touches only in-memory maps + one indexed UPDATE of
  denormalized `threads` counters per incoming message; no full recompute, no table scans.

---

## 6. SQLite rules (mandatory)

| Rule | Detail |
| --- | --- |
| Journal | WAL; `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout` set (ARCHITECTURE §7) |
| Checkpoints | Passive auto-checkpoint + `TRUNCATE` checkpoint when idle (daemon-owned; GUI never checkpoints) |
| Indexes | Full hot-path index list per ARCHITECTURE §7; new query → query plan check (§3.3) |
| Vacuum | `VACUUM` only in maintenance windows (Settings → daemon maintenance or idle soak); never mid-session |
| ANALYZE | After initial sync of a new account, and after large bulk imports |
| FTS weights | `bm25` weights: subject ≫ from/to > body (ARCHITECTURE §7); prefix queries only for ≥3 chars |
| Snippets | Generated lazily for the visible page; never for the whole result set |
| Blob avoidance | DB stores paths, not blobs (bodies on disk, attachment blobs content-addressed) |
| Growth target | DB ≤ ~25% of cached mail size on disk (heuristic; measure at M3 with the 10k-message fixture) |

---

## 7. Network & energy rules

- **IDLE first:** when IMAP IDLE is active (INBOX, re-issued ~29 min — RFC 2177), **no polling
  runs**. Poll fallback (IDLE-less servers) uses a 3-minute interval.
- Per account: one IMAP connection (the actor) + short-lived connections only for batch
  operations if needed; no connection-per-mailbox.
- Never re-fetch: bodies within sync window, attachments, and flags already cached are not
  downloaded again except on explicit resync or server divergence (CONDSTORE reconcile).
- Backfill is bounded: **3 concurrent fetchers per account** (ARCHITECTURE §9), yielding to
  interactive traffic; backfill pauses while the user interacts with the same account's lists
  if a P-budget would otherwise suffer.
- Send path: SMTP connection opened per send burst; connection closed after 30 s idle.

---

## 8. Startup path (P1/P2/P11)

1. Daemon starts at login → warms DB, runs checkpoints, opens IDLE connections. GUI startup
   never waits for the daemon: it opens a **read-only** pool and streams what's on disk.
2. GUI startup sequence (budgeted): open DB (< 200 ms, P11) → first page of default view
   (Inbox, keyset, 50 rows) → sidebar counts from the denormalized `threads` table → paint.
   Skeleton rows cover any gap (USER_FLOW §12) — no spinner walls.
3. Cold start with daemon down: GUI spawns daemon (ARCHITECTURE §2) but renders from cache
   immediately; sync status shows "paused" until IPC handshake completes.

---

## 9. Regression prevention

- Criterion benches + budget table live in-repo; a PR touching hot paths (list rendering, sync
  loop, search, IPC, startup) **must** paste before/after numbers.
- Budget violations → Plane work item: label `perf`, priority per global mapping, description
  carrying the measured number, fixture, and PR link.
- CI does not run perf gates (flakiness); the 3-OS debug builds in CI only catch compile-level
  regressions. Perf verification is a local/pre-release activity — done in M3 and M5 exit
  criteria.
- The 10k-conversation fixture (generated `.eml` set, checked into test fixtures as generator
  script `tool/generate_fixtures.dart`) is the standard load test corpus for P3/P4/P5.

---

## 10. Antipatterns (do not ship)

| Antipattern | Why it fails |
| --- | --- |
| `ListView` with all children | Kills P3 at 10k rows |
| Rebuilding the whole list on one flag change | Frame drops; violates §4.5 |
| Doing FTS queries on every keystroke (no debounce) | P10 violation by construction |
| Storing bodies/attachments in SQLite blobs | DB bloat, slow vacuum, memory pressure |
| Per-message transactions during sync | WAL churn, P9 violation |
| Unbounded channels between IPC and engine | Memory blowup under sync burst |
| Retry loops without backoff | Battery/network waste; server bans |
| Polling while IDLE is active | Breaks P8 expectations and wastes energy |
| Formatting dates/sizes in `build()` | Rebuild cost; see §4.2 |
| Loading mail-parser output on every scroll | CPU burn; cache parsed text (P5) |

---

## 11. Engineering minimums (AGENTS.md §6, restated as perf-relevant)

- `const` constructors wherever possible; lazy lists; no network/IO/parsing in `build()`;
  dispose all controllers (analyzer-enforced).
- 60 fps target; profile with `flutter run --profile` when in doubt.
- New features ship with tests in the same commit (unit for logic, widget for screens) — perf
  tests are the criterion benches and count as coverage for hot paths.
- Secrets never in source; keyring-only credential storage (also read/write-cost discipline —
  keyring access only on auth paths, never per request).

---

## 12. Cross-links

- [PRD](./PRD.md) — F2/F7 acceptance criteria and M3/M5 exit criteria carry P-budgets.
- [ARCHITECTURE](./ARCHITECTURE.md) — §7 DB, §9 sync engine, §5 IPC, §2 process model.
- [USER_FLOW](./USER_FLOW.md) — optimistic UI and state matrix that make budgets perceivable.
- [DEVELOPMENT_ENVIRONMENT](./DEVELOPMENT_ENVIRONMENT.md) — gates that enforce §4–§6 at commit
  and PR time.
- [DESIGN](./DESIGN.md) — motion/shadow rules that keep rendering cheap.
