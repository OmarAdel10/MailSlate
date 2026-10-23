//! Reader API: individual message access for the GUI (PRD F3/F4).
//!
//! Every call opens a fresh IPC connection, asks the daemon to run the
//! operation, and maps the wire reply onto typed values. The daemon owns
//! all message state; the GUI never touches the database (ARCHITECTURE §1).

use mailslate_core::infra::{ipc::Client, paths};
use serde_json::Value;

use super::system::DaemonError;

/// One message's header fields, as displayed in the reader.
#[derive(Debug, Clone)]
pub struct MessageDetail {
    pub id: i64,
    pub account_id: i64,
    pub subject: Option<String>,
    pub from_json: Option<String>,
    pub to_json: Option<String>,
    pub date_utc: Option<i64>,
    pub snippet: Option<String>,
    pub has_attachments: bool,
    pub body_state: String,
}

/// One message's decoded body.
#[derive(Debug, Clone)]
pub struct MessageBody {
    pub message_id: i64,
    pub text_body: Option<String>,
    pub html_clean: Option<String>,
}

/// Result of [`get_message`]: the message detail.
#[derive(Debug, Clone)]
pub struct MessageResult {
    pub message: MessageDetail,
}

/// Result of [`get_attachments`]: the attachment list.
#[derive(Debug, Clone)]
pub struct AttachmentList {
    pub attachments: Vec<AttachmentSummary>,
    pub total: i64,
}

/// One attachment summary for the reader's attachment list.
#[derive(Debug, Clone)]
pub struct AttachmentSummary {
    pub id: i64,
    pub part_id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub content_id: Option<String>,
    pub blob_path: Option<String>,
}

/// Returns one message's header fields by id and account.
pub async fn get_message(account_id: i64, message_id: i64) -> Result<MessageResult, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .get_message(account_id, message_id)
        .await
        .map_err(DaemonError::from)?;
    parse_message_detail(&reply)
}

/// Returns one message's decoded body by id.
pub async fn get_message_body(message_id: i64) -> Result<MessageBody, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .get_message_body(message_id)
        .await
        .map_err(DaemonError::from)?;
    parse_message_body(&reply)
}

/// Returns one message's attachments by id.
pub async fn get_attachments(message_id: i64) -> Result<AttachmentList, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .get_attachments(message_id)
        .await
        .map_err(DaemonError::from)?;
    parse_attachments(&reply)
}

/// Marks a message as read or unread in all its mailbox locations.
pub async fn mark_read(account_id: i64, message_id: i64, seen: bool) -> Result<(), DaemonError> {
    let mut client = connect().await?;
    client
        .mark_read(account_id, message_id, seen)
        .await
        .map_err(DaemonError::from)?;
    Ok(())
}

/// Opens a client to the daemon for `data_dir`, spawning one on demand.
async fn connect() -> Result<Client, DaemonError> {
    Client::connect_or_spawn(&paths::data_dir())
        .await
        .map_err(DaemonError::from)
}

/// Parses the message detail reply into typed results.
fn parse_message_detail(reply: &Value) -> Result<MessageResult, DaemonError> {
    Ok(MessageResult {
        message: MessageDetail {
            id: field_i64(reply, "id")?,
            account_id: field_i64(reply, "account_id")?,
            subject: reply
                .get("subject")
                .and_then(Value::as_str)
                .map(str::to_string),
            from_json: reply
                .get("from_json")
                .and_then(Value::as_str)
                .map(str::to_string),
            to_json: reply
                .get("to_json")
                .and_then(Value::as_str)
                .map(str::to_string),
            date_utc: reply.get("date_utc").and_then(Value::as_i64),
            snippet: reply
                .get("snippet")
                .and_then(Value::as_str)
                .map(str::to_string),
            has_attachments: reply
                .get("has_attachments")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            body_state: field_str(reply, "body_state")?,
        },
    })
}

