//! Infrastructure adapters: paths, database, keyring.
//!
//! Concrete implementations behind the traits in `domain/` (ARCHITECTURE.md
//! §3.1); the daemon injects them at startup. `paths` is pure resolution;
//! `db` and `keyring` touch the outside world.

pub mod blobs;
pub mod db;
#[cfg(unix)]
pub mod events;
pub mod imap;
#[cfg(unix)]
pub mod ipc;
pub mod keyring;
pub mod mime;
pub mod mime_out;
pub mod oauth;
pub mod paths;
pub mod sanitize;
