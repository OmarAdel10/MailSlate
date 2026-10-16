# MailSlate — Development Environment Specification

| Field | Value |
| --- | --- |
| Version | 1.0 |
| Date | 2026-10-16 |
| Status | Approved for implementation |
| Related | [PRD](./PRD.md) · [ARCHITECTURE](./ARCHITECTURE.md) · [USER_FLOW](./USER_FLOW.md) · [DESIGN](./DESIGN.md) · [APP_ICON](./APP_ICON.md) · [PERFORMANCE_GUIDELINES](./PERFORMANCE_GUIDELINES.md) |

Everything a developer needs to build, run, test, and ship MailSlate: machine prerequisites per
OS, toolchain pins, repository layout, the FRB codegen loop, OAuth app registration walkthroughs,
the CI/CD pipelines, and the quality gates every commit must pass.

**Verified state (2026-10-16, primary dev machine):** Flutter 3.47.6 stable · Dart 3.13.5 ·
DevTools 2.60.0 · cargo 1.96.0 · rustc 1.96.0 · OpenJDK 25.0.4.1 (for GreenMail integration
tests). `flutter_rust_bridge_codegen` is installed per §4.

---

## 1. Platform prerequisites

### 1.1 All platforms

| Tool | Purpose | Notes |
| --- | --- | --- |
| Flutter SDK 3.47.6 stable | GUI app | Pin exact version (§3.1) |
| Rust stable 1.96+ | Daemon, bridge, core | `rust-toolchain.toml` pins toolchain + components |
| Git | VCS | Branch/PR rules in the repo `AGENTS.md` |
| ImageMagick ≥ 7 (`magick`) | Icon pipeline | Only needed when regenerating icons (APP_ICON §5) |
| Node.js omitted intentionally | — | No JS toolchain anywhere in this repo |

### 1.2 Linux

```bash
sudo apt install clang cmake ninja-build pkg-config libgtk-3-dev
# runtime keyring (secret store): gnome-keyring + libsecret — verify with:
secret-tool store --label=test svc test <<< check && secret-tool lookup svc test
```

- Wayland and X11 both supported (Flutter desktop).
- If no systemd user session (e.g. container), see §12.3 autostart fallback.

### 1.3 Windows

- Visual Studio 2022 Build Tools with **Desktop development with C++** workload (MSVC + Windows 10/11 SDK).
- Rust MSVC toolchain (`stable-x86_64-pc-windows-msvc`).
- No WebView2 dependency (no webview used anywhere).

### 1.4 macOS

- Xcode Command Line Tools (`xcode-select --install`); full Xcode required only for notarization.
- Rust targets: `aarch64-apple-darwin` and `x86_64-apple-darwin` (or build host-native only for dev).

### 1.5 Optional (integration tests only)

- **Java 17+** (OpenJDK 25 verified) — runs the GreenMail test server JAR (§8.2). Not needed for
  unit tests, app builds, or normal development. CI installs it via `actions/setup-java`.
- **Docker is intentionally not used anywhere in this project** — no Dockerfiles, no
  docker-compose, no container-based fixtures. Do not add one.

---

## 2. Repository layout

```
mailslate/
├── lib/                      # Flutter app (feature-first, ARCHITECTURE §4)
│   ├── core/                 # ffi, di, failures, theme, routing, logging, widgets
│   ├── features/             # shell, accounts, mailbox, reader, composer, search, settings
│   └── src/rust/             # FRB-generated Dart bindings (committed, never hand-edited)
├── rust/                     # Cargo workspace (ARCHITECTURE §3)
│   ├── crates/core/          # mailslate-core: domain + infra
│   ├── crates/protocol/      # mailslate-protocol: IPC types + framing
│   ├── crates/daemon/        # mailslate-daemon: headless sync engine binary
│   ├── crates/bridge/        # mailslate-bridge: FRB cdylib for the GUI
│   └── rust_builder/         # FRB generated plugin shim (committed)
├── specs/                    # This suite — source of truth (PRD, ARCHITECTURE, …)
├── tool/                     # generate_icons.sh, dev helpers, JAR fetcher
├── assets/branding/icon/     # Icon masters + sources (APP_ICON)
├── test/                     # Dart unit + widget tests
└── .github/workflows/        # CI/CD (§9, §10)
```

