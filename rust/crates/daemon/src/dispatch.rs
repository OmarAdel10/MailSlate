//! Request dispatch: the daemon side of the method table.
//!
//! Only `ping` and `shutdown` exist in M1; the mail methods land with their
//! features (ARCHITECTURE.md §5.3).

use mailslate_core::domain::accounts::{
    AccountPatch, AuthKind, ImapAccountDraft, Security, add_imap_account, get_accounts,
    remove_account, update_account,
};
use mailslate_core::infra::db::pool::ReadPool;
use mailslate_core::infra::db::writer::WriterActor;
use mailslate_core::infra::keyring::KeyStore;
use mailslate_protocol::{BridgeError, ErrorKind, PROTOCOL_VERSION, PingResult, methods};
use serde_json::{Value, json};
use std::sync::Arc;

/// What the connection handler should do after replying.
#[derive(Debug)]
pub enum Dispatched {
    /// Reply and keep serving.
    Reply(Value),
    /// Reply, then shut the daemon down.
    Stop(Value),
}

/// State shared by every dispatched call: the single-writer handle,
/// the keyring-backed secret store, and the read pool.
pub struct DispatchCtx {
    /// The only database writer (batched transactions).
    pub writer: Arc<WriterActor>,
    /// Secrets live here; the daemon is the sole keyring user.
    pub keystore: Arc<dyn KeyStore>,
    /// Readonly connections for read-only methods.
    pub reader: Arc<ReadPool>,
}

/// Dispatches one method call against shared daemon state.
///
/// Unknown methods are a `not_found` error, never a panic.
pub fn dispatch(
    ctx: &DispatchCtx,
    method: &str,
    params: &Value,
) -> Result<Dispatched, BridgeError> {
    match method {
        methods::PING => Ok(Dispatched::Reply(ping_result())),
        methods::SHUTDOWN => Ok(Dispatched::Stop(json!({ "ok": true }))),
        methods::ADD_IMAP_ACCOUNT => add_imap_account_dispatch(ctx, params).map(Dispatched::Reply),
        methods::REMOVE_ACCOUNT => remove_account_dispatch(ctx, params).map(Dispatched::Reply),
        methods::UPDATE_ACCOUNT => update_account_dispatch(ctx, params).map(Dispatched::Reply),
        methods::GET_ACCOUNTS => get_accounts_dispatch(ctx).map(Dispatched::Reply),
        other => Err(BridgeError::new(
            ErrorKind::NotFound,
            format!("unknown method: {other}"),
        )),
    }
}

/// Parses and applies `add_imap_account`, persisting the row and the
/// secret before replying.
fn add_imap_account_dispatch(ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let conn = required(params, "conn")?;
    let password = string(required(params, "creds")?, "password")?;
    let email = string(conn, "email")?;
    let draft = ImapAccountDraft {
        display_name: optional(conn, "display_name")?.unwrap_or(email.clone()),
        email,
        imap_host: string(conn, "imap_host")?,
        imap_port: port(conn, "imap_port")?,
        imap_security: security(&string(conn, "security")?)?,
        smtp_host: string(conn, "smtp_host")?,
        smtp_port: port(conn, "smtp_port")?,
        smtp_security: security(&string(conn, "smtp_security")?)?,
        auth: AuthKind::Password,
    };
    let account_id = add_imap_account(ctx.keystore.as_ref(), &ctx.writer, &draft, &password)
        .map_err(account_error)?;
    Ok(json!({ "account_id": account_id }))
}

/// Parses and applies `remove_account`, scrubbing the keyring first.
fn remove_account_dispatch(ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let account_id = account_id(params)?;
    remove_account(ctx.keystore.as_ref(), &ctx.writer, account_id).map_err(account_error)?;
    Ok(json!({ "ok": true }))
}

