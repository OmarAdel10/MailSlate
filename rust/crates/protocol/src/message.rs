//! IPC envelopes: request, response, event (ARCHITECTURE.md §5.1–§5.2).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{BridgeError, ErrorKind};

/// A call from the GUI (or a test client) to the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Correlates the response with this request; unique per connection.
    pub id: u64,
    /// Method name — see [`crate::methods`].
    pub method: String,
    /// Method arguments; `null` when the method takes none.
    #[serde(default)]
    pub params: Value,
}

impl Request {
    /// Builds a request without parameters.
    pub fn new(id: u64, method: impl Into<String>) -> Self {
        Self {
            id,
            method: method.into(),
            params: Value::Null,
        }
    }

    /// Attaches method parameters.
    pub fn with_params(mut self, params: Value) -> Self {
        self.params = params;
        self
    }
}

/// A daemon reply to a [`Request`].
///
/// On the wire: `{"id":42,"ok":true,"result":{…}}` or
/// `{"id":42,"ok":false,"error":{…}}` (ARCHITECTURE.md §5.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// `id` of the [`Request`] being answered.
    pub id: u64,
    /// `true` for a result response, `false` for an error response.
    pub ok: bool,
    /// Payload for successful calls; absent on failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Failure detail; absent on successes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<BridgeError>,
}

impl Response {
    /// Success response carrying `result`.
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Failure response carrying `error`.
    pub fn err(id: u64, error: BridgeError) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }

    /// Converts into `Result`, defending against malformed envelopes.
    ///
    /// `ok: true` without a payload yields `null` (methods may legitimately
    /// return nothing); `ok: false` without an error becomes an
    /// [`ErrorKind::Internal`] failure describing the envelope breach.
    pub fn into_result(self) -> Result<Value, BridgeError> {
        if self.ok {
            Ok(self.result.unwrap_or(Value::Null))
        } else {
            Err(self.error.unwrap_or_else(|| {
                BridgeError::new(
                    ErrorKind::Internal,
                    "malformed response: ok=false without error",
                )
            }))
        }
    }
}

/// A daemon-to-GUI event (ARCHITECTURE.md §5.4).
///
/// Events are coalesced invalidation notices, not a journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Event name, e.g. `"mail_new"`.
    pub event: String,
    /// Event payload.
    #[serde(default)]
    pub data: Value,
}

impl Event {
    /// Builds an event with a payload.
    pub fn new(event: impl Into<String>, data: Value) -> Self {
        Self {
            event: event.into(),
            data,
        }
    }
}

/// Result of the `ping` handshake call (ARCHITECTURE.md §5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingResult {
    /// Protocol version the daemon speaks.
    pub protocol_version: u32,
    /// Daemon crate version, e.g. `"0.1.0"`.
    pub daemon_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_serializes_per_spec_and_roundtrips() {
        let req = Request::new(7, crate::methods::PING);
        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value, json!({"id": 7, "method": "ping", "params": null}));
        let back: Request = serde_json::from_value(value).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn request_params_roundtrip() {
        let req = Request::new(1, "mutate").with_params(json!({"op": "flag"}));
        assert_eq!(req.params["op"], json!("flag"));
        let back: Request = serde_json::from_value(serde_json::to_value(&req).unwrap()).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn ok_response_serializes_per_spec() {
        let resp = Response::ok(
            42,
            json!({"protocol_version": 1, "daemon_version": "0.1.0"}),
        );
        assert_eq!(
            serde_json::to_value(&resp).unwrap(),
            json!({
                "id": 42,
                "ok": true,
                "result": {"protocol_version": 1, "daemon_version": "0.1.0"}
            })
        );
    }

    #[test]
    fn error_response_omits_result_and_roundtrips() {
        let resp = Response::err(9, BridgeError::new(ErrorKind::Network, "boom"));
        let value = serde_json::to_value(&resp).unwrap();
        assert_eq!(
            value,
            json!({
                "id": 9,
                "ok": false,
                "error": {"kind": "network", "message": "boom", "retryable": true}
            })
        );
        let back: Response = serde_json::from_value(value).unwrap();
        assert_eq!(back, resp);
    }

    #[test]
    fn into_result_maps_both_directions() {
        let ok = Response::ok(1, json!({"a": 1})).into_result().unwrap();
        assert_eq!(ok, json!({"a": 1}));

        let err = Response::err(1, BridgeError::new(ErrorKind::Validation, "bad args"))
            .into_result()
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
    }

    #[test]
    fn ok_without_result_yields_null() {
        let resp = Response {
            id: 3,
            ok: true,
            result: None,
            error: None,
        };
        assert_eq!(resp.into_result().unwrap(), Value::Null);
    }

    #[test]
    fn error_response_without_error_maps_to_internal() {
        let resp = Response {
            id: 3,
            ok: false,
            result: None,
            error: None,
        };
        let err = resp.into_result().unwrap_err();
        assert_eq!(err.kind, ErrorKind::Internal);
    }

    #[test]
    fn event_roundtrips() {
        let event = Event::new("mail_new", json!({"thread_id": 5}));
        let back: Event = serde_json::from_value(serde_json::to_value(&event).unwrap()).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn ping_result_roundtrips() {
        let ping = PingResult {
            protocol_version: crate::PROTOCOL_VERSION,
            daemon_version: "0.1.0".into(),
        };
        let back: PingResult =
            serde_json::from_value(serde_json::to_value(&ping).unwrap()).unwrap();
        assert_eq!(back, ping);
        let parsed: PingResult =
            serde_json::from_value(json!({"protocol_version": 1, "daemon_version": "0.1.0"}))
                .unwrap();
        assert_eq!(parsed, ping);
    }
}
