//! Infrastructure adapters: paths, database, keyring.
//!
//! Concrete implementations behind the traits in `domain/` (ARCHITECTURE.md
//! §3.1); the daemon injects them at startup. `paths` is pure resolution;
//! `db` and `keyring` touch the outside world.

pub mod db;
pub mod imap;
#[cfg(unix)]
pub mod ipc;
pub mod keyring;
pub mod mime;
pub mod oauth;
pub mod paths;
