# MailSlate

A high-performance, local-first desktop email client — Flutter GUI, Rust sync daemon,
SQLite + FTS5 local store. Direct IMAP/SMTP with OAuth2 for Gmail, Outlook, and any
generic IMAP provider.

> **Status: pre-alpha.** The v1 specification suite is complete (`specs/`); implementation
> starts with milestone M1 (see [Roadmap](#roadmap)). No packages are published yet.

## Highlights (v1 scope)

- **Multi-account** — Gmail, Outlook/Microsoft 365, and generic IMAP (Yahoo, Fastmail, iCloud,
  self-hosted) with OAuth2 (XOAUTH2) or app passwords. Multiple accounts side by side, plus a
  unified All-Accounts view.
- **Local-first & offline** — mail within your sync window is cached locally and fully readable
  and searchable without a network connection. Mutations queue and replay.
- **Instant search** — SQLite FTS5 over subject, sender, recipients, and body (`from:`,
  `to:`, `subject:`, `in:`, `tag:`, `is:unread`, `has:attachment`).
- **Conversation threading** on by default, Gmail-style.
- **Tags** — Gmail labels mirror as tags; local-only tags work on every account.
- **A real send stack** — compose, reply, reply-all, forward, attachments, autosaved drafts,
  optimistic Sent with an outbox that survives offline and crashes.
- **Notifications that outlive the window** — a background daemon keeps syncing and firing
  native notifications after you close the app.
- **Calm, monochrome UI** — a Cal.com-derived design system with light and dark themes that
  follow the system.

## Architecture in one paragraph

A headless **Rust daemon** owns all mail logic: IMAP/SMTP connections, the SQLite database
(single writer), FTS5 indexing, the outbox, OAuth token handling (OS keyring), and
notifications. The **Flutter GUI** is a read-only viewer plus command surface: it reads a WAL
snapshot and talks to the daemon over a local IPC socket, with a typed
[flutter_rust_bridge](https://cjycode.com/flutter_rust_bridge/) v2 API. Closing the GUI does not
stop mail; quitting the app does not stop notifications. Full detail:
[specs/ARCHITECTURE.md](./specs/ARCHITECTURE.md).

## Platforms

| Platform | Packages (CD) | Requirements |
| --- | --- | --- |
| Linux | `.deb`, `.rpm`, AppImage | GTK 3, a secret-service keyring (gnome-keyring) |
| Windows | Installer (`.exe`) | Windows 10+ |
| macOS | `.dmg` | macOS 12+ |

## Quickstart (developers)

Prerequisites per OS, toolchain pins, and OAuth client registration walkthroughs live in
[specs/DEVELOPMENT_ENVIRONMENT.md](./specs/DEVELOPMENT_ENVIRONMENT.md).

```bash
git clone https://github.com/OmarAdel10/MailSlate.git
cd MailSlate

flutter pub get
cargo fetch --manifest-path rust/Cargo.toml
flutter_rust_bridge_codegen generate

flutter run -d linux        # or -d windows / -d macos
```

Scratch profile (keeps your real mail data untouched):

```bash
MAILSLATE_DATA_DIR=/tmp/mailslate-scratch flutter run -d linux
```

## Project layout

```
lib/        Flutter GUI (core/ + features/{shell,accounts,mailbox,reader,composer,search,settings})
rust/       Cargo workspace (crates: core, protocol, daemon, bridge)
specs/      Spec suite — the source of truth
tool/       Helper scripts (icons, test fixtures, GreenMail fetcher)
.github/    CI (every branch + PRs), CD (master + v* tags)
```

## Documentation

| Spec | What it covers |
| --- | --- |
| [PRD](./specs/PRD.md) | Features F1–F10, milestones, metrics, open questions |
| [ARCHITECTURE](./specs/ARCHITECTURE.md) | Process model, IPC, crates, DB schema, sync engine, ADRs |
| [USER_FLOW](./specs/USER_FLOW.md) | Every flow, keyboard map, state matrix |
| [DESIGN](./specs/DESIGN.md) | Design system (Cal.com-derived, light + dark) |
| [APP_ICON](./specs/APP_ICON.md) | Brand mark brief + icon pipeline |
| [DEVELOPMENT_ENVIRONMENT](./specs/DEVELOPMENT_ENVIRONMENT.md) | Setup, toolchains, CI/CD, troubleshooting |
| [PERFORMANCE_GUIDELINES](./specs/PERFORMANCE_GUIDELINES.md) | Performance budgets and rules |
| [AGENTS.md](./AGENTS.md) | Repository contract (agents + humans) |

## Roadmap

| Milestone | Scope | Branch |
| --- | --- | --- |
| Pipelines | CI on every branch/PR, labels, review bots, CD skeleton | `chore/ci-setup` |
| M1 | Foundation: shells, DI, theme, FRB bridge, daemon bootstrap | `feature/m1-foundation` |
| M2 | Accounts & sync: OAuth, IMAP IDLE, SQLite schema, FTS5, threading | `feature/m2-accounts-sync` |
| M3 | Reading & search: 3-pane UI, thread reader, search, triage | `feature/m3-reading-search` |
| M4 | Compose & send: composer, drafts, outbox, SMTP | `feature/m4-compose-send` |
| M5 | Notifications & ship: daemon notifications, packaging, releases | `feature/m5-notifications-ship` |

## Contributing

- `development` and `master` accept changes **only via pull requests**; direct and force
  pushes are rejected.
- Commits follow the emoji + Conventional Commits convention; every commit passes the FAST
  gate (`dart format`, `flutter analyze`, `cargo fmt`, `cargo clippy`).
- CI runs on every branch and every PR. CD runs only on `master` `v*` tags.
- No Docker anywhere in the toolchain — integration fixtures use a standalone GreenMail JAR.

Read [AGENTS.md](./AGENTS.md) before opening your first PR.
