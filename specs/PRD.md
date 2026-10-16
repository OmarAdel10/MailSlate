# PRD — MailSlate

| | |
|---|---|
| **Product** | MailSlate |
| **Version** | 1.0 (v1 scope) |
| **Date** | 2026-10-16 |
| **Status** | Approved — source of truth per AGENTS.md §7 |
| **Related specs** | [ARCHITECTURE.md](ARCHITECTURE.md) · [USER_FLOW.md](USER_FLOW.md) · [DESIGN.md](DESIGN.md) · [APP_ICON.md](APP_ICON.md) · [DEVELOPMENT_ENVIRONMENT.md](DEVELOPMENT_ENVIRONMENT.md) · [PERFORMANCE_GUIDELINES.md](PERFORMANCE_GUIDELINES.md) |

---

## 1. Overview

MailSlate is a **local-first desktop email client** for Linux, Windows, and macOS. A Flutter frontend renders the UI; a Rust core (via `flutter_rust_bridge` FFI) implements everything else: IMAP/SMTP protocol handling, OAuth2, the sync engine, full-text search (SQLite FTS5), and the local database. A small background daemon keeps accounts synced and delivers native notifications **even when the app window is closed**.

MailSlate connects directly to mail providers (no intermediary server): Gmail, Outlook/Microsoft 365, and any standard IMAP/SMTP server. All mail data lives on the user's machine. There is no MailSlate account, no telemetry, and no cloud dependency.

## 2. Goals

- **G1 — Correct mail, offline.** Full local cache: reading, search, and triage work with zero network. Sending and flag changes queue and drain when connectivity returns.
- **G2 — Fast.** Desktop-native responsiveness: cold start < 2 s, Inbox interactive < 500 ms, search p95 < 100 ms at 100k messages, 60 fps at 10k+ conversations.
- **G3 — Always-on mail.** Background daemon syncs continuously (IMAP IDLE) and raises system notifications while the UI is closed.
- **G4 — Multi-account, unified.** Any mix of Gmail / Outlook / generic IMAP accounts side by side, with unified cross-account views and a single FTS5 search across all mail.
- **G5 — Cal.com-grade calm.** The design system (see [DESIGN.md](DESIGN.md), derived from `DESIGN-cal.md`) delivers a monochrome, hairline-structured, keyboard-friendly interface — no clutter, no accent-color noise.

## 3. Non-goals (v1)

| Out of scope | Note |
|---|---|
| Mobile (Android/iOS) | Desktop-only v1. Architecture does not preclude later mobile. |
| End-to-end encryption (PGP/S-MIME) | Future milestone; schema reserves no fields yet. |
| Calendar / invitations | ICS attachments render as files; no invite actions. |
| Server-side filters/rules engine | Triage is manual/UI-driven in v1. |
| Multi-window / pop-out composer | Composer is an in-app route (see [USER_FLOW.md](USER_FLOW.md)). |
| Command palette (Cmd+K) | v2. |
| Compact/dense list mode | v2. |
| Attachment text indexing in search | v1 indexes headers + bodies only. |
| SQLCipher DB encryption | Documented upgrade path; v1 relies on OS disk encryption (see §7). |
| Telemetry / analytics / crash reporting to third parties | None. Ever, in v1. |
| i18n / localization | English-only v1; strings centralized in Dart for future extraction. |
| Contacts manager | v1 offers recency-based recipient autocomplete from synced mail. |

## 4. Personas & primary use cases

| Persona | Needs | Primary flows |
|---|---|---|
| **Multi-account professional** — Gmail + work Outlook + personal IMAP | One inbox view, consistent triage, background notifications | Unified Inbox, search syntax, bulk actions |
| **Keyboard power user** | Speed, shortcuts, no mouse | List navigation, archive/star/reply keys, search |
| **Privacy-conscious user** | Local data, no cloud middleman, remote images blocked | Offline reading, image blocking banner, keyring-stored credentials |

## 5. Platform & distribution

| Platform | Artifacts | Notes |
|---|---|---|
| Linux | `.deb`, `.rpm`, AppImage | Primary dev platform |
| Windows 10/11 | Inno Setup `.exe` installer | Code signing recommended |
| macOS 13+ | `.dmg` | Notarization required for distribution |

