# Follow-ups

Out-of-scope findings parked during feature work. One line each, with the file
location. Every entry is also tracked as a Plane work item in the MAILSLATE
project (global `AGENTS.md` §4 and §13).

## Open

- MAILSLATE-8 — Docs: FAST-gate cargo commands in `specs/DEVELOPMENT_ENVIRONMENT.md`
  (§12.1) and root `AGENTS.md` (§3/§4) are written as bare `cargo ...` but assume
  the working directory `rust/`; make the cwd explicit. Found during `chore/ci-setup`.
- MAILSLATE-9 — M1: `.github/workflows/ci.yml` build-matrix is Linux-only and the
  windows/macos legs of `.github/workflows/cd.yml` auto-skip until the Windows/macOS
  Flutter scaffolds are committed (`flutter create` on each OS); restore the legs then.
- MAILSLATE-10 — M1: CI does not yet fail on stale flutter_rust_bridge codegen
  (`AGENTS.md` §5); add a freshness check once the FRB 2.13.0 toolchain is wired.
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
