//! Accounts API: wizard and settings operations for the GUI.
//!
//! Every call opens a fresh IPC connection (the client is single-shot
//! after failure), asks the daemon to run the operation, and maps the
//! wire reply onto typed values. The daemon owns all persistence; the
//! GUI never touches the database or the keyring (ARCHITECTURE §1).

use mailslate_core::infra::{ipc::Client, paths};
use serde_json::{json, Value};

use super::system::DaemonError;

/// One configured account, as `get_accounts` lists it.
#[derive(Debug, Clone)]
pub struct AccountSummary {
    /// Row id of the account.
    pub id: i64,
    /// Human-friendly name shown in the sidebar.
    pub display_name: String,
    /// The account's email address.
    pub email: String,
    /// Provider family: `gmail`, `outlook`, or `imap`.
    pub provider: String,
    /// How the account authenticates: `password` or `oauth2`.
    pub auth_type: String,
    /// Accent color, when one was chosen.
    pub color: Option<String>,
    /// How many days of mail the initial sync keeps.
    pub sync_window_days: i64,
}

/// Connection details for a generic IMAP/SMTP account
/// (the wizard's "Other IMAP" path).
#[derive(Debug, Clone)]
pub struct ImapDraft {
    /// Optional display name; the email address is used when absent.
    pub display_name: Option<String>,
    /// The account's email address (also the IMAP username).
    pub email: String,
    /// IMAP server host.
    pub imap_host: String,
    /// IMAP server port.
    pub imap_port: u16,
    /// IMAP transport security: `tls`, `starttls`, or `plaintext`.
    pub imap_security: String,
    /// SMTP server host.
    pub smtp_host: String,
    /// SMTP server port.
    pub smtp_port: u16,
    /// SMTP transport security: `tls`, `starttls`, or `plaintext`.
    pub smtp_security: String,
}

/// Mutable account fields for [`update_account`].
///
/// The color is tri-state: [`Self::clear_color`] removes it, a
/// `Some` color sets it, and neither leaves it untouched.
#[derive(Debug, Clone)]
pub struct AccountPatch {
    /// New display name, when changing it.
    pub display_name: Option<String>,
    /// New accent color, when setting one.
    pub color: Option<String>,
    /// Remove the accent color (wins over [`Self::color`]).
    pub clear_color: bool,
    /// New sync window in days, when changing it.
    pub sync_window_days: Option<i64>,
}

/// One leg of a connection probe, as the wizard's progress row shows.
#[derive(Debug, Clone)]
pub struct ConnectionStep {
    /// Whether the step succeeded.
    pub ok: bool,
    /// Why the step failed, when it did.
    pub error: Option<String>,
}

/// Result of [`test_connection`]: the IMAP login probe and the SMTP
/// reachability probe, both always present.
#[derive(Debug, Clone)]
pub struct ConnectionTest {
    /// The IMAP connect-and-login probe.
    pub imap: ConnectionStep,
    /// The SMTP reachability probe.
    pub smtp: ConnectionStep,
}

/// Lists every configured account.
pub async fn list_accounts() -> Result<Vec<AccountSummary>, DaemonError> {
    let mut client = connect().await?;
    let reply = client.get_accounts().await.map_err(DaemonError::from)?;
    parse_accounts(&reply)
}

/// Adds a generic IMAP/SMTP account; replies with the new row id.
///
/// The password crosses in-memory and lands in the keyring inside the
/// daemon; the bridge never persists it.
pub async fn add_imap_account(draft: ImapDraft, password: String) -> Result<i64, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .add_imap_account(wire_conn(&draft), json!({ "password": password }))
        .await
        .map_err(DaemonError::from)?;
    account_id(&reply, "add_imap_account")
}

/// Starts an OAuth flow for `provider` (`google` or `microsoft`);
/// replies with the URL to open in the system browser.
pub async fn start_oauth(provider: String) -> Result<String, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .start_oauth(&provider)
        .await
        .map_err(DaemonError::from)?;
    reply
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| internal("start_oauth reply carries no url"))
}

/// Starts a reconnect flow for an existing OAuth account, so an
/// expired or revoked grant is refreshed in place; replies with the
/// consent URL to open in the system browser.
pub async fn start_reauth(account_id: i64) -> Result<String, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .start_reauth(account_id)
        .await
        .map_err(DaemonError::from)?;
    reply
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| internal("start_reauth reply carries no url"))
}