Distribution is via GitHub Releases (built in CD on `master` with `v*` tags; see [DEVELOPMENT_ENVIRONMENT.md](DEVELOPMENT_ENVIRONMENT.md)). Auto-update via Shorebird release/patch on desktop; installer-level auto-update is out of scope for v1.

## 6. Functional requirements

### F1 — Accounts & authentication

- F1.1 Connect **Gmail** via Google OAuth2 (authorization-code + PKCE, system browser, loopback redirect per RFC 8252). Scope: `https://mail.google.com/`.
- F1.2 Connect **Outlook / Microsoft 365** via Microsoft Entra OAuth2 (auth-code + PKCE). Scopes: `offline_access`, `https://outlook.office.com/IMAP.AccessAsUser.All`, `https://outlook.office.com/SMTP.Send`.
- F1.3 Connect **generic IMAP/SMTP** accounts with username/password (or app password), with auto-filled server presets for common providers (Gmail app-password, iCloud, Yahoo, Fastmail, Proton Bridge, Zoho) plus fully manual host/port/security fields.
- F1.4 **Connection test** before saving: IMAP login + mailbox list + SMTP probe. Clear per-protocol error reporting.
- F1.5 **Multi-account**: unlimited accounts; each with color dot, display name, sync settings.
- F1.6 **Token lifecycle**: access tokens refresh automatically; expired/revoked refresh tokens surface an inline *Reconnect* action (re-runs OAuth) without losing local data.
- F1.7 Credentials/tokens are stored **only** in the OS keyring (libsecret / Windows Credential Manager / Keychain). Never in the DB, config files, or logs.
- F1.8 **Remove account**: user chooses *keep local data* (offline archive, read-only) or *delete local data* (with confirmation).

*Done when:* all three account types connect, sync, send, and survive daemon+GUI restarts; token refresh verified for both OAuth providers.

### F2 — Sync engine (background, daemon-owned)

- F2.1 **Initial sync**: mailbox discovery, then envelopes for all messages, prioritized Inbox-first, visible progressively (list usable while syncing).
- F2.2 **Body backfill**: full bodies cached within the per-account sync window (default 1 year; options 30 d / 90 d / 1 y / all). Recent 30 days first, then the rest of the window. Beyond the window, headers are cached and bodies fetch on open.
- F2.3 **Live sync**: IMAP IDLE on Inbox (re-issued every ~29 min per RFC 2177) for IDLE-capable servers; polling fallback otherwise.
- F2.4 **Incremental resync**: CONDSTORE/QRESYNC `MODSEQ` deltas where supported; UID-range scan fallback; UIDVALIDITY change triggers that mailbox's full resync.
- F2.5 **Mutations**: flags, move, archive, delete, tag/label changes apply optimistically to local state and are queued to the server ([ARCHITECTURE.md](ARCHITECTURE.md) outbox). Server truth reconciles on next delta refresh.
- F2.6 **Resilience**: exponential backoff with jitter (cap 60 s) on connection failure; auth errors stop retrying and raise re-auth state; account errors are visible per account.
- F2.7 **Continuity**: sync runs in the daemon, independent of GUI lifetime. GUI quit ≠ sync stop.

*Done when:* 5k-message mailbox fully synced < 5 min on broadband; new mail appears via IDLE < 2 s; killing the GUI does not interrupt sync.

### F3 — Mailbox model & views

- F3.1 Canonical views per account and unified across accounts: **Inbox, Starred, Drafts, Sent, Spam, Trash, Archive**, plus **Important** (shown only if any connected account provides it — Gmail), and **Tags** section.
- F3.2 **Tags/labels unified**: Gmail labels appear as tags (system labels map to views; user labels map to tags, including Gmail label colors). Generic IMAP custom keywords map to tags where the server supports them; otherwise tags are local-only. User can create/rename/color/delete local tags.
- F3.3 **Threading**: messages grouped into conversations by default (Gmail thread IDs where available; otherwise References/In-Reply-To clustering). Flat-view toggle is v2.
- F3.4 Conversation list shows: participants, subject, snippet, date, thread count, attachment clip, unread state, star.
- F3.5 **Archive semantics** are provider-aware: Gmail = remove `INBOX` label; generic IMAP = move to the account's Archive mailbox (create `Archive` if absent, per user confirmation).
- F3.6 Trash/Spam: move semantics per provider; permanent delete and empty-trash actions behind confirmation.

