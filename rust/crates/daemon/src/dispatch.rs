//! Request dispatch: the daemon side of the method table.
//!
//! Only `ping` and `shutdown` exist in M1; the mail methods land with their
//! features (ARCHITECTURE.md §5.3).

use mailslate_protocol::{BridgeError, ErrorKind, PROTOCOL_VERSION, PingResult, methods};
use serde_json::{Value, json};

/// What the connection handler should do after replying.
#[derive(Debug)]
pub enum Dispatched {
    /// Reply and keep serving.
    Reply(Value),
    /// Reply, then shut the daemon down.
    Stop(Value),
}

/// Dispatches one method call.
///
/// Unknown methods are a `not_found` error, never a panic.
pub fn dispatch(method: &str) -> Result<Dispatched, BridgeError> {
    match method {
        methods::PING => Ok(Dispatched::Reply(ping_result())),
        methods::SHUTDOWN => Ok(Dispatched::Stop(json!({ "ok": true }))),
        other => Err(BridgeError::new(
            ErrorKind::NotFound,
            format!("unknown method: {other}"),
        )),
    }
}

/// The `ping` payload: both sides' versions for the handshake.
pub fn ping_result() -> Value {
    serde_json::to_value(PingResult {
        protocol_version: PROTOCOL_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").to_string(),
    })
    .expect("PingResult serializes to JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_returns_protocol_and_daemon_versions() {
        let Dispatched::Reply(value) = dispatch(methods::PING).unwrap() else {
            panic!("ping must not stop the daemon");
        };
        assert_eq!(value["protocol_version"], json!(PROTOCOL_VERSION));
        assert_eq!(value["daemon_version"], json!(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn shutdown_replies_and_stops() {
        let Dispatched::Stop(value) = dispatch(methods::SHUTDOWN).unwrap() else {
            panic!("shutdown must stop the daemon");
        };
        assert_eq!(value, json!({ "ok": true }));
    }

    #[test]
    fn unknown_methods_are_not_found() {
        let err = dispatch("nope").unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.message.contains("nope"));
    }
}