/// Names and files the account whose OAuth handshake just finished;
/// replies with the new row id.
pub async fn complete_oauth_account(
    display_name: String,
    email: String,
    color: Option<String>,
) -> Result<i64, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .complete_oauth_account(&display_name, &email, color.as_deref())
        .await
        .map_err(DaemonError::from)?;
    account_id(&reply, "complete_oauth_account")
}

/// Patches mutable account fields (name, color, sync window).
pub async fn update_account(account_id: i64, patch: AccountPatch) -> Result<(), DaemonError> {
    let mut client = connect().await?;
    client
        .update_account(account_id, wire_patch(&patch))
        .await
        .map_err(DaemonError::from)?;
    Ok(())
}

/// Removes an account; the daemon scrubs its keyring secrets and row.
pub async fn remove_account(account_id: i64) -> Result<(), DaemonError> {
    let mut client = connect().await?;
    client
        .remove_account(account_id)
        .await
        .map_err(DaemonError::from)?;
    Ok(())
}

/// Probes a would-be IMAP/SMTP account before the wizard saves it:
/// the daemon attempts the real IMAP login and an SMTP reachability
/// check against the draft's servers (USER_FLOW §3).
///
/// Both steps always come back — one failing does not hide the other.
pub async fn test_connection(
    draft: ImapDraft,
    password: String,
) -> Result<ConnectionTest, DaemonError> {
    let mut client = connect().await?;
    let reply = client
        .test_connection(wire_conn(&draft), json!({ "password": password }))
        .await
        .map_err(DaemonError::from)?;
    parse_connection_test(&reply)
}

/// Opens a client to the daemon for `data_dir`, spawning one on demand
/// so the GUI works even when nothing autostarted it at login.
async fn connect() -> Result<Client, DaemonError> {
    Client::connect_or_spawn(&paths::data_dir())
        .await
        .map_err(DaemonError::from)
}

/// The `conn` payload of `add_imap_account`, matching the daemon's
/// field names exactly.
fn wire_conn(draft: &ImapDraft) -> Value {
    json!({
        "display_name": draft.display_name,
        "email": draft.email,
        "imap_host": draft.imap_host,
        "imap_port": draft.imap_port,
        "security": draft.imap_security,
        "smtp_host": draft.smtp_host,
        "smtp_port": draft.smtp_port,
        "smtp_security": draft.smtp_security,
    })
}

/// The `patch` payload of `update_account`, encoding the color
/// tri-state: `clear_color` wins as an explicit null, a color is sent
/// as a string, and neither leaves the key absent (untouched).
fn wire_patch(patch: &AccountPatch) -> Value {
    let mut wire = serde_json::Map::new();
    if let Some(name) = &patch.display_name {
        wire.insert("display_name".into(), json!(name));
    }
    if patch.clear_color {
        wire.insert("color".into(), Value::Null);
    } else if let Some(color) = &patch.color {
        wire.insert("color".into(), json!(color));
    }
    if let Some(days) = patch.sync_window_days {
        wire.insert("sync_window_days".into(), json!(days));
    }
    Value::Object(wire)
}

/// Parses the `get_accounts` reply into typed rows; a malformed reply
/// is an internal error, never a panic.
fn parse_accounts(reply: &Value) -> Result<Vec<AccountSummary>, DaemonError> {
    let rows = reply
        .get("accounts")
        .and_then(Value::as_array)
        .ok_or_else(|| internal("get_accounts reply carries no accounts array"))?;
    rows.iter()
        .map(|row| {
            Ok(AccountSummary {
                id: field_i64(row, "id")?,
                display_name: field_str(row, "display_name")?,
                email: field_str(row, "email")?,
                provider: field_str(row, "provider")?,
                auth_type: field_str(row, "auth_type")?,
                color: row.get("color").and_then(Value::as_str).map(str::to_string),
                sync_window_days: field_i64(row, "sync_window_days")?,
            })
        })
        .collect()
}

/// Pulls a string field out of an account row.
fn field_str(row: &Value, key: &str) -> Result<String, DaemonError> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| internal(format!("account row field {key} is malformed")))
}

/// Pulls an integer field out of an account row.
fn field_i64(row: &Value, key: &str) -> Result<i64, DaemonError> {
    row.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| internal(format!("account row field {key} is malformed")))
}

/// Pulls `account_id` out of a creation reply.
fn account_id(reply: &Value, source: &str) -> Result<i64, DaemonError> {
    reply
        .get("account_id")
        .and_then(Value::as_i64)
        .ok_or_else(|| internal(format!("{source} reply carries no account_id")))
}