*Done when:* every view renders correct, live-updating content across ≥2 accounts; Gmail and generic IMAP archive behave per F3.5.

### F4 — Reading

- F4.1 Reader renders **sanitized HTML** (allow-list sanitizer in Rust) or plain text; scripts, forms, and trackers never execute.
- F4.2 **Remote images blocked by default** per message, with a *Load images* banner action (per-message, session-scoped memory).
- F4.3 Thread view: newest highlighted, older collapsed (headers only), expand on click; quoted text collapsed behind a "Show quoted" chip.
- F4.4 Mark-read on open (batched to server), with unread/read toggle; keyboard-complete reading.
- F4.5 Attachments: listed as chips (name, size, type); view (system default app) or save-as; inline preview for common image types. Attachment blobs fetched on demand, cached on disk.
- F4.6 Links open in the system browser.

*Done when:* complex marketing HTML renders safely with no remote fetch by default; 50 MB attachments stream to disk without memory spikes.

### F5 — Compose & send

- F5.1 New / Reply / Reply-all / Forward, from account selector (multi-account), in-app composer route (`Cmd/Ctrl+N`).
- F5.2 Recipient fields with chips; autocomplete from recency (locally derived); validation on send.
- F5.3 Plain-text editor in v1; HTML send out of scope (documented).
- F5.4 **Draft autosave** locally every 3 s after typing; drafts sync to server Drafts mailbox (APPEND for IMAP; Gmail drafts via APPEND as well — Gmail API not used).
- F5.5 Attachments: attach via file picker; streamed at send time.
- F5.6 **Send** via SMTP (XOAUTH2 or password): optimistic append to Sent; queued in outbox; failures surface with retry/keep-as-draft options.
- F5.7 Sent-mail handling: Gmail auto-saves to Sent (we skip duplicate APPEND); generic IMAP APPENDs with `\Seen`; Message-ID stamped locally for dedupe on server echo.

*Done when:* round-trip send to a real account lands in recipient inbox and Sent exactly once (no dupes); offline send queues and drains on reconnect.

### F6 — Organization: tags, stars, flags

- F6.1 Star toggle (synced `\Flagged` / Gmail `STARRED`), unread/read, answered, junk/not-junk.
- F6.2 Tag create/rename/color/delete; assign/remove tags per conversation or selection (bulk).
- F6.3 Gmail label mutations push via IMAP (`X-GM-LABELS` / folder ops); generic IMAP tag mutations push as keyword `STORE` where supported.
- F6.4 **Drag-and-drop is v2**; v1 uses context menus and shortcut actions.

*Done when:* tag changes round-trip to Gmail web and to a generic IMAP server supporting keywords.

### F7 — Search

- F7.1 Global FTS5 search across all accounts, as-you-type with 150 ms debounce, ranked by relevance (bm25) with snippet highlighting.
- F7.2 Query syntax subset: `from:` `to:` `subject:` `in:` `tag:` `is:unread` `is:starred` `has:attachment` + free text (prefix-matched).
- F7.3 Filter chips (account, tag, unread, has:attachment) combinable with any query.
- F7.4 Results respect the local index: bodies indexed as cached (see F2.2); "not indexed yet" hint for headers-only messages.
- F7.5 Search latency p95 < 100 ms at 100k messages ([PERFORMANCE_GUIDELINES.md](PERFORMANCE_GUIDELINES.md)).

*Done when:* syntax + chips compose correctly; results open the right thread; latency budget met on the reference fixture.

### F8 — Notifications

- F8.1 New mail in Inbox raises a **native OS notification**: sender + subject + snippet. Fired by the daemon — works when the GUI is closed.
- F8.2 Click behavior: GUI closed → launches (or focuses) the app and opens that thread; GUI focused on the same thread → notification suppressed.
- F8.3 Never notify for: Spam, sent-by-self echoes, messages matching "muted" threads (no mute in v1), or while the same thread is focused.
- F8.4 Per-account notification toggles; global "Start at login" toggle (default ON once the first account is added).
- F8.5 OS-level "do not disturb" is respected as-is (no in-app DND in v1).

