//! Threading API: conversation-level operations for the GUI (PRD F3.3).
//!
//! Every call opens a fresh IPC connection, asks the daemon to run the
//! operation, and maps the wire reply onto typed values. The daemon owns
//! the threads table; the GUI never touches the database.

use mailslate_core::infra::{ipc::Client, paths};
use serde_json::Value;

use super::system::DaemonError;

/// One conversation thread, as listed in a mailbox view.
#[derive(Debug, Clone)]
pub struct ThreadSummary {
    /// Row id of the thread.
    pub id: i64,
    /// Account the thread belongs to.
    pub account_id: i64,
    /// Normalized subject (displayed as the conversation subject).
    pub subject: Option<String>,
    /// Date of the newest message in the thread.
    pub last_date: Option<i64>,
    /// Total messages in the thread.
    pub message_count: i64,
    /// Unread messages in the thread.
    pub unread_count: i64,
    /// Whether any message in the thread has attachments.
    pub has_attachments: bool,
    /// Snippet from the newest message.
    pub last_snippet: Option<String>,
}

/// Result of [`list_threads`]: the matched threads and the total count.
#[derive(Debug, Clone)]
pub struct ThreadList {
    /// Matched threads, newest first, up to the page limit.
    pub threads: Vec<ThreadSummary>,
    /// Total threads across all pages (not just this page).
    pub total: i64,
}

/// One message within a thread, newest-first.
#[derive(Debug, Clone)]
pub struct ThreadMessage {
    /// Row id of the message.
    pub id: i64,
    /// Message subject.
    pub subject: Option<String>,
    /// JSON-serialized From addresses.
    pub from_json: Option<String>,
    /// Message date as unix epoch.
    pub date_utc: Option<i64>,
    /// Message snippet.
    pub snippet: Option<String>,
    /// Whether the message has attachments.
    pub has_attachments: bool,
    /// Body state: `headers` or `full`.
    pub body_state: String,
}

/// Result of [`list_thread_messages`]: the matched messages and total.
#[derive(Debug, Clone)]
pub struct ThreadMessageList {
    /// Matched messages, newest first, up to the page limit.
    pub messages: Vec<ThreadMessage>,
    /// Total messages across all pages (not just this page).
    pub total: i64,
}

/// Lists `account_id`'s threads newest-first, with optional keyset
/// cursor (`last_date` + `last_id`) for pagination.
pub async fn list_threads(
    account_id: i64,
    limit: u32,
    last_date: Option<i64>,
    last_id: Option<i64>,
) -> Result<ThreadList, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .get_threads(account_id, limit, last_date, last_id)
        .await
        .map_err(DaemonError::from)?;
    parse_thread_list(&reply)
}

/// Lists `thread_id`'s messages newest-first, with optional keyset
/// cursor (`last_date` + `last_id`) for pagination.
pub async fn list_thread_messages(
    thread_id: i64,
    limit: u32,
    last_date: Option<i64>,
    last_id: Option<i64>,
) -> Result<ThreadMessageList, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .get_thread_messages(thread_id, limit, last_date, last_id)
        .await
        .map_err(DaemonError::from)?;
    parse_thread_message_list(&reply)
}

/// Opens a client to the daemon for `data_dir`, spawning one on demand.
async fn connect() -> Result<Client, DaemonError> {
    Client::connect_or_spawn(&paths::data_dir())
        .await
        .map_err(DaemonError::from)
}

