//! Flutter bridge surface for the MailSlate GUI.
//!
//! This is the crate `flutter_rust_bridge` v2 generates its Dart bindings
//! from (lockstep version rule: AGENTS.md §5). The GUI's Dart code only ever
//! calls these functions; they are a thin client that talks to the daemon
//! over IPC and reads the local database read-only (ARCHITECTURE.md §1).
//!
//! The FRB codegen integration (`lib/src/rust/`, `rust/rust_builder/`)
//! lands with M1. Until then this crate pins the shape: small, documented,
//! side-effect-free helpers only.

/// Returns a greeting from the Rust side of the bridge.
///
/// The M1 smoke path uses this to prove the FFI link end to end.
pub fn bridge_hello() -> String {
    format!("hello from mailslate-bridge {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_mentions_crate_version() {
        assert!(bridge_hello().contains(env!("CARGO_PKG_VERSION")));
    }
}