Per-folder READMEs are required — see §13.3.

---

## 3. Toolchain pinning

### 3.1 Flutter & Dart

- `.fvmrc` (FVM) or equivalent pin: **Flutter 3.47.6 stable** (Dart 3.13.5 — matches
  `pubspec.yaml` `sdk: ^3.13.5`). Document the exact version in the root README.
- Do not upgrade Flutter inside a feature branch; upgrades are their own `chore/` branch.

### 3.2 Rust

`rust-toolchain.toml` at repo root:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

Upgrade policy: same as Flutter — dedicated branch, FULL gate re-run.

### 3.3 flutter_rust_bridge — version lockstep rule

- `flutter_rust_bridge_codegen --version` **must equal** the `flutter_rust_bridge` crate
  version in `rust/crates/bridge/Cargo.toml` and the `flutter_rust_bridge` Dart dependency in
  `pubspec.yaml`. All three pinned together in one place; bumped only in a dedicated commit.
- Install/upgrade: `cargo install flutter_rust_bridge_codegen --version <pin>`.

---

## 4. First-time setup

```bash
git clone https://github.com/OmarAdel10/MailSlate.git
cd MailSlate
git checkout development

flutter pub get
cargo fetch --manifest-path rust/Cargo.toml
flutter_rust_bridge_codegen generate        # regenerate committed bindings (no-op if fresh)

flutter run -d linux                        # or -d windows / -d macos
```

**Branch rules apply from day one:** `development` and `master` accept changes **only via pull
requests** — direct pushes and force pushes are rejected by repository rules. Create work
branches per the global branch taxonomy (`feature/`, `fix/`, `chore/`, `docs/`, …).

---

## 5. Running in development

### 5.1 The two processes

| Process | Command | Notes |
| --- | --- | --- |
| GUI | `flutter run -d linux` | The FRB bridge **auto-spawns the daemon** in dev if not running |
| Daemon (standalone) | `cargo run -p mailslate-daemon` | Run manually for backend logs, breakpoints, or when testing daemon-only flows |

Logs: GUI → DevTools console; daemon → stdout with `MAILSLATE_LOG=debug` (default `info`).
Redaction rules are enforced (ARCHITECTURE §16): tokens, passwords, and message bodies are
never logged.

### 5.2 Isolated profiles (required for tests and experiments)

`MAILSLATE_DATA_DIR` overrides the profile directory. Use it for scratch runs so your real
profile stays untouched:

```bash
MAILSLATE_DATA_DIR=/tmp/mailslate-scratch flutter run -d linux
MAILSLATE_DATA_DIR=/tmp/mailslate-scratch cargo run -p mailslate-daemon
```

Wiping the scratch profile resets the database, blobs, and IPC sockets (paths per
ARCHITECTURE §16).

### 5.3 Environment variables

| Variable | Used by | Purpose |
| --- | --- | --- |
| `MAILSLATE_DATA_DIR` | GUI + daemon | Override profile dir (tests, scratch) |
| `MAILSLATE_LOG` | GUI + daemon | Log level: `error|warn|info|debug|trace` |
| `MAILSLATE_GOOGLE_CLIENT_ID` | Build/runtime | Google OAuth client ID (§7.1) |
| `MAILSLATE_MICROSOFT_CLIENT_ID` | Build/runtime | Microsoft OAuth client ID (§7.2) |

Client IDs are public identifiers (no secrets) — safe in env config, never committed to the repo.

---

## 6. The FRB codegen loop

- Rust API surface lives in `rust/crates/bridge/src/api/`. After **any** change to the bridge
  crate's public API or types:

```bash
flutter_rust_bridge_codegen generate --watch    # during dev
flutter_rust_bridge_codegen generate            # once before committing
```

- Generated artifacts (`lib/src/rust/`, `rust/rust_builder/`) are **committed** in the same
  commit as the Rust change that produced them. CI fails if codegen output is stale.