/// Parses the thread list reply into typed results; a malformed reply
/// is an internal error, never a panic.
fn parse_thread_list(reply: &Value) -> Result<ThreadList, DaemonError> {
    let threads = reply
        .get("threads")
        .and_then(Value::as_array)
        .ok_or_else(|| internal("get_threads reply carries no threads array"))?;
    let mut items = Vec::new();
    for row in threads {
        items.push(ThreadSummary {
            id: field_i64(row, "id")?,
            account_id: field_i64(row, "account_id")?,
            subject: row
                .get("subject")
                .and_then(Value::as_str)
                .map(str::to_string),
            last_date: row.get("last_date").and_then(Value::as_i64),
            message_count: field_i64(row, "message_count")?,
            unread_count: field_i64(row, "unread_count")?,
            has_attachments: row
                .get("has_attachments")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            last_snippet: row
                .get("last_snippet")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    let total = field_i64(reply, "total")?;
    Ok(ThreadList {
        threads: items,
        total,
    })
}

/// Parses the thread message list reply into typed results.
fn parse_thread_message_list(reply: &Value) -> Result<ThreadMessageList, DaemonError> {
    let messages = reply
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| internal("get_thread_messages reply carries no messages array"))?;
    let mut items = Vec::new();
    for row in messages {
        items.push(ThreadMessage {
            id: field_i64(row, "id")?,
            subject: row
                .get("subject")
                .and_then(Value::as_str)
                .map(str::to_string),
            from_json: row
                .get("from_json")
                .and_then(Value::as_str)
                .map(str::to_string),
            date_utc: row.get("date_utc").and_then(Value::as_i64),
            snippet: row
                .get("snippet")
                .and_then(Value::as_str)
                .map(str::to_string),
            has_attachments: row
                .get("has_attachments")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            body_state: row
                .get("body_state")
                .and_then(Value::as_str)
                .unwrap_or("headers")
                .to_string(),
        });
    }
    let total = field_i64(reply, "total")?;
    Ok(ThreadMessageList {
        messages: items,
        total,
    })
}

/// Pulls an integer field out of a row.
fn field_i64(row: &Value, key: &str) -> Result<i64, DaemonError> {
    row.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| internal(format!("thread row field {key} is malformed")))
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
    use mailslate_protocol::{methods, BridgeError, ErrorKind};
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
        /// `get_threads` replies without a `threads` array.
        MalformedThreads,
        /// `get_thread_messages` replies without a `messages` array.
        MalformedMessages,
    }

    /// The canned result payload for one request.
    fn canned(method: &str) -> Value {
        match method {
            methods::GET_THREADS => json!({
                "threads": [
                    {
                        "id": 100,
                        "account_id": 1,
                        "subject": "Project Alpha",
                        "last_date": 2000,
                        "message_count": 2,
                        "unread_count": 1,
                        "has_attachments": false,
                        "last_snippet": "alpha snippet",
                    },
                ],
                "total": 1,
            }),
            methods::GET_THREAD_MESSAGES => json!({
                "messages": [
                    {
                        "id": 2,
                        "subject": "Alpha update",
                        "from_json": "[{\"name\":\"Alice\",\"address\":\"alice@example.com\"}]",
                        "date_utc": 2001,
                        "snippet": "update snippet",
                        "has_attachments": false,
                        "body_state": "full",
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
                        Behavior::MalformedThreads if request.method == methods::GET_THREADS => {
                            json!({ "nope": true })
                        }
                        Behavior::MalformedMessages
                            if request.method == methods::GET_THREAD_MESSAGES =>
                        {
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
    async fn list_threads_returns_threads_and_total() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let result = list_threads(1, 50, None, None).await.expect("list threads");

        assert_eq!(result.threads.len(), 1);
        assert_eq!(result.threads[0].id, 100);
        assert_eq!(result.threads[0].account_id, 1);
        assert_eq!(result.threads[0].subject.as_deref(), Some("Project Alpha"));
        assert_eq!(result.threads[0].message_count, 2);
        assert_eq!(result.threads[0].unread_count, 1);
        assert_eq!(result.total, 1);

        let params = recorded_request(&recorded, methods::GET_THREADS);
        assert_eq!(params["account_id"], json!(1));
        assert_eq!(params["limit"], json!(50));

        daemon.restore();
    }

    #[tokio::test]
    async fn list_threads_maps_malformed_replies_to_internal() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::MalformedThreads).await;

        let error = list_threads(1, 50, None, None)
            .await
            .expect_err("malformed reply must fail");
        assert_eq!(error.kind, "internal");
        assert!(error.message.contains("threads"));

        daemon.restore();
    }

    #[tokio::test]
    async fn list_thread_messages_returns_messages_and_total() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let result = list_thread_messages(100, 50, None, None)
            .await
            .expect("list thread messages");

        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].id, 2);
        assert_eq!(result.messages[0].subject.as_deref(), Some("Alpha update"));
        assert_eq!(result.total, 1);

        let params = recorded_request(&recorded, methods::GET_THREAD_MESSAGES);
        assert_eq!(params["thread_id"], json!(100));
        assert_eq!(params["limit"], json!(50));

        daemon.restore();
    }

    #[tokio::test]
    async fn list_thread_messages_maps_malformed_replies_to_internal() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::MalformedMessages).await;

        let error = list_thread_messages(100, 50, None, None)
            .await
            .expect_err("malformed reply must fail");
        assert_eq!(error.kind, "internal");
        assert!(error.message.contains("messages"));

        daemon.restore();
    }
}
