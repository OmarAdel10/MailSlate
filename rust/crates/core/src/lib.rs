//! MailSlate core: domain engines and infrastructure adapters.
//!
//! This crate owns all mail state: the per-account sync engines, the mailbox
//! and tag model, threading, search, and the SQLite storage layer
//! (ARCHITECTURE.md §3). The GUI never runs this code in-process — it talks
//! to the daemon over IPC; see `mailslate-protocol`.
//!
//! The module layout: `infra/` (paths, database, keyring) lands with M1;
//! `domain/` (sync engines, mailbox model, threading, search) arrives with
//! M2+.

pub mod infra;

/// Returns the crate version from `Cargo.toml`.
pub fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_version_is_semver_like() {
        let version = crate_version();
        assert!(!version.is_empty());
        let core = version.split(['-', '+']).next().unwrap_or(version);
        assert_eq!(core.split('.').count(), 3, "expected semver, got {version}");
    }
}
