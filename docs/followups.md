# Follow-ups

Out-of-scope findings parked during feature work. One line each, with the file
location. Every entry is also tracked as a Plane work item in the MAILSLATE
project (global `AGENTS.md` §4 and §13).

## Open

- MAILSLATE-8 — Docs: FAST-gate cargo commands in `specs/DEVELOPMENT_ENVIRONMENT.md`
  (§12.1) and root `AGENTS.md` (§3/§4) are written as bare `cargo ...` but assume
  the working directory `rust/`; make the cwd explicit. Found during `chore/ci-setup`.
