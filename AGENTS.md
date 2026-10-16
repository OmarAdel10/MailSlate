# AGENTS.md — MailSlate Project Contract

This file is the per-repository contract for agents and humans working on MailSlate. It
**tightens** the global user-level `AGENTS.md` for this project; where the two conflict, the
global file wins. The global file remains authoritative for worktree policy, branch lifecycle,
merge gates, Plane lifecycle rules, severity policy, and skill routing categories — this file
adds the project-specific facts and commands.

---

## 1. What this repo is

**MailSlate** — a high-performance, local-first desktop email client. Flutter GUI + Rust
daemon connected through `flutter_rust_bridge` v2 (dart:ffi). SQLite/FTS5 local store. Direct
IMAP/SMTP with OAuth2 for Gmail, Outlook, and generic IMAP providers.

The `specs/` suite is the source of truth:

| Spec | Content |
| --- | --- |
| [PRD](./specs/PRD.md) | Features F1–F10, milestones M1–M5, success metrics, open questions |
| [ARCHITECTURE](./specs/ARCHITECTURE.md) | Daemon/GUI process model, IPC protocol, crates, DB schema, sync engine, ADRs |
| [USER_FLOW](./specs/USER_FLOW.md) | 3-pane shell, every flow, keyboard map, state matrix |
| [DESIGN](./specs/DESIGN.md) | Cal.com-derived design system (light + dark) |
| [APP_ICON](./specs/APP_ICON.md) | Brand mark brief + icon pipeline (art pending) |
| [DEVELOPMENT_ENVIRONMENT](./specs/DEVELOPMENT_ENVIRONMENT.md) | Toolchains, setup, OAuth registration, CI/CD, troubleshooting |
| [PERFORMANCE_GUIDELINES](./specs/PERFORMANCE_GUIDELINES.md) | Budgets P1–P12 + mandatory perf rules |
| [DESIGN-cal.md](./specs/DESIGN-cal.md) | Committed source artifact for DESIGN.md — not normative on its own |

## 2. Repository map

```
lib/        Flutter GUI (core/ + features/ — ARCHITECTURE §4)
rust/       Cargo workspace: crates/{core,protocol,daemon,bridge} (ARCHITECTURE §3)
specs/      Spec suite (normative)
tool/       generate_icons.sh, fetch_greenmail.sh, fixtures generator
.github/    workflows (ci.yml, cd.yml, label.yml), labeler.yml
```

The GUI never writes the database. The daemon owns all mail state and is the single SQLite
writer; the GUI reads a WAL snapshot read-only and talks to the daemon over IPC
(ARCHITECTURE §1, §5).

## 3. Commands

| Task | Command |
| --- | --- |
| Setup | `flutter pub get` · `cargo fetch --manifest-path rust/Cargo.toml` |
| FRB codegen | `flutter_rust_bridge_codegen generate` (add `--watch` while developing) |
| Run GUI (dev) | `flutter run -d linux` (or `-d windows` / `-d macos`) |
| Run daemon standalone | `cargo run -p mailslate-daemon` |
| Dart format/lint | `dart format --set-exit-if-changed lib test` · `flutter analyze` |
| Rust format/lint | `cargo fmt --check` · `cargo clippy --workspace -- -D warnings` |
| Dart tests | `flutter test` |
| Rust tests | `cargo test --workspace` (+ `--features greenmail` for IMAP/SMTP integration — needs Java, no Docker) |
| Scratch profile | `MAILSLATE_DATA_DIR=/tmp/mailslate-scratch …` |
| Debug logs | `MAILSLATE_LOG=debug …` |

Toolchain pins: Flutter 3.47.6 (`.fvmrc`), Rust stable (`rust-toolchain.toml`). Details and
prerequisites: DEVELOPMENT_ENVIRONMENT §1–§3.

## 4. Quality gates

### FAST — before **every** commit

```bash
dart format --set-exit-if-changed lib test
flutter analyze                 # exit 0; infos are failures
cargo fmt --check
cargo clippy --workspace -- -D warnings
```

Then inspect the staged diff: **no** secrets/keys, **no** `print`/`debugPrint` outside tests,
**no** new `TODO`/`FIXME`.

### FULL — before **every** merge into `development`

FAST, plus `flutter test`, `cargo test --workspace`, a debug build of the touched platform,
feature branch synced from `origin/development`, and scope audit
(`git diff origin/development...HEAD --stat` — declared domains only).

Hot-path PRs (list rendering, sync loop, search, IPC, startup) must include before/after
numbers against the PERFORMANCE_GUIDELINES budgets. Full gate mechanics, severity policy, and
Gate 1/Gate 2 sequence: global `AGENTS.md` §6, §8, §10.

## 5. flutter_rust_bridge rules

