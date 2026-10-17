//! Error taxonomy shared across the IPC boundary (ARCHITECTURE.md §11).
//!
//! Rust sides use this type at the protocol boundary; the Dart side maps it
//! onto a sealed `AppFailure` hierarchy.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Category of a protocol-level failure.
///
/// Serialized in snake_case. New kinds are additive and must keep the
/// retryability semantics honest ([`ErrorKind::default_retryable`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Transport failure: connection dropped, timeout, DNS.
    Network,
    /// The account needs the user to re-authorize (token invalid or revoked).
    AuthExpired,
    /// Credentials were rejected by the server.
    AuthFailed,
    /// The server rejected the operation (IMAP NO/BAD, SMTP 4xx/5xx, HTTP).
    ServerRejected,
    /// The referenced object does not exist.
    NotFound,
    /// The caller supplied invalid arguments.
    Validation,
    /// Local persistence failed (SQLite, blobs).
    Storage,
    /// The daemon is not reachable.
    DaemonUnavailable,
    /// The two sides disagree on the protocol version.
    ProtocolMismatch,
    /// Anything not covered by the other kinds.
    Internal,
}

impl ErrorKind {
    /// Snake-case wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::AuthExpired => "auth_expired",
            Self::AuthFailed => "auth_failed",
            Self::ServerRejected => "server_rejected",
            Self::NotFound => "not_found",
            Self::Validation => "validation",
            Self::Storage => "storage",
            Self::DaemonUnavailable => "daemon_unavailable",
            Self::ProtocolMismatch => "protocol_mismatch",
            Self::Internal => "internal",
        }
    }

    /// Whether retrying the same operation can plausibly succeed as-is.
    pub const fn default_retryable(self) -> bool {
        matches!(self, Self::Network | Self::DaemonUnavailable)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error object carried by a failed [`crate::Response`].
///
/// `message` must be human-readable and free of secrets (ARCHITECTURE.md §12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeError {
    /// Failure category.
    pub kind: ErrorKind,
    /// Human-readable description, safe to surface in the UI.
    pub message: String,
    /// Whether retrying can plausibly succeed.
    #[serde(default)]
    pub retryable: bool,
}

impl BridgeError {
    /// Builds an error using the kind's default retryability.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retryable: kind.default_retryable(),
        }
    }

    /// Overrides the retryable flag.
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for BridgeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kinds_serialize_as_snake_case() {
        let cases = [
            (ErrorKind::Network, "network"),
            (ErrorKind::AuthExpired, "auth_expired"),
            (ErrorKind::AuthFailed, "auth_failed"),
            (ErrorKind::ServerRejected, "server_rejected"),
            (ErrorKind::NotFound, "not_found"),
            (ErrorKind::Validation, "validation"),
            (ErrorKind::Storage, "storage"),
            (ErrorKind::DaemonUnavailable, "daemon_unavailable"),
            (ErrorKind::ProtocolMismatch, "protocol_mismatch"),
            (ErrorKind::Internal, "internal"),
        ];
        for (kind, name) in cases {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(name));
            assert_eq!(kind.as_str(), name);
            let back: ErrorKind = serde_json::from_value(json!(name)).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn retryability_defaults_follow_the_kind() {
        assert!(ErrorKind::Network.default_retryable());
        assert!(ErrorKind::DaemonUnavailable.default_retryable());
        for kind in [
            ErrorKind::AuthExpired,
            ErrorKind::AuthFailed,
            ErrorKind::ServerRejected,
            ErrorKind::NotFound,
            ErrorKind::Validation,
            ErrorKind::Storage,
            ErrorKind::ProtocolMismatch,
            ErrorKind::Internal,
        ] {
            assert!(
                !kind.default_retryable(),
                "{kind} must not default to retryable"
            );
        }
    }

    #[test]
    fn new_uses_kind_default_and_can_be_overridden() {
        let err = BridgeError::new(ErrorKind::Storage, "disk busy");
        assert!(!err.retryable);
        let err = err.with_retryable(true);
        assert!(err.retryable);
        assert_eq!(err.to_string(), "storage: disk busy");
    }

    #[test]
    fn retryable_defaults_to_false_when_absent_on_the_wire() {
        let err: BridgeError =
            serde_json::from_value(json!({"kind": "storage", "message": "x"})).unwrap();
        assert!(!err.retryable);
    }

    #[test]
    fn bridge_error_roundtrips_and_is_a_std_error() {
        let err = BridgeError::new(ErrorKind::Network, "connection reset by peer");
        let back: BridgeError =
            serde_json::from_value(serde_json::to_value(&err).unwrap()).unwrap();
        assert_eq!(back, err);
        let dyn_err: &dyn std::error::Error = &err;
        assert_eq!(dyn_err.to_string(), "network: connection reset by peer");
    }
}