/// Parses and applies `update_account` with tri-state patch fields.
fn update_account_dispatch(ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let account_id = account_id(params)?;
    let patch = required(params, "patch")?;
    let color = match patch.get("color") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(color)) => Some(Some(color.clone())),
        Some(_) => return Err(validation("patch.color must be a string or null")),
    };
    let parsed = AccountPatch {
        display_name: patch
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        color,
        sync_window_days: patch.get("sync_window_days").and_then(Value::as_i64),
    };
    update_account(&ctx.writer, account_id, parsed).map_err(account_error)?;
    Ok(json!({ "ok": true }))
}

/// Lists every account with its stored defaults.
fn get_accounts_dispatch(ctx: &DispatchCtx) -> Result<Value, BridgeError> {
    let conn = ctx
        .reader
        .acquire()
        .map_err(|err| BridgeError::new(ErrorKind::Internal, format!("read pool: {err}")))?;
    let accounts =
        get_accounts(&conn).map_err(|err| BridgeError::new(ErrorKind::Storage, err.to_string()))?;
    let accounts: Vec<Value> = accounts
        .iter()
        .map(|account| {
            json!({
                "id": account.id,
                "display_name": account.display_name,
                "email": account.email,
                "provider": account.provider,
                "auth_type": account.auth_type,
                "color": account.color,
                "sync_window_days": account.sync_window_days,
            })
        })
        .collect();
    Ok(json!({ "accounts": accounts }))
}

/// Maps a domain account failure onto the wire error vocabulary.
fn account_error(err: mailslate_core::domain::accounts::AccountError) -> BridgeError {
    BridgeError::new(ErrorKind::Internal, err.to_string())
}

fn validation(message: impl Into<String>) -> BridgeError {
    BridgeError::new(ErrorKind::Validation, message.into())
}

fn required<'a>(params: &'a Value, key: &str) -> Result<&'a Value, BridgeError> {
    params
        .get(key)
        .ok_or_else(|| validation(format!("missing field: {key}")))
}

fn string(params: &Value, key: &str) -> Result<String, BridgeError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| validation(format!("missing string field: {key}")))
}

fn optional(params: &Value, key: &str) -> Result<Option<String>, BridgeError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(validation(format!("{key} must be a string"))),
    }
}

fn port(params: &Value, key: &str) -> Result<u16, BridgeError> {
    let value = params
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| validation(format!("{key} must be a port number")))?;
    Ok(value)
}

fn security(security: &str) -> Result<Security, BridgeError> {
    match security {
        "tls" => Ok(Security::Tls),
        "starttls" => Ok(Security::StartTls),
        "plaintext" => Ok(Security::Plaintext),
        other => Err(validation(format!("unknown security: {other}"))),
    }
}

