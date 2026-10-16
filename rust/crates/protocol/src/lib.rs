//! IPC protocol shared between the MailSlate daemon and the GUI bridge.
//!
//! The wire format is a 32-bit little-endian length prefix followed by a
//! JSON payload, carried over a Unix domain socket (Linux/macOS) or a named
//! pipe (Windows). Both sides are our own crates, so the types here are the
//! single source of truth (ARCHITECTURE.md §5).
//!
//! The full request/response/event envelope lands with the M2 daemon work;
//! this skeleton pins the versioned handshake contract.

/// Current IPC protocol version.
///
/// The daemon and GUI perform a handshake on connect; a mismatch surfaces as
/// `protocol_mismatch` instead of undefined behavior.
pub const PROTOCOL_VERSION: u32 = 1;

/// First message exchanged on every IPC connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hello {
    /// Protocol version the sender speaks.
    pub protocol_version: u32,
}

impl Hello {
    /// Builds a hello for the current protocol version.
    pub fn current() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
        }
    }
}

/// Returns `true` when both sides speak a compatible protocol version.
pub fn is_compatible(peer: u32) -> bool {
    peer == PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_hello_is_compatible() {
        assert!(is_compatible(Hello::current().protocol_version));
    }

    #[test]
    fn other_versions_are_rejected() {
        assert!(!is_compatible(PROTOCOL_VERSION + 1));
        assert!(!is_compatible(0));
        assert!(!is_compatible(u32::MAX));
    }
}