- Never hand-edit generated files. Never call `build_runner` for FRB (that is a different tool
  and is not used in this repo).

---

## 7. OAuth app registration (once per developer / per release)

Client IDs for both providers are **developer-owned registrations**; MailSlate ships no secrets.
Register once, export the IDs, develop.

### 7.1 Google Cloud (Gmail)

1. Create a project in [Google Cloud Console](https://console.cloud.google.com/).
2. **APIs & Services → OAuth consent screen**: User type `External`; add the scope
   `https://mail.google.com/`.
3. **Credentials → Create credentials → OAuth client ID → Application type: Desktop app.**
   No client secret is required for the desktop flow; the loopback redirect uses an ephemeral
   port (`http://127.0.0.1:<port>`) per RFC 8252.
4. While the app is in **Testing** publishing status: up to 100 test users, no verification —
   **but refresh tokens expire after 7 days**, so Gmail re-authorization is required weekly.
   This is a Google policy, not a MailSlate bug; the app surfaces it as a Reconnect card
   (USER_FLOW §9).
5. Production release of the app (public users) additionally requires Google's **restricted
   scope verification + CASA security assessment** for `https://mail.google.com/`. Tracked as
   an open question in PRD §11.
6. Export: `MAILSLATE_GOOGLE_CLIENT_ID=<id>`.

### 7.2 Microsoft Entra (Outlook / Office 365)

1. [Entra admin center](https://entra.microsoft.com/) → **App registrations → New registration**.
   Supported account types: any org directory + personal Microsoft accounts (Outlook.com).
2. **Authentication → Add a platform → Mobile and desktop applications**; enable the loopback
   redirect `http://localhost` (dynamic port). Enable public client flows (PKCE, no secret).
3. **API permissions → Add → Microsoft Graph → Delegated**: add
   `https://outlook.office.com/IMAP.AccessAsUser.All` and `https://outlook.office.com/SMTP.Send`,
   plus `offline_access` for refresh tokens. (These are the Exchange Online scopes from
   Microsoft's 'Authenticate an IMAP, POP, or SMTP connection using OAuth' guide — not Graph
   mail scopes.)
4. No tenant-wide admin consent is required for personal-account use; work accounts may need
   tenant consent depending on org policy.
5. Export: `MAILSLATE_MICROSOFT_CLIENT_ID=<id>`.

The daemon performs the full RFC 8252 flow (system browser → loopback listener → PKCE S256 →
token exchange → OS keyring). Details and the SASL XOAUTH2 string format: ARCHITECTURE §8.

---

## 8. Testing setup

### 8.1 What runs where

| Layer | Command | Uses |
| --- | --- | --- |
| Rust unit + DB integration | `cargo test --workspace` | Temp-file SQLite, fake infra traits |
| Rust IMAP/SMTP integration | `cargo test -p mailslate-core --features greenmail` | GreenMail JAR (§8.2) |
| IPC protocol | `cargo test -p mailslate-protocol` | Loopback sockets, no network |
| Dart unit (BLoCs, repos) | `flutter test` | Mock repos |
| Widget tests / goldens | `flutter test` | Fake FRB layer |

### 8.2 GreenMail integration server (replaces Docker)

- `tool/fetch_greenmail.sh` downloads a pinned GreenMail standalone JAR into `.tools/`
  (gitignored). One-time step, re-run only to update the pin.
- Tests spawn GreenMail on ephemeral ports (IMAP + SMTP + IMAPS), run against it, then tear it
  down. Feature-gated (`--features greenmail`) so Java-less machines still run everything else.
- Java is the only optional prerequisite (§1.5). CI installs it via `actions/setup-java`.
- **No Docker anywhere** — this is the project policy, not a temporary state.

### 8.3 Test data

- Never commit real mail, tokens, or personal identities. Fixtures = synthetic `.eml` files.
- OAuth flows are mocked in-process (fake token endpoint); only manual testing hits real
  Google/Microsoft endpoints.

---

## 9. CI — runs on **every branch and every PR**

Workflow: `.github/workflows/ci.yml`. Triggers: `push` (all branches, including `development`
and `master`) and `pull_request` (all targets).

| Job | Steps |
| --- | --- |
| `flutter` | `dart format --set-exit-if-changed lib test` · `flutter analyze` · `flutter test` |
| `rust` | `cargo fmt --check` · `cargo clippy --workspace -- -D warnings` · `cargo test --workspace` (with `setup-java` for GreenMail features) |
| `build-matrix` | Linux/Windows/macOS debug builds — runs on PRs and on `development`/`master` pushes only (cost control on feature branches) |
| `labels` | `.github/workflows/label.yml` + `.github/labeler.yml` (actions/labeler v7) — applies `type/*` and `area/*` labels (fork-safe) |

Fast feedback rule: `flutter` and `rust` jobs are the merge blockers; the 3-OS matrix validates
platform code but does not gate formatting-only runs.

### 9.1 PR review bots (configured, committed)

- **CodeRabbit** — GitHub App + `.coderabbit.yaml` (schema v2) with path instructions for
  `specs/`, `rust/`, `lib/`.
- **Codacy** — GitHub App (quality checks on PRs) + `.codacy.yml`; optional `codacy-cli-v2` for
  local runs.

Both are advisory reviewers; the authoritative gates are §13 and CI.

---

## 10. CD — `master` only, on `v*` tags

Workflow: `.github/workflows/cd.yml`. Guarded: never runs on `development` or feature branches.
Trigger: pushing a tag `vX.Y.Z` on `master` (release flow: PR from a `release/*` branch into
`master`, merge, then tag).

| Step | Tool | Artifacts |
| --- | --- | --- |
| Linux packages | fastforge | `.deb`, `.rpm`, AppImage |
| Windows installer | fastforge | Inno Setup `.exe` |
| macOS bundle | fastforge | `.dmg` |
| Code push | Shorebird (release + patches) | Desktop OTA updates — production-ready since Feb 2025 |
| Publish | GitHub Releases | Tagged release with all installers attached, auto release notes |

### 10.1 Required repository secrets

| Secret | Purpose |
| --- | --- |
| `SHOREBIRD_TOKEN` | Shorebird CLI auth (`shorebird init` run once per repo, `shorebird.yaml` committed) |
| Signing/notarization (optional) | Windows code-signing cert, macOS notarization credentials — recommended before public release, not required for internal builds |

Prerequisites documented for whoever operates releases: a Shorebird account, and the CodeRabbit
and Codacy apps installed on the repository (§9.1).

### 10.2 Release versioning

`vMAJOR.MINOR.PATCH` on `master`; `pubspec.yaml` `version:` must match the tag (CI validates).

---

## 11. Autostart installation (daemon at login)

The daemon must run at login for notifications-while-GUI-closed (ARCHITECTURE §2). Installers
(for end users) and `tool/install-autostart.sh` (for devs) create the entry; no admin rights
required on any platform.

| Platform | Mechanism | Entry |
| --- | --- | --- |
| Linux | systemd user unit | `~/.config/systemd/user/mailslate-daemon.service` (+ `systemctl --user enable`) |
| macOS | LaunchAgent | `~/Library/LaunchAgents/com.mailslate.daemon.plist` |
| Windows | Registry Run key *or* Task Scheduler at logon | `HKCU\…\Run` / logon task, no elevation |

Fallbacks: no systemd user session → XDG autostart `.desktop` (Linux); all platforms also
support on-demand spawning by the GUI (ARCHITECTURE §2). "Start at login" is a Settings toggle
(USER_FLOW §10), default ON after the first account is added.

---

## 12. Quality gates (mandatory)

### 12.1 FAST gate — before **every** commit

```bash
dart format --set-exit-if-changed lib test
flutter analyze                     # exit 0, infos are failures
cargo fmt --check
cargo clippy --workspace -- -D warnings
```

Plus staged-diff review: no secrets/keys, no `print`/`debugPrint` outside tests, no new
`TODO`/`FIXME`.

### 12.2 FULL gate — before **every** PR merge

FAST, plus:

```bash
flutter test
cargo test --workspace
flutter build linux --debug        # compile proof (per-OS builds run in CI matrix)
git diff origin/development...HEAD --stat   # scope audit — only declared domains
```

Green on `origin/development` head: sync the feature branch from `origin/development` first;
`pubspec.lock` conflicts keep development's version, then `flutter pub get`.

### 12.3 Documentation gates

- `///` doc comments on all public Dart and Rust APIs (enforced in review — CodeRabbit path
  instructions flag missing docs).
- Root `README.md` refreshed every phase; per-feature and per-crate READMEs kept current
  (§13.3).

---

## 13. Documentation policy

### 13.1 Spec suite (source of truth)

`specs/` is authoritative (§7 of the repo `AGENTS.md`): PRD, ARCHITECTURE, USER_FLOW, DESIGN,
APP_ICON, DEVELOPMENT_ENVIRONMENT, PERFORMANCE_GUIDELINES. No out-of-spec features; ambiguous
spec → ask before building. After each completed feature, specs are synced with shipped reality.

### 13.2 Repo `AGENTS.md`

Root `AGENTS.md` is the per-repo contract for agents and humans: stack map, commands, gates
(with cargo equivalents), FRB regen rule, branch/PR rules, Plane tracking workflow, and the
skill routing table. The global contract (user-level) wins on conflict.

### 13.3 README policy

- Root `README.md`: product blurb, status, highlights, quickstart, platform support, docs and
  roadmap links.
- Per-feature READMEs: `lib/features/*/README.md` and per-crate `rust/crates/*/README.md`.
- Template: purpose · structure · key files · consumes/produces (interfaces) · tests.
- The root README was refreshed on the `docs/spec-suite` branch together with this suite —
  keep it current each phase (AGENTS.md §8).

---

## 14. Milestones → branches

| Milestone (PRD §8) | Branch |
| --- | --- |
| Pipelines (CI/CD, labels, bot configs) | `chore/ci-setup` |
| Spec suite (this document set) | `docs/spec-suite` |
| M1 Foundation | `feature/m1-foundation` |
| M2 Accounts & sync | `feature/m2-accounts-sync` |
| M3 Reading & search | `feature/m3-reading-search` |
| M4 Compose & send | `feature/m4-compose-send` |
| M5 Notifications & ship | `feature/m5-notifications-ship` |

`chore/ci-setup` lands **first** — pipelines must run on every PR from the very first feature
branch. Work items are tracked in Plane per the global workflow; branch taxonomy, PR-required
rules, and merge protocol come from the global `AGENTS.md`.

---

## 15. Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `mismatched function signature` / stale FFI at runtime | FRB codegen not re-run | `flutter_rust_bridge_codegen generate`; check §3.3 version lockstep |
| Keyring errors on Linux | No secret-service running | Start `gnome-keyring-daemon` / desktop session; verify with `secret-tool` |
| Daemon not spawning in dev | Stale socket / binary missing | Check profile dir sockets (ARCHITECTURE §16); `cargo build -p mailslate-daemon` |
| GreenMail tests skipped | Java missing | Install JDK 17+ or run without `--features greenmail` |
| `flutter analyze` passes locally, CI fails | Different Flutter version | Match `.fvmrc` pin (§3.1) |
| Gmail asks for re-auth weekly | Testing-status 7-day refresh token expiry | Expected; §7.1 item 4 |

---

## 16. Cross-links

- [ARCHITECTURE](./ARCHITECTURE.md) — process model, IPC, paths/config (§16), OAuth (§8).
- [PRD](./PRD.md) — milestones, open questions (Google verification, signing certs, icon art).
- [USER_FLOW](./USER_FLOW.md) — Settings & lifecycle (autostart toggle, quit semantics).
- [DESIGN](./DESIGN.md) — no build impact; referenced by UI work only.
- [APP_ICON](./APP_ICON.md) — icon pipeline uses §1.1 ImageMagick prerequisite.
- [PERFORMANCE_GUIDELINES](./PERFORMANCE_GUIDELINES.md) — budgets verified via §12 gates and profiling recipes.
