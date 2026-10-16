//! IPC protocol shared by the MailSlate daemon and the GUI bridge.
//!
//! Wire format: a 32-bit little-endian length prefix followed by a UTF-8
//! JSON payload, carried over a Unix domain socket (Linux/macOS) or a named
//! pipe (Windows). Both ends are our own crates, so this crate is the single
//! source of truth for the typed contract (ARCHITECTURE.md §5).
//!
//! - [`message`] — request/response/event envelopes.
//! - [`error`] — the `BridgeError` taxonomy crossing the boundary.
//! - [`framing`] — the length-prefixed JSON codec with a hard frame cap.
//! - [`methods`] — method-name constants for the v1 surface.

pub mod error;
pub mod framing;
pub mod message;
pub mod methods;

pub use error::{BridgeError, ErrorKind};
pub use framing::{FrameError, MAX_FRAME_BYTES, read_frame, read_json_frame, write_json_frame};
pub use message::{Event, PingResult, Request, Response};

/// Current IPC protocol version (ARCHITECTURE.md §5.2).
///
/// The GUI's first request on every connection is `ping`; a mismatch is
/// surfaced as [`ErrorKind::ProtocolMismatch`] rather than undefined behavior.
pub const PROTOCOL_VERSION: u32 = 1;

/// Returns `true` when both sides speak a compatible protocol version.
pub fn is_compatible(peer: u32) -> bool {
    peer == PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_is_compatible() {
        assert!(is_compatible(PROTOCOL_VERSION));
    }

    #[test]
    fn other_versions_are_rejected() {
        assert!(!is_compatible(PROTOCOL_VERSION + 1));
        assert!(!is_compatible(0));
        assert!(!is_compatible(u32::MAX));
    }
}