/// Parses the message body reply into typed results.
fn parse_message_body(reply: &Value) -> Result<MessageBody, DaemonError> {
    Ok(MessageBody {
        message_id: field_i64(reply, "message_id")?,
        text_body: reply
            .get("text_body")
            .and_then(Value::as_str)
            .map(str::to_string),
        html_clean: reply
            .get("html_clean")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Parses the attachments reply into typed results.
fn parse_attachments(reply: &Value) -> Result<AttachmentList, DaemonError> {
    let attachments = reply
        .get("attachments")
        .and_then(Value::as_array)
        .ok_or_else(|| internal("get_attachments reply carries no attachments array"))?;
    let mut items = Vec::new();
    for row in attachments {
        items.push(AttachmentSummary {
            id: field_i64(row, "id")?,
            part_id: field_str(row, "part_id")?,
            filename: row
                .get("filename")
                .and_then(Value::as_str)
                .map(str::to_string),
            mime_type: row
                .get("mime_type")
                .and_then(Value::as_str)
                .map(str::to_string),
            size_bytes: row.get("size_bytes").and_then(Value::as_i64),
            content_id: row
                .get("content_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            blob_path: row
                .get("blob_path")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }
    let total = field_i64(reply, "total")?;
    Ok(AttachmentList {
        attachments: items,
        total,
    })
}

/// Pulls an integer field out of a reply object.
fn field_i64(row: &Value, key: &str) -> Result<i64, DaemonError> {
    row.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| internal(format!("reader reply field {key} is malformed")))
}

/// Pulls a string field out of a reply object.
fn field_str(row: &Value, key: &str) -> Result<String, DaemonError> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| internal(format!("reader reply field {key} is malformed")))
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
        /// `get_message` replies without required fields.
        MalformedMessage,
    }

    /// The canned result payload for one request.
    fn canned(method: &str) -> Value {
        match method {
            methods::GET_MESSAGE => json!({
                "id": 1,
                "account_id": 1,
                "subject": "Reader subject",
                "from_json": "[{\"name\":\"Alice\",\"address\":\"alice@example.com\"}]",
                "to_json": "[{\"name\":\"Bob\",\"address\":\"bob@example.com\"}]",
                "date_utc": 1000,
                "snippet": "reader snippet",
                "has_attachments": false,
                "body_state": "full",
            }),
            methods::GET_MESSAGE_BODY => json!({
                "message_id": 1,
                "text_body": "Plain text body",
                "html_clean": "<b>HTML body</b>",
            }),
            methods::GET_ATTACHMENTS => json!({
                "attachments": [
                    {
                        "id": 1,
                        "part_id": "part-1",
                        "filename": "report.pdf",
                        "mime_type": "application/pdf",
                        "size_bytes": 1024,
                        "content_id": "<cid-1>",
                        "blob_path": "ab/cd1234",
                    },
                ],
                "total": 1,
            }),
            methods::MARK_READ => json!({ "ok": true, "updated": 1 }),
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
                        Behavior::MalformedMessage if request.method == methods::GET_MESSAGE => {
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
    async fn get_message_returns_detail() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let result = get_message(1, 1).await.expect("get message");

        assert_eq!(result.message.id, 1);
        assert_eq!(result.message.account_id, 1);
        assert_eq!(result.message.subject.as_deref(), Some("Reader subject"));
        assert_eq!(result.message.body_state, "full");

        let params = recorded_request(&recorded, methods::GET_MESSAGE);
        assert_eq!(params["account_id"], json!(1));
        assert_eq!(params["message_id"], json!(1));

        daemon.restore();
    }

    #[tokio::test]
    async fn get_message_maps_malformed_replies_to_internal() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::MalformedMessage).await;

        let error = get_message(1, 1)
            .await
            .expect_err("malformed reply must fail");
        assert_eq!(error.kind, "internal");
        assert!(error.message.contains("id"));

        daemon.restore();
    }

    #[tokio::test]
    async fn get_message_body_returns_body() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let body = get_message_body(1).await.expect("get message body");

        assert_eq!(body.message_id, 1);
        assert_eq!(body.text_body.as_deref(), Some("Plain text body"));
        assert_eq!(body.html_clean.as_deref(), Some("<b>HTML body</b>"));

        let params = recorded_request(&recorded, methods::GET_MESSAGE_BODY);
        assert_eq!(params["message_id"], json!(1));

        daemon.restore();
    }

    #[tokio::test]
    async fn get_attachments_returns_attachments() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let list = get_attachments(1).await.expect("get attachments");

        assert_eq!(list.attachments.len(), 1);
        assert_eq!(list.attachments[0].id, 1);
        assert_eq!(list.attachments[0].filename.as_deref(), Some("report.pdf"));
        assert_eq!(list.total, 1);

        let params = recorded_request(&recorded, methods::GET_ATTACHMENTS);
        assert_eq!(params["message_id"], json!(1));

        daemon.restore();
    }

    #[tokio::test]
    async fn mark_read_marks_as_read() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        mark_read(1, 1, true).await.expect("mark read");

        let params = recorded_request(&recorded, methods::MARK_READ);
        assert_eq!(params["account_id"], json!(1));
        assert_eq!(params["message_id"], json!(1));
        assert_eq!(params["seen"], json!(true));

        daemon.restore();
    }
}