/// Parses the `test_connection` reply; a malformed reply is an
/// internal error, never a panic.
fn parse_connection_test(reply: &Value) -> Result<ConnectionTest, DaemonError> {
    Ok(ConnectionTest {
        imap: parse_connection_step(reply, "imap")?,
        smtp: parse_connection_step(reply, "smtp")?,
    })
}

/// Parses one `{ ok, error? }` step object out of the reply.
fn parse_connection_step(reply: &Value, step: &str) -> Result<ConnectionStep, DaemonError> {
    let object = reply
        .get(step)
        .ok_or_else(|| internal(format!("test_connection reply carries no {step} step")))?;
    let ok = object
        .get("ok")
        .and_then(Value::as_bool)
        .ok_or_else(|| internal(format!("test_connection {step} step carries no ok flag")))?;
    Ok(ConnectionStep {
        ok,
        error: object
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
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
    use mailslate_protocol::{methods, BridgeError, ErrorKind, PROTOCOL_VERSION};
    use std::sync::{Arc, Mutex};
    use tokio::net::UnixListener;

    /// Recorded `(method, params)` pairs from every served request.
    type Recorded = Arc<Mutex<Vec<(String, Value)>>>;

    /// Per-fake-daemon reply behavior.
    #[derive(Clone, Copy, PartialEq)]
    enum Behavior {
        /// Canned happy-path replies.
        Normal,
        /// `get_accounts` replies without an `accounts` array.
        MalformedAccounts,
        /// `test_connection` replies without the step objects.
        MalformedConnection,
        /// `test_connection` reports the IMAP step as failed.
        ImapFails,
        /// `start_reauth` replies without a `url`.
        MalformedReauth,
    }

    /// The canned result payload for one request.
    fn canned(method: &str) -> Value {
        match method {
            methods::PING => json!({
                "protocol_version": PROTOCOL_VERSION,
                "daemon_version": "9.9.9",
            }),
            methods::GET_ACCOUNTS => json!({ "accounts": [
                {
                    "id": 1,
                    "display_name": "Work",
                    "email": "work@example.com",
                    "provider": "imap",
                    "auth_type": "password",
                    "color": null,
                    "sync_window_days": 30,
                },
                {
                    "id": 2,
                    "display_name": "Gmail",
                    "email": "me@gmail.com",
                    "provider": "gmail",
                    "auth_type": "oauth2",
                    "color": "blue",
                    "sync_window_days": 90,
                },
            ]}),
            methods::ADD_IMAP_ACCOUNT => json!({ "account_id": 7 }),
            methods::TEST_CONNECTION => json!({
                "imap": { "ok": true },
                "smtp": { "ok": true },
            }),
            methods::START_OAUTH => json!({ "url": "https://accounts.example.test/auth" }),
            methods::START_REAUTH => json!({ "url": "https://accounts.example.test/reauth" }),
            methods::COMPLETE_OAUTH_ACCOUNT => json!({ "account_id": 11 }),
            methods::UPDATE_ACCOUNT | methods::REMOVE_ACCOUNT => json!({ "ok": true }),
            _ => json!({}),
        }
    }

    /// Serves requests until the listener errors: canned replies by
    /// method, `add_imap_account` with password `reject-me` fails as a
    /// validation error, and every request is recorded.
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
                        Behavior::MalformedAccounts if request.method == methods::GET_ACCOUNTS => {
                            json!({ "nope": true })
                        }
                        Behavior::MalformedConnection
                            if request.method == methods::TEST_CONNECTION =>
                        {
                            json!({ "nope": true })
                        }
                        Behavior::ImapFails if request.method == methods::TEST_CONNECTION => {
                            json!({
                                "imap": { "ok": false, "error": "auth failed (rejected credentials)" },
                                "smtp": { "ok": true },
                            })
                        }
                        Behavior::MalformedReauth if request.method == methods::START_REAUTH => {
                            json!({ "nope": true })
                        }
                        _ => canned(&request.method),
                    };
                    let response = if (request.method == methods::ADD_IMAP_ACCOUNT
                        || request.method == methods::TEST_CONNECTION)
                        && request.params["creds"]["password"] == json!("reject-me")
                    {
                        Response::err(
                            request.id,
                            BridgeError::new(ErrorKind::Validation, "bad account"),
                        )
                    } else {
                        Response::ok(request.id, result)
                    };
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

    /// A generic draft covering every field.
    fn draft() -> ImapDraft {
        ImapDraft {
            display_name: None,
            email: "work@example.com".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_security: "tls".to_string(),
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_security: "tls".to_string(),
        }
    }

    /// The single recorded request for `method` (tests make exactly one
    /// call of each kind).
    fn recorded_request(recorded: &Recorded, method: &str) -> Value {
        let requests = recorded.lock().expect("record lock");
        requests
            .iter()
            .find(|(name, _)| name == method)
            .map(|(_, params)| params.clone())
            .unwrap_or_else(|| panic!("no {method} request was recorded"))
    }

    #[tokio::test]
    async fn list_accounts_parses_rows() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let accounts = list_accounts().await.expect("list accounts");

        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].id, 1);
        assert_eq!(accounts[0].display_name, "Work");
        assert_eq!(accounts[0].email, "work@example.com");
        assert_eq!(accounts[0].provider, "imap");
        assert_eq!(accounts[0].auth_type, "password");
        assert_eq!(accounts[0].color, None);
        assert_eq!(accounts[0].sync_window_days, 30);
        assert_eq!(accounts[1].color.as_deref(), Some("blue"));
        assert_eq!(accounts[1].sync_window_days, 90);

        daemon.restore();
    }

    #[tokio::test]
    async fn list_accounts_maps_malformed_replies_to_internal() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::MalformedAccounts).await;

        let error = list_accounts()
            .await
            .expect_err("malformed reply must fail");
        assert_eq!(error.kind, "internal");
        assert!(error.message.contains("accounts"));

        daemon.restore();
    }

    #[tokio::test]
    async fn add_imap_account_sends_conn_and_creds_and_returns_id() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let id = add_imap_account(draft(), "secret-pw".to_string())
            .await
            .expect("add account");

        assert_eq!(id, 7);
        let params = recorded_request(&recorded, methods::ADD_IMAP_ACCOUNT);
        assert_eq!(params["conn"]["email"], json!("work@example.com"));
        assert_eq!(params["conn"]["display_name"], Value::Null);
        assert_eq!(params["conn"]["imap_host"], json!("imap.example.com"));
        assert_eq!(params["conn"]["imap_port"], json!(993));
        assert_eq!(params["conn"]["security"], json!("tls"));
        assert_eq!(params["conn"]["smtp_host"], json!("smtp.example.com"));
        assert_eq!(params["conn"]["smtp_port"], json!(465));
        assert_eq!(params["conn"]["smtp_security"], json!("tls"));
        assert_eq!(params["creds"]["password"], json!("secret-pw"));

        daemon.restore();
    }

    #[tokio::test]
    async fn add_imap_account_maps_daemon_errors() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let error = add_imap_account(draft(), "reject-me".to_string())
            .await
            .expect_err("rejected password must fail");
        assert_eq!(error.kind, "validation");
        assert_eq!(error.message, "bad account");
        assert!(!error.retryable);

        daemon.restore();
    }

    #[tokio::test]
    async fn start_oauth_returns_the_authorize_url() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let url = start_oauth("google".to_string())
            .await
            .expect("start oauth");

        assert_eq!(url, "https://accounts.example.test/auth");
        let params = recorded_request(&recorded, methods::START_OAUTH);
        assert_eq!(params["provider"], json!("google"));

        daemon.restore();
    }

    #[tokio::test]
    async fn start_reauth_returns_the_reconnect_url() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let url = start_reauth(7).await.expect("start reauth");

        assert_eq!(url, "https://accounts.example.test/reauth");
        let params = recorded_request(&recorded, methods::START_REAUTH);
        assert_eq!(params["account_id"], json!(7));

        daemon.restore();
    }

    #[tokio::test]
    async fn start_reauth_without_a_url_is_internal() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::MalformedReauth).await;

        let error = start_reauth(7)
            .await
            .expect_err("malformed reply must fail");

        assert_eq!(error.kind, "internal");
        assert_eq!(error.message, "start_reauth reply carries no url");

        daemon.restore();
    }

    #[tokio::test]
    async fn complete_oauth_account_sends_fields_and_returns_id() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let id = complete_oauth_account(
            "Gmail".to_string(),
            "me@gmail.com".to_string(),
            Some("blue".to_string()),
        )
        .await
        .expect("complete oauth");

        assert_eq!(id, 11);
        let params = recorded_request(&recorded, methods::COMPLETE_OAUTH_ACCOUNT);
        assert_eq!(params["display_name"], json!("Gmail"));
        assert_eq!(params["email"], json!("me@gmail.com"));
        assert_eq!(params["color"], json!("blue"));

        daemon.restore();
    }

    #[tokio::test]
    async fn complete_oauth_account_without_color_sends_null() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        complete_oauth_account("Gmail".to_string(), "me@gmail.com".to_string(), None)
            .await
            .expect("complete oauth");

        let params = recorded_request(&recorded, methods::COMPLETE_OAUTH_ACCOUNT);
        assert_eq!(params["color"], Value::Null);

        daemon.restore();
    }

    #[tokio::test]
    async fn update_account_encodes_the_color_tri_state() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        update_account(
            3,
            AccountPatch {
                display_name: Some("Renamed".to_string()),
                color: None,
                clear_color: true,
                sync_window_days: Some(90),
            },
        )
        .await
        .expect("update clears the color");
        update_account(
            3,
            AccountPatch {
                display_name: None,
                color: Some("green".to_string()),
                clear_color: false,
                sync_window_days: None,
            },
        )
        .await
        .expect("update sets the color");
        update_account(
            3,
            AccountPatch {
                display_name: None,
                color: None,
                clear_color: false,
                sync_window_days: None,
            },
        )
        .await
        .expect("update touches nothing");

        let updates: Vec<Value> = {
            let requests = recorded.lock().expect("record lock");
            requests
                .iter()
                .filter(|(name, _)| name == methods::UPDATE_ACCOUNT)
                .map(|(_, params)| params.clone())
                .collect()
        };
        assert_eq!(updates.len(), 3);
        assert_eq!(updates[0]["patch"]["color"], Value::Null);
        assert_eq!(updates[0]["patch"]["display_name"], json!("Renamed"));
        assert_eq!(updates[0]["patch"]["sync_window_days"], json!(90));
        assert_eq!(updates[0]["account_id"], json!(3));
        assert_eq!(updates[1]["patch"]["color"], json!("green"));
        assert!(updates[1]["patch"].get("display_name").is_none());
        assert!(updates[2]["patch"].get("color").is_none());

        daemon.restore();
    }

    #[tokio::test]
    async fn remove_account_sends_the_id_and_succeeds() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        remove_account(9).await.expect("remove account");

        let params = recorded_request(&recorded, methods::REMOVE_ACCOUNT);
        assert_eq!(params["account_id"], json!(9));

        daemon.restore();
    }

    #[test]
    fn wire_conn_matches_the_daemon_field_names() {
        let value = wire_conn(&draft());
        assert_eq!(value["email"], json!("work@example.com"));
        assert_eq!(value["imap_port"], json!(993));
        assert_eq!(value["smtp_port"], json!(465));
        assert_eq!(value["security"], json!("tls"));
        assert!(value.get("imap_security").is_none());
    }

    #[tokio::test]
    async fn test_connection_sends_conn_and_creds_and_parses_both_steps() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let test = test_connection(draft(), "secret-pw".to_string())
            .await
            .expect("test connection");

        assert!(test.imap.ok);
        assert_eq!(test.imap.error, None);
        assert!(test.smtp.ok);
        assert_eq!(test.smtp.error, None);
        let params = recorded_request(&recorded, methods::TEST_CONNECTION);
        assert_eq!(params["conn"]["email"], json!("work@example.com"));
        assert_eq!(params["conn"]["imap_host"], json!("imap.example.com"));
        assert_eq!(params["conn"]["security"], json!("tls"));
        assert_eq!(params["conn"]["smtp_port"], json!(465));
        assert_eq!(params["creds"]["password"], json!("secret-pw"));

        daemon.restore();
    }

    #[tokio::test]
    async fn test_connection_parses_a_failed_step_with_its_error() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::ImapFails).await;

        let test = test_connection(draft(), "secret-pw".to_string())
            .await
            .expect("test connection parses a failed step");

        assert!(!test.imap.ok);
        assert_eq!(
            test.imap.error.as_deref(),
            Some("auth failed (rejected credentials)")
        );
        assert!(test.smtp.ok);
        assert_eq!(test.smtp.error, None);

        daemon.restore();
    }

    #[tokio::test]
    async fn test_connection_maps_daemon_errors() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::Normal).await;

        let error = test_connection(draft(), "reject-me".to_string())
            .await
            .expect_err("rejected password must fail");
        assert_eq!(error.kind, "validation");
        assert_eq!(error.message, "bad account");
        assert!(!error.retryable);

        daemon.restore();
    }

    #[tokio::test]
    async fn test_connection_maps_malformed_replies_to_internal() {
        let _guard = ENV_LOCK.lock().await;
        let recorded = Recorded::default();
        let daemon = fake_daemon(&recorded, Behavior::MalformedConnection).await;

        let error = test_connection(draft(), "secret-pw".to_string())
            .await
            .expect_err("malformed reply must fail");
        assert_eq!(error.kind, "internal");

        daemon.restore();
    }
}