fn account_id(params: &Value) -> Result<i64, BridgeError> {
    params
        .get("account_id")
        .and_then(Value::as_i64)
        .ok_or_else(|| validation("missing account_id"))
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
    use mailslate_core::infra::db::pool::ReadPool;
    use mailslate_core::infra::db::writer::WriterActor;
    use mailslate_core::infra::db::{DB_FILE_NAME, migrations};
    use mailslate_core::infra::keyring::FakeKeyStore;
    use std::sync::Arc;

    /// A dispatch context over a fresh migrated database in `dir`.
    fn test_ctx(dir: &tempfile::TempDir) -> Arc<DispatchCtx> {
        let db_path = dir.path().join(DB_FILE_NAME);
        let mut conn = mailslate_core::infra::db::open_rw(&db_path).unwrap();
        migrations::run(&mut conn).unwrap();
        drop(conn);
        Arc::new(DispatchCtx {
            writer: Arc::new(WriterActor::spawn(&db_path)),
            keystore: Arc::new(FakeKeyStore::new()),
            reader: Arc::new(ReadPool::open(&db_path).unwrap()),
        })
    }

    /// The add_imap_account request payload for a test connection.
    fn add_params() -> Value {
        json!({
            "conn": {
                "display_name": "Work",
                "email": "work@example.com",
                "imap_host": "imap.example.com",
                "imap_port": 993,
                "security": "tls",
                "smtp_host": "smtp.example.com",
                "smtp_port": 465,
                "smtp_security": "tls"
            },
            "creds": { "password": "secret-pw" }
        })
    }

    #[test]
    fn ping_returns_protocol_and_daemon_versions() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let Dispatched::Reply(value) = dispatch(&ctx, methods::PING, &json!({})).unwrap() else {
            panic!("ping must not stop the daemon");
        };
        assert_eq!(value["protocol_version"], json!(PROTOCOL_VERSION));
        assert_eq!(value["daemon_version"], json!(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn shutdown_replies_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let Dispatched::Stop(value) = dispatch(&ctx, methods::SHUTDOWN, &json!({})).unwrap() else {
            panic!("shutdown must stop the daemon");
        };
        assert_eq!(value, json!({ "ok": true }));
    }

    #[test]
    fn unknown_methods_are_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let err = dispatch(&ctx, "nope", &json!({})).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.message.contains("nope"));
    }

    #[test]
    fn add_imap_account_dispatch_creates_row_and_secret() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let Dispatched::Reply(value) =
            dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap()
        else {
            panic!("add_imap_account must reply");
        };
        let account_id = value["account_id"].as_i64().expect("account_id");
        ctx.writer.flush();
        let reader = ctx.reader.acquire().unwrap();
        let stored: String = reader
            .query_row(
                "SELECT imap_host FROM accounts WHERE id = ?1",
                [account_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "imap.example.com");
        assert_eq!(
            ctx.keystore
                .get(&mailslate_core::infra::keyring::imap_password_key(
                    account_id
                ))
                .unwrap(),
            Some("secret-pw".to_string())
        );
    }

    #[test]
    fn remove_account_dispatch_scrubs_secrets_and_row() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let Dispatched::Reply(added) =
            dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap()
        else {
            panic!("add_imap_account must reply");
        };
        let account_id = added["account_id"].as_i64().unwrap();
        let key = mailslate_core::infra::keyring::imap_password_key(account_id);
        ctx.keystore
            .store(
                &mailslate_core::infra::keyring::oauth_key(account_id),
                "tok",
            )
            .unwrap();

        dispatch(
            &ctx,
            methods::REMOVE_ACCOUNT,
            &json!({ "account_id": account_id }),
        )
        .unwrap();

        assert_eq!(ctx.keystore.get(&key).unwrap(), None);
        assert_eq!(
            ctx.keystore
                .get(&mailslate_core::infra::keyring::oauth_key(account_id))
                .unwrap(),
            None
        );
        let Dispatched::Reply(listing) = dispatch(&ctx, methods::GET_ACCOUNTS, &json!({})).unwrap()
        else {
            panic!("get_accounts must reply");
        };
        assert_eq!(listing["accounts"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn update_account_dispatch_applies_patch() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let Dispatched::Reply(added) =
            dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap()
        else {
            panic!("add_imap_account must reply");
        };
        let account_id = added["account_id"].as_i64().unwrap();

        dispatch(
            &ctx,
            methods::UPDATE_ACCOUNT,
            &json!({
                "account_id": account_id,
                "patch": { "display_name": "Renamed", "color": null, "sync_window_days": 90 }
            }),
        )
        .unwrap();

        let Dispatched::Reply(listing) = dispatch(&ctx, methods::GET_ACCOUNTS, &json!({})).unwrap()
        else {
            panic!("get_accounts must reply");
        };
        let account = &listing["accounts"].as_array().unwrap()[0];
        assert_eq!(account["display_name"], json!("Renamed"));
        assert_eq!(account["sync_window_days"], json!(90));
    }

    #[test]
    fn get_accounts_dispatch_lists_rows() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap();
        dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap();

        let Dispatched::Reply(listing) = dispatch(&ctx, methods::GET_ACCOUNTS, &json!({})).unwrap()
        else {
            panic!("get_accounts must reply");
        };
        let accounts = listing["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0]["provider"], json!("imap"));
        assert_eq!(accounts[0]["auth_type"], json!("password"));
    }
}
