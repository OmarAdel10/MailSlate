//! MailSlate core: domain engines and infrastructure adapters.
//!
//! This crate owns all mail state: the per-account sync engines, the mailbox
//! and tag model, threading, search, and the SQLite storage layer
//! (ARCHITECTURE.md §3). The GUI never runs this code in-process — it talks
//! to the daemon over IPC; see `mailslate-protocol`.
//!
//! The module layout (`domain/`, `infra/`) lands with M1/M2. This skeleton
//! exists so the workspace, CI, and the crate contracts are in place from
//! day one.

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
        assert_eq!(
            version.split('.').count(),
            3,
            "expected semver, got {version}"
        );
    }
}
