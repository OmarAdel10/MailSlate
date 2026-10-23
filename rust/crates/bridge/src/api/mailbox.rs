//! Mailbox API: conversation mailbox listing for the GUI (PRD F3).
//!
//! Every call opens a fresh IPC connection, asks the daemon to run the
//! operation, and maps the wire reply onto typed values. The daemon owns
//! the mailboxes table; the GUI never touches the database.

use mailslate_core::infra::{ipc::Client, paths};
use serde_json::Value;

use super::system::DaemonError;

/// One mailbox, as listed in the mailbox sidebar.
#[derive(Debug, Clone)]
pub struct MailboxSummary {
    /// Row id of the mailbox.
    pub id: i64,
    /// Account the mailbox belongs to.
    pub account_id: i64,
    /// IMAP mailbox name (e.g. "INBOX", "Sent").
    pub name: String,
    /// Canonical kind: `inbox`, `sent`, `drafts`, `trash`, `spam`,
    /// `archive`, `all`, `important`, or `user`.
    pub kind: String,
    /// Server-reported special-use attribute, when any.
    pub special_use: Option<String>,
    /// Accent color, when one was chosen.
    pub color: Option<String>,
    /// Display sort order (lower sorts first).
    pub sort_order: i64,
    /// UIDVALIDITY of the last sync.
    pub uidvalidity: Option<i64>,
    /// UIDNEXT of the last sync.
    pub uidnext: Option<i64>,
    /// Highest MODSEQ seen.
    pub highest_modseq: Option<i64>,
    /// Unix epoch of the last completed sync.
    pub last_synced_at: Option<i64>,
}

/// Result of [`list_mailboxes`]: the matched mailboxes and total count.
#[derive(Debug, Clone)]
pub struct MailboxList {
    /// Matched mailboxes, sorted by sort_order.
    pub mailboxes: Vec<MailboxSummary>,
    /// Total mailboxes across all pages (not just this page).
    pub total: i64,
}

/// Lists `account_id`'s mailboxes, with optional keyset cursor
/// (`last_sort_order` + `last_kind` + `last_name`) for pagination.
pub async fn list_mailboxes(
    account_id: i64,
    limit: u32,
    last_sort_order: Option<i64>,
    last_kind: Option<String>,
    last_name: Option<String>,
) -> Result<MailboxList, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .get_mailboxes(
            account_id,
            limit,
            last_sort_order,
            last_kind.as_deref(),
            last_name.as_deref(),
        )
        .await
        .map_err(DaemonError::from)?;
    parse_mailbox_list(&reply)
}

/// Opens a client to the daemon for `data_dir`, spawning one on demand.
async fn connect() -> Result<Client, DaemonError> {
    Client::connect_or_spawn(&paths::data_dir())
        .await
        .map_err(DaemonError::from)
}

/// Parses the mailbox list reply into typed results; a malformed reply
/// is an internal error, never a panic.
fn parse_mailbox_list(reply: &Value) -> Result<MailboxList, DaemonError> {
    let mailboxes = reply
        .get("mailboxes")
        .and_then(Value::as_array)
        .ok_or_else(|| internal("get_mailboxes reply carries no mailboxes array"))?;
    let mut items = Vec::new();
    for row in mailboxes {
        items.push(MailboxSummary {
            id: field_i64(row, "id")?,
            account_id: field_i64(row, "account_id")?,
            name: field_str(row, "name")?,
            kind: field_str(row, "kind")?,
            special_use: row
                .get("special_use")
                .and_then(Value::as_str)
                .map(str::to_string),
            color: row.get("color").and_then(Value::as_str).map(str::to_string),
            sort_order: field_i64(row, "sort_order")?,
            uidvalidity: row.get("uidvalidity").and_then(Value::as_i64),
            uidnext: row.get("uidnext").and_then(Value::as_i64),
            highest_modseq: row.get("highest_modseq").and_then(Value::as_i64),
            last_synced_at: row.get("last_synced_at").and_then(Value::as_i64),
        });
    }
    let total = field_i64(reply, "total")?;
    Ok(MailboxList {
        mailboxes: items,
        total,
    })
}

/// Pulls a string field out of a mailbox row.
fn field_str(row: &Value, key: &str) -> Result<String, DaemonError> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| internal(format!("mailbox row field {key} is malformed")))
}

/// Pulls an integer field out of a mailbox row.
fn field_i64(row: &Value, key: &str) -> Result<i64, DaemonError> {
    row.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| internal(format!("mailbox row field {key} is malformed")))
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
        /// `get_mailboxes` replies without a `mailboxes` array.
        MalformedMailboxes,
    }

    /// The canned result payload for one request.
    fn canned(method: &str) -> Value {
        match method {
            methods::GET_MAILBOXES => json!({
                "mailboxes": [
                    {
                        "id": 1,
                        "account_id": 1,
                        "name": "INBOX",
                        "kind": "inbox",
                        "special_use": null,
                        "color": null,
                        "sort_order": 0,
                        "uidvalidity": null,
                        "uidnext": null,
                        "highest_modseq": null,
                        "last_synced_at": null,
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
                        Behavior::MalformedMailboxes
                            if request.method == methods::GET_MAILBOXES =>
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
    async fn list_mailboxes_returns_mailboxes_and_total() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let result = list_mailboxes(1, 50, None, None, None)
            .await
            .expect("list mailboxes");

        assert_eq!(result.mailboxes.len(), 1);
        assert_eq!(result.mailboxes[0].id, 1);
        assert_eq!(result.mailboxes[0].account_id, 1);
        assert_eq!(result.mailboxes[0].name, "INBOX");
        assert_eq!(result.mailboxes[0].kind, "inbox");
        assert_eq!(result.total, 1);

        let params = recorded_request(&recorded, methods::GET_MAILBOXES);
        assert_eq!(params["account_id"], json!(1));
        assert_eq!(params["limit"], json!(50));

        daemon.restore();
    }

    #[tokio::test]
    async fn list_mailboxes_maps_malformed_replies_to_internal() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::MalformedMailboxes).await;

        let error = list_mailboxes(1, 50, None, None, None)
            .await
            .expect_err("malformed reply must fail");
        assert_eq!(error.kind, "internal");
        assert!(error.message.contains("mailboxes"));

        daemon.restore();
    }
}