*Done when:* with GUI fully quit, a new message produces a notification within 2 s of IMAP IDLE delivery (see [USER_FLOW.md](USER_FLOW.md) notification flow).

### F9 — Offline, outbox & resilience

- F9.1 Reading, search, and triage on cached mail work offline; a status banner shows offline state.
- F9.2 Mutations queue in the local outbox with idempotent replay; drain order preserved (per message).
- F9.3 Reconnect replays the outbox, then refreshes deltas; conflicts resolve server-wins with prior local intent re-applied where still valid.
- F9.4 Auth expiry → inline *Reconnect* card on the account row and (if relevant) composer.
- F9.5 Crash/kill safety: outbox and DB survive process kills (WAL); daemon restarts cleanly; no duplicate sends (idempotency via queued-op state machine).

*Done when:* airplane-mode session with 10 queued ops replays exactly once on reconnect.

### F10 — Settings & lifecycle

- F10.1 Accounts: list/edit/add/remove, per-account sync window, notifications on/off, color.
- F10.2 Appearance: theme — System / Light / Dark ([DESIGN.md](DESIGN.md)).
- F10.3 Startup: "Start MailSlate in the background at login" toggle (default ON once ≥1 account exists; installs/removes the OS autostart entry).
- F10.4 Mail: default account, undo-history depth (v2), archive folder preference for generic IMAP.
- F10.5 About: version, protocol-safe acknowledgements, links to specs.
- F10.6 **Quit semantics**: closing the window quits the GUI only — daemon keeps syncing (one-time subtle hint after first close); daemon can be stopped explicitly from Settings or tray.

*Done when:* autostart entry is installed/removed per toggle on all three OSes; closing the window keeps notifications alive.

## 7. Non-functional requirements

| # | Requirement |
|---|---|
| NFR-1 | **Performance budgets** as in [PERFORMANCE_GUIDELINES.md](PERFORMANCE_GUIDELINES.md) — enforcement is part of milestone exit criteria. |
| NFR-2 | **Security**: OAuth2 + PKCE, loopback bound to `127.0.0.1` ephemeral port, `state` validated; secrets only in OS keyring; HTML sanitized allow-list in Rust; remote content blocked by default; IPC sockets user-only permissions. |
| NFR-3 | **Privacy**: zero telemetry; no requests beyond the user's own configured mail/OAuth servers. |
| NFR-4 | **Data-at-rest**: mail DB is plaintext SQLite protected by OS disk encryption; secrets never in DB; SQLCipher upgrade path documented in [ARCHITECTURE.md](ARCHITECTURE.md). |
| NFR-5 | **Reliability**: no data loss on kill -9 (WAL + outbox state machine); all sync operations idempotent. |
| NFR-6 | **Accessibility**: full keyboard operation, visible focus rings (2 px accent, see DESIGN.md), WCAG AA contrast, text scaling respected. |
| NFR-7 | **Engineering minimums** follow root `AGENTS.md` (const Widgets, no IO in `build`, controller disposal, tests with every feature commit). |

## 8. Milestones

