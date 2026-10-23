//! Search API: full-text search across one account's messages (PRD F7).
//!
//! Every call opens a fresh IPC connection, asks the daemon to run the
//! search, and maps the wire reply onto typed results. The daemon owns
//! the FTS5 index; the GUI never touches the database.

use mailslate_core::infra::{ipc::Client, paths};
use serde_json::Value;

use super::system::DaemonError;

/// One search hit, as `search` returns it.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// Row id of the matched message.
    pub message_id: i64,
    /// Account the message belongs to.
    pub account_id: i64,
    /// Message subject, when stored.
    pub subject: Option<String>,
    /// Highlighted snippet around the match, when available.
    pub snippet: Option<String>,
}

/// Result of [`search`]: the matched messages and the total count.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Matched messages, newest first, up to the page limit.
    pub messages: Vec<SearchHit>,
    /// Total matches across all pages (not just this page).
    pub total: i64,
}

/// Full-text search across `account_id`'s messages using `query`.
///
/// The query may use the syntax subset: `from:/to:/subject:/in:/tag:/`
/// `is:unread:/is:starred:/has:attachment` plus free text.
pub async fn search(account_id: i64, query: String) -> Result<SearchResult, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .search(account_id, &query)
        .await
        .map_err(DaemonError::from)?;
    parse_search(&reply)
}

/// Opens a client to the daemon for `data_dir`, spawning one on demand.
async fn connect() -> Result<Client, DaemonError> {
    Client::connect_or_spawn(&paths::data_dir())
        .await
        .map_err(DaemonError::from)
}

/// Parses the search reply into typed results; a malformed reply is an
/// internal error, never a panic.
fn parse_search(reply: &Value) -> Result<SearchResult, DaemonError> {
    let messages = reply
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| internal("search reply carries no messages array"))?;
    let mut hits = Vec::new();
    for row in messages {
        hits.push(SearchHit {
            message_id: field_i64(row, "message_id")?,
            account_id: field_i64(row, "account_id")?,
            subject: row
                .get("subject")
                .and_then(Value::as_str)
                .map(str::to_string),
            snippet: row
                .get("snippet")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    let total = field_i64(reply, "total")?;
    Ok(SearchResult {
        messages: hits,
        total,
    })
}

/// Pulls an integer field out of a hit row.
fn field_i64(row: &Value, key: &str) -> Result<i64, DaemonError> {
    row.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| internal(format!("search hit field {key} is malformed")))
}

/// A malformed daemon reply, mapped like a wire `internal` error.
fn internal(message: impl Into<String>) -> DaemonError {
    DaemonError {
        kind: "internal".to_string(),
        message: message.into(),
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::system::tests::ENV_LOCK;
    use mailslate_protocol::async_io::{read_json_frame_async, write_json_frame_async};
    use mailslate_protocol::message::{Request, Response};
    use mailslate_protocol::methods;
    use mailslate_protocol::{BridgeError, ErrorKind, PROTOCOL_VERSION};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::net::UnixListener;

    /// Recorded `(method, params)` pairs from every served request.
    type Recorded = Arc<Mutex<Vec<(String, Value)>>>;

    /// Per-fake-daemon reply behavior.
    #[derive(Clone, Copy, PartialEq)]
    enum Behavior {
        /// Canned happy-path replies.
        Normal,
        /// `search` replies without a `messages` array.
        MalformedMessages,
    }

    /// The canned result payload for one request.
    fn canned(method: &str) -> Value {
        match method {
            methods::SEARCH => json!({
                "messages": [
                    {
                        "message_id": 3,
                        "account_id": 1,
                        "subject": "Quarterly report",
                        "snippet": "the budgets look fine",
                    },
                ],
                "total": 1,
            }),
            _ => json!({}),
        }
    }

    /// Serves requests until the listener errors: canned replies by
    /// method, and every request is recorded.
    async fn serve(listener: UnixListener, recorded: Recorded, behavior: Behavior) {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let recorded = Arc::clone(&recorded);
            tokio::spawn(async move {
                while let Ok(request) = read_json_frame_async::<_, Request>(&mut stream).await {
                    recorded
                        .lock()
                        .expect("record lock")
                        .push((request.method.clone(), request.params.clone()));
                    let result = match behavior {
                        Behavior::MalformedMessages if request.method == methods::SEARCH => {
                            json!({ "nope": true })
                        }
                        _ => canned(&request.method),
                    };
                    let response = Response::ok(request.id, result);
                    if write_json_frame_async(&mut stream, &response)
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            });
        }
    }

    /// The fake daemon plus the env it redirected; `restore` puts the
    /// environment back and drops the socket directory.
    struct FakeDaemon {
        _dir: tempfile::TempDir,
        saved_data: Option<std::ffi::OsString>,
        saved_xdg: Option<std::ffi::OsString>,
    }

    impl FakeDaemon {
        fn restore(self) {
            match self.saved_data {
                Some(value) => std::env::set_var("MAILSLATE_DATA_DIR", value),
                None => std::env::remove_var("MAILSLATE_DATA_DIR"),
            }
            match self.saved_xdg {
                Some(value) => std::env::set_var("XDG_RUNTIME_DIR", value),
                None => std::env::remove_var("XDG_RUNTIME_DIR"),
            }
        }
    }

    /// Binds a fake daemon at the data dir the bridge resolves and
    /// redirects the env to it.
    async fn fake_daemon(recorded: &Recorded, behavior: Behavior) -> FakeDaemon {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket).expect("bind fake daemon");
        let rec = Arc::clone(recorded);
        tokio::spawn(async move {
            serve(listener, rec, behavior).await;
        });
        let saved_data = std::env::var_os("MAILSLATE_DATA_DIR");
        let saved_xdg = std::env::var_os("XDG_RUNTIME_DIR");
        std::env::set_var("MAILSLATE_DATA_DIR", dir.path());
        std::env::remove_var("XDG_RUNTIME_DIR");
        FakeDaemon {
            _dir: dir,
            saved_data,
            saved_xdg,
        }
    }

    /// The single recorded request for `method`.
    fn recorded_request(recorded: &Recorded, method: &str) -> Value {
        let requests = recorded.lock().expect("record lock");
        requests
            .iter()
            .find(|(name, _)| name == method)
            .map(|(_, params)| params.clone())
            .unwrap_or_else(|| panic!("no {method} request was recorded"))
    }

    #[tokio::test]
    async fn search_returns_hits_and_total() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let result = search(1, "budgets".to_string()).await.expect("search");

        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].message_id, 3);
        assert_eq!(result.messages[0].account_id, 1);
        assert_eq!(
            result.messages[0].subject.as_deref(),
            Some("Quarterly report")
        );
        assert_eq!(
            result.messages[0].snippet.as_deref(),
            Some("the budgets look fine")
        );
        assert_eq!(result.total, 1);

        let params = recorded_request(&recorded, methods::SEARCH);
        assert_eq!(params["account_id"], json!(1));
        assert_eq!(params["query"], json!("budgets"));

        daemon.restore();
    }

    #[tokio::test]
    async fn search_maps_malformed_replies_to_internal() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::MalformedMessages).await;

        let error = search(1, "budgets".to_string())
            .await
            .expect_err("malformed reply must fail");
        assert_eq!(error.kind, "internal");
        assert!(error.message.contains("messages"));

        daemon.restore();
    }
}