- Version lockstep: `flutter_rust_bridge_codegen --version` **==** the `flutter_rust_bridge`
  crate version in `rust/crates/bridge/Cargo.toml` **==** the Dart dependency version. Bump all
  three in one dedicated commit.
- Generated artifacts (`lib/src/rust/`, `rust/rust_builder/`) are **committed** in the same
  commit as the Rust change that produced them. CI fails on stale codegen.
- Never hand-edit generated files. `build_runner` is not used in this repo.

## 6. Branch, PR & commit rules

- Branch taxonomy: `feature/`, `fix/`, `refactor/`, `perf/`, `chore/`, `docs/`, `release/` —
  per global `AGENTS.md` §3.
- **`development` and `master` accept changes only via pull requests.** Direct pushes and force
  pushes are rejected by repository rules; never rewrite shared history.
- **Never delete branches** (local or remote) — audit trail (global §3).
- `chore/ci-setup` lands **before the first feature branch**: CI must run on every PR from day
  one (global §9).
- Commits: **emoji-commit skill**, always. Stage by explicit path only — `git add -A` / `git
  add .` are forbidden. One coherent unit per commit; generated code only alongside its source.
- Milestone branches: `feature/m1-foundation`, `feature/m2-accounts-sync`,
  `feature/m3-reading-search`, `feature/m4-compose-send`, `feature/m5-notifications-ship`
  (PRD §8).

## 7. Spec discipline

- No out-of-spec features. Spec ambiguous → ask; never guess.
- Spec changes ride `docs/` branches with their own PR; DESIGN-cal.md is source material,
  DESIGN.md is normative.
- After every completed feature, specs are synced to shipped reality (global workflow:
  `filling-gaps`).
- PRD §11 open questions (Google restricted-scope verification, signing/notarization certs,
  icon art delivery) are tracked there — check before release work.

## 8. Documentation policy

- Root `README.md`: refreshed every phase.
- Per-feature/per-crate READMEs (`lib/features/*/README.md`, `rust/crates/*/README.md`) —
  template: **purpose · structure · key files · consumes/produces · tests**.
- `///` doc comments on every public Dart and Rust API (review-enforced; CodeRabbit path
  instructions flag gaps).

## 9. Plane tracking

- Work items: state lifecycle per global `AGENTS.md` §13 (Backlog → Todo → Planning → In
  Progress → Review → Done). **Never delete** work items — close by state change.
- Plane project identifier: **MAILSLATE** (created 2026-10-16). **Never guess identifiers,
  states, or labels** — list them first (`planecli state list --project MAILSLATE`,
  `planecli label list --project MAILSLATE`); create only what is missing.
- Every out-of-scope finding gets a Plane work item in the same session (global §4/§13), with
  priority/labels derived from the finding — never defaults.
- GitHub PR labels (`type/*`, `area/*`, applied by `.github/labeler.yml`) mirror work-item
  labels on the Plane side where applicable.

## 10. CI/CD

- **CI runs on every branch and every PR** (`ci.yml`): dart format, `flutter analyze`,
  `flutter test`, `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`;
  3-OS build matrix on PRs and `development`/`master` pushes.
- **CD is `master`-only, `v*` tags** (`cd.yml`): fastforge builds `.deb`/`.rpm`/AppImage, Inno
  Setup `.exe`, `.dmg`; Shorebird release + patches; GitHub Release with artifacts.
- **No Docker anywhere** — not for tests, not for dev, not for CI (DEVELOPMENT_ENVIRONMENT
  §1.5, §8.2). Integration fixtures use the GreenMail JAR.

## 11. Skill routing (project-relevant)

| Situation | Skill |
| --- | --- |
| Every commit | `emoji-commit` |
| Merge into `development` | `merge-message` + global merge sequence |
| Code-change task complete | `implement-review-ship` loop (fresh reviewers, quality gate, then PR) |
| Feature complete | `qa-post-implementation` → `test-coverage-analysis` → `filling-gaps` |
| Bug found | `systematic-debugging` before proposing fixes |
| Claiming done/passing | `verification-before-completion` (fresh output required) |
| New logic | `test-driven-development` |
| New window/UI feature | Check `specs/DESIGN.md` + `specs/USER_FLOW.md` first |

## 12. Engineering minimums (project edition)

- PERFORMANCE_GUIDELINES §11 applies as written: `const` everywhere possible, dispose all
  controllers, no IO/parsing/formatting in `build()`, lazy lists, profile before optimizing.
- Secrets only in the OS keyring (ARCHITECTURE §12); tokens/passwords/bodies never logged
  (redaction rules, ARCHITECTURE §16).
- New features ship with tests in the same commit; hot paths additionally get bench coverage.
- IPC and DB access go through the layers in ARCHITECTURE §3–§4 — no shortcuts around the
  daemon ownership model (the GUI must never open the DB read-write).