| Milestone | Scope | Exit criteria | Branch |
|---|---|---|---|
| **M1 — Foundation** | CI/CD pipelines; cargo workspace (core/protocol/daemon/bridge); FRB integrate; DB schema + migrations runner; keyring adapter; daemon skeleton with IPC handshake; Flutter shell: 3-pane layout, design tokens light/dark, DI, routing, logging. | Pipelines green on every branch; `ping()` round-trips GUI→daemon; theme switches with system; migration runner creates v1 schema. | `chore/ci-setup` (first), then `feature/m1-foundation` |
| **M2 — Accounts & sync** | OAuth flows (Google/Microsoft); account wizard + connection test; IMAP connect; initial sync; IDLE; mailbox tree; unified views on live data; account settings. | Connect Gmail + Outlook + generic IMAP; Inbox live-updates via IDLE; sync survives GUI restart. | `feature/m2-accounts-sync` |
| **M3 — Reading, threads, search** | Sanitized HTML reader; thread grouping UI; attachments view/download; flags/star/read ops; archive/delete/spam; triage keyboard; FTS5 search + filters + syntax. | F3/F4/F6/F7 exit criteria pass on live accounts; search p95 budget met on fixture. | `feature/m3-reading-search` |
| **M4 — Compose & send** | Composer; drafts autosave + sync; SMTP send; outbox drain/retry; reply/forward; attach; Sent handling. | F5 + F9 exit criteria pass; no duplicate sends in echo tests. | `feature/m4-compose-send` |
| **M5 — Notifications & ship** | Daemon native notifications (3 OSes); autostart install/uninstall; tray icon (unread badge, optional); settings completion; packaging (deb/rpm/AppImage, exe, dmg); Shorebird wiring; perf hardening; GA checklist. | F8 + F10 exit criteria pass; installers produced by CD on a `v*` tag; budgets met on reference hardware. | `feature/m5-notifications-ship` |

Each milestone = one feature branch → PR into `development` under the full AGENTS.md gate (FAST/FULL gates, `implement-review-ship`, `qa-post-implementation`, Plane work items).

## 9. Success metrics

| Metric | Target | Measured by |
|---|---|---|
| Cold start to usable Inbox | < 2 s / < 500 ms warm | Manual + DevTools timeline |
| Search p95 @ 100k msgs | < 100 ms | criterion bench + manual |
| UI frame budget | 60 fps @ 10k conversations | DevTools profile on fixture |
| Initial 5k-message sync | < 5 min broadband | Fixture stopwatch vs GreenMail |
| IDLE → notification | < 2 s | Live Gmail test |
| Crash-free sessions | ≥ 99.5 % | Local crash log (no telemetry) |
| Outbox correctness | 100 % exactly-once replay in tests | Integration suite |

## 10. Risks & mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Google restricted-scope verification (CASA) needed for public release; Testing status = 7-day refresh-token expiry | Gmail use requires weekly re-auth in dev builds | Documented in [DEVELOPMENT_ENVIRONMENT.md](DEVELOPMENT_ENVIRONMENT.md); plan verification before public GA; re-auth flow (F1.6) makes cost visible but small |
| IMAP server quirks (IDLE-less, no CONDSTORE, keyword-less) | Sync/tag degradation | Fallback paths in F2; tags local-only where unsupported |
| HTML sanitization bypass | Security | Allow-list sanitizer + remote-content blocking + no-JS renderer; security review at Gate 1 |
| OS notification plumbing divergence (3 OSes) | M5 slippage | Isolated behind one Rust `notify` module; per-OS CI smoke |
| Daemon lifecycle complexity (autostart, single-instance) | UX bugs | Explicit dev/CI matrix; manual lifecycle test checklist per milestone |
| Plaintext DB | Privacy on shared machines | Documented; OS disk encryption assumption; SQLCipher upgrade path |

## 11. Open questions

1. Google OAuth verification timeline — can GA ship with Testing-mode Gmail until verification completes? (Decision needed before public release.)
2. Code-signing certificates (Windows EV / Apple Developer ID) procurement timing — affects M5 packaging ship.
3. App icon final artwork — placeholder derived from design tokens until user delivers masters (see [APP_ICON.md](APP_ICON.md)).

## 12. Spec index

| Spec | Role |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | System structure: daemon/GUI split, IPC, schema, sync engine, security model |
| [USER_FLOW.md](USER_FLOW.md) | Every user-facing flow, states, and edge cases |
| [DESIGN.md](DESIGN.md) | Cal.com-derived design system adapted to app chrome (light + dark) |
| [APP_ICON.md](APP_ICON.md) | Icon identity brief + per-platform technical pipeline |
| [DEVELOPMENT_ENVIRONMENT.md](DEVELOPMENT_ENVIRONMENT.md) | Toolchains, setup, OAuth registration, tests, CI/CD |
| [PERFORMANCE_GUIDELINES.md](PERFORMANCE_GUIDELINES.md) | Budgets, rules, and measurement for all layers |
| [DESIGN-cal.md](DESIGN-cal.md) | Source design analysis (Cal.com marketing system) |
