//! Request dispatch: the daemon side of the method table.
//!
//! Only `ping` and `shutdown` exist in M1; the mail methods land with their
//! features (ARCHITECTURE.md §5.3).

use mailslate_core::domain::accounts::{
    AccountPatch, AuthKind, ImapAccountDraft, PENDING_ACCOUNT_ID, Security, add_imap_account,
    add_oauth_account, get_accounts, remove_account, update_account,
};
use mailslate_core::infra::db::pool::ReadPool;
use mailslate_core::infra::db::writer::WriterActor;
use mailslate_core::infra::keyring::KeyStore;
use mailslate_core::infra::oauth::authorize;
use mailslate_core::infra::oauth::grant::{self, TokenPoster};
use mailslate_core::infra::oauth::loopback::LoopbackListener;
use mailslate_core::infra::oauth::pkce;
use mailslate_core::infra::oauth::providers::{self, Provider};
use mailslate_core::infra::oauth::{OAuthError, tokens};
use mailslate_protocol::message::Event;
use mailslate_protocol::{BridgeError, ErrorKind, PROTOCOL_VERSION, PingResult, methods};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

use crate::events::EventBus;

/// What the connection handler should do after replying.
#[derive(Debug)]
pub enum Dispatched {
    /// Reply and keep serving.
    Reply(Value),
    /// Reply, then shut the daemon down.
    Stop(Value),
}

/// An OAuth flow started but not yet completed: the verifier/state pair
/// behind the browser authorization, kept until the GUI confirms.
///
/// The waiter thread receives its own clones of `verifier`/`state`/
/// `redirect_uri`, so the stored fields are observability only — the
/// completion path consumes just `provider`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PendingAuth {
    pub provider: Provider,
    pub verifier: String,
    pub state: String,
    pub redirect_uri: String,
}

/// State shared by every dispatched call: the single-writer handle,
/// the keyring-backed secret store, the read pool, the event bus, and
/// the in-flight OAuth session.
pub struct DispatchCtx {
    /// The only database writer (batched transactions).
    pub writer: Arc<WriterActor>,
    /// Secrets live here; the daemon is the sole keyring user.
    pub keystore: Arc<dyn KeyStore>,
    /// Readonly connections for read-only methods.
    pub reader: Arc<ReadPool>,
    /// Domain events fanned out to connected GUIs.
    pub bus: Arc<EventBus>,
    /// The OAuth session waiting for `complete_oauth_account`.
    pub oauth: std::sync::Mutex<Option<PendingAuth>>,
    /// Token endpoint used for exchanges (swapped for fakes in tests).
    pub token_poster: Arc<dyn TokenPoster>,
    /// How long the callback waiter waits before giving up.
    pub oauth_deadline: Duration,
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
        methods::START_OAUTH => start_oauth_dispatch(ctx, params).map(Dispatched::Reply),
        methods::COMPLETE_OAUTH_ACCOUNT => {
            complete_oauth_account_dispatch(ctx, params).map(Dispatched::Reply)
        }
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

/// Starts an OAuth authorization-code flow: binds a loopback listener,
/// builds the provider authorize URL, publishes `oauth_url_ready`, and
/// spawns the callback waiter that stages tokens for
/// [`complete_oauth_account_dispatch`].
fn start_oauth_dispatch(ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let provider = match string(params, "provider")?.as_str() {
        "google" => Provider::Google,
        "microsoft" => Provider::Microsoft,
        other => {
            return Err(validation(format!("unknown oauth provider: {other}")));
        }
    };
    let client_id = providers::client_id(provider).map_err(oauth_error)?;
    let profile = providers::profile(provider);

    let listener = LoopbackListener::bind()
        .map_err(|err| BridgeError::new(ErrorKind::Internal, format!("loopback bind: {err}")))?;
    let redirect_uri = format!("http://127.0.0.1:{}/callback", listener.port());
    let verifier = pkce::new_verifier();
    let state = pkce::new_state();
    let url = authorize::authorize_url(
        profile,
        &client_id,
        &redirect_uri,
        &state,
        &pkce::challenge_for(&verifier),
    );

    *ctx.oauth.lock().expect("oauth session lock") = Some(PendingAuth {
        provider,
        verifier: verifier.clone(),
        state: state.clone(),
        redirect_uri: redirect_uri.clone(),
    });

    ctx.bus
        .publish(Event::new("oauth_url_ready", json!({ "url": url })));
    spawn_oauth_waiter(
        ctx,
        listener,
        provider,
        client_id,
        verifier,
        state,
        redirect_uri,
    );
    Ok(json!({ "url": url }))
}

/// Waits for the browser callback out-of-band: when it arrives the flow
/// completes against the token endpoint and the snapshot is staged under
/// [`PENDING_ACCOUNT_ID`]; every outcome is announced on the bus.
fn spawn_oauth_waiter(
    ctx: &DispatchCtx,
    listener: LoopbackListener,
    provider: Provider,
    client_id: String,
    verifier: String,
    state: String,
    redirect_uri: String,
) {
    let waiter = listener.for_waiter();
    let keystore = Arc::clone(&ctx.keystore);
    let poster = Arc::clone(&ctx.token_poster);
    let bus = Arc::clone(&ctx.bus);
    let deadline = ctx.oauth_deadline;
    std::thread::spawn(move || {
        let outcome: Result<Option<String>, String> = (|| {
            let (code, _) = waiter
                .wait_for_code(&state, deadline)
                .map_err(|err| err.to_string())?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| err.to_string())?;
            let (snapshot, email) = runtime
                .block_on(grant::exchange_code_detail(
                    poster.as_ref(),
                    provider,
                    &code,
                    &verifier,
                    &redirect_uri,
                    &client_id,
                    now_unix(),
                ))
                .map_err(|err| err.to_string())?;
            tokens::save(keystore.as_ref(), PENDING_ACCOUNT_ID, &snapshot)
                .map_err(|err| err.to_string())?;
            Ok(email)
        })();
        match outcome {
            Ok(email) => bus.publish(Event::new(
                "oauth_completed",
                json!({
                    "account_pending": PENDING_ACCOUNT_ID,
                    "email": email,
                }),
            )),
            Err(reason) => bus.publish(Event::new("oauth_failed", json!({ "reason": reason }))),
        }
    });
}

/// Turns the staged OAuth tokens into a real account: consumes the
/// pending session, moves the snapshot to the new account's keyring
/// keys, and writes the row with the provider's endpoints.
fn complete_oauth_account_dispatch(
    ctx: &DispatchCtx,
    params: &Value,
) -> Result<Value, BridgeError> {
    let session = ctx
        .oauth
        .lock()
        .expect("oauth session lock")
        .take()
        .ok_or_else(|| validation("no pending oauth session"))?;
    let display_name = string(params, "display_name")?;
    let email = string(params, "email")?;
    let color = optional(params, "color")?;
    let profile = providers::profile(session.provider);
    let staged = tokens::load(ctx.keystore.as_ref(), PENDING_ACCOUNT_ID)
        .map_err(|err| BridgeError::new(ErrorKind::Internal, err.to_string()))?
        .ok_or_else(|| validation("oauth token snapshot missing from the keyring"))?;

    let account_id = add_oauth_account(
        ctx.keystore.as_ref(),
        &ctx.writer,
        session.provider.db_name(),
        &display_name,
        &email,
        color,
        (profile.imap_host, profile.imap_port, profile.imap_security),
        (profile.smtp_host, profile.smtp_port, profile.smtp_security),
        &staged,
    )
    .map_err(account_error)?;
    Ok(json!({ "account_id": account_id }))
}

/// Maps an OAuth failure onto the wire error vocabulary.
fn oauth_error(err: OAuthError) -> BridgeError {
    BridgeError::new(ErrorKind::Internal, err.to_string())
}

/// Unix seconds; token expiry arithmetic uses whole seconds only.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs() as i64
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
        test_ctx_with(dir, std::time::Duration::from_secs(5))
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

    // ---- OAuth authorize-code flow (ARCHITECTURE.md §8) ----

    use mailslate_core::infra::keyring::oauth_key;
    use mailslate_core::infra::oauth::grant::{TokenPoster, TokenResponse};
    use mailslate_core::infra::oauth::providers::Provider;
    use mailslate_core::infra::oauth::{OAuthError, tokens};
    use mailslate_protocol::message::Event;
    use std::io::{Read, Write};
    use std::time::Duration;

    /// Serializes client-id env mutations; daemon tests share one process.
    static OAUTH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Installs a public Google client id for one test's lifetime.
    struct ClientIdGuard;
    impl ClientIdGuard {
        fn set(value: &str) -> Self {
            unsafe { std::env::set_var("MAILSLATE_GOOGLE_CLIENT_ID", value) };
            Self
        }
    }
    impl Drop for ClientIdGuard {
        fn drop(&mut self) {
            unsafe { std::env::remove_var("MAILSLATE_GOOGLE_CLIENT_ID") };
        }
    }

    /// A canned token endpoint: asserts the grant type, replies with a
    /// success token response carrying an email id_token.
    struct FakePoster;
    impl TokenPoster for FakePoster {
        fn post(
            &self,
            _url: &str,
            form: &[(&str, &str)],
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<TokenResponse, OAuthError>> + Send + '_>,
        > {
            let grant_type = form
                .iter()
                .find(|(key, _)| *key == "grant_type")
                .map(|(_, value)| value.to_string());
            Box::pin(async move {
                assert_eq!(grant_type.as_deref(), Some("authorization_code"));
                Ok(TokenResponse {
                    access_token: "at-1".into(),
                    refresh_token: Some("rt-1".into()),
                    expires_in: 3600,
                    scope: None,
                    id_token: Some(id_token_with_email("user@example.com")),
                })
            })
        }
    }

    fn b64url(raw: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
    }

    fn id_token_with_email(email: &str) -> String {
        let header = b64url(br#"{"alg":"none"}"#);
        let payload = b64url(format!(r#"{{"email":"{email}"}}"#).as_bytes());
        format!("{header}.{payload}.sig")
    }

    fn test_ctx_with(dir: &tempfile::TempDir, oauth_deadline: Duration) -> Arc<DispatchCtx> {
        let db_path = dir.path().join(DB_FILE_NAME);
        let mut conn = mailslate_core::infra::db::open_rw(&db_path).unwrap();
        migrations::run(&mut conn).unwrap();
        drop(conn);
        Arc::new(DispatchCtx {
            writer: Arc::new(WriterActor::spawn(&db_path)),
            keystore: Arc::new(FakeKeyStore::new()),
            reader: Arc::new(ReadPool::open(&db_path).unwrap()),
            bus: crate::events::EventBus::new(),
            oauth: std::sync::Mutex::new(None),
            token_poster: Arc::new(FakePoster),
            oauth_deadline,
        })
    }

    /// Pulls the next event named `name`, skipping interleaved others.
    fn wait_event(events: &mut tokio::sync::broadcast::Receiver<Event>, name: &str) -> Event {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            match events.try_recv() {
                Ok(event) if event.event == name => return event,
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {}
                Err(_) => {}
            }
            if std::time::Instant::now() > deadline {
                panic!("event {name} never arrived");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn start_oauth_publishes_url_and_stores_session() {
        let _env = OAUTH_ENV_LOCK.lock().unwrap();
        let _id = ClientIdGuard::set("google-app-id");
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(5));
        let mut events = ctx.bus.subscribe();

        let Dispatched::Reply(value) =
            dispatch(&ctx, methods::START_OAUTH, &json!({ "provider": "google" })).unwrap()
        else {
            panic!("start_oauth must reply");
        };
        let url = value["url"].as_str().expect("authorize url").to_string();
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth"));

        let ready = wait_event(&mut events, "oauth_url_ready");
        assert_eq!(ready.data["url"], json!(url));

        let session = ctx.oauth.lock().unwrap().clone().expect("pending session");
        assert_eq!(session.provider, Provider::Google);
        assert!(!session.verifier.is_empty());
        assert!(!session.state.is_empty());
    }

    #[test]
    fn start_oauth_rejects_unknown_provider() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(5));
        let err =
            dispatch(&ctx, methods::START_OAUTH, &json!({ "provider": "yahoo" })).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
    }

    #[test]
    fn oauth_waiter_stages_tokens_and_publishes_completed() {
        let _env = OAUTH_ENV_LOCK.lock().unwrap();
        let _id = ClientIdGuard::set("google-app-id");
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(5));
        let mut events = ctx.bus.subscribe();

        let Dispatched::Reply(value) =
            dispatch(&ctx, methods::START_OAUTH, &json!({ "provider": "google" })).unwrap()
        else {
            panic!("start_oauth must reply");
        };
        let url = value["url"].as_str().expect("authorize url").to_string();
        let _ready = wait_event(&mut events, "oauth_url_ready");

        let session = ctx.oauth.lock().unwrap().clone().expect("pending session");
        let port: u16 = session
            .redirect_uri
            .trim_start_matches("http://127.0.0.1:")
            .trim_end_matches("/callback")
            .parse()
            .expect("loopback port in redirect_uri");

        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "GET /callback?code=cd-9&state={} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            session.state
        )
        .unwrap();
        let mut scratch = [0u8; 256];
        let _ = stream.read(&mut scratch);
        drop(stream);

        let completed = wait_event(&mut events, "oauth_completed");
        assert_eq!(completed.data["email"], json!("user@example.com"));

        let staged = ctx
            .keystore
            .get(&oauth_key(0))
            .unwrap()
            .expect("staged tokens");
        assert!(staged.contains("at-1"), "staged snapshot: {staged}");
        let _ = url;
    }

    #[test]
    fn oauth_waiter_publishes_failure_on_timeout() {
        let _env = OAUTH_ENV_LOCK.lock().unwrap();
        let _id = ClientIdGuard::set("google-app-id");
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_millis(200));
        let mut events = ctx.bus.subscribe();

        dispatch(&ctx, methods::START_OAUTH, &json!({ "provider": "google" })).unwrap();
        let _ready = wait_event(&mut events, "oauth_url_ready");

        let failed = wait_event(&mut events, "oauth_failed");
        assert!(failed.data["reason"].is_string());
    }

    #[test]
    fn complete_oauth_account_creates_oauth2_account() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(5));

        ctx.oauth.lock().unwrap().replace(PendingAuth {
            provider: Provider::Google,
            verifier: "verifier".into(),
            state: "state".into(),
            redirect_uri: "http://127.0.0.1:1/callback".into(),
        });
        tokens::save(
            ctx.keystore.as_ref(),
            0,
            &mailslate_core::infra::oauth::tokens::TokenSnapshot {
                access_token: "at-1".into(),
                refresh_token: "rt-1".into(),
                expires_at: 4_000_000_000,
                scopes: String::new(),
            },
        )
        .unwrap();

        let Dispatched::Reply(value) = dispatch(
            &ctx,
            methods::COMPLETE_OAUTH_ACCOUNT,
            &json!({ "display_name": "Gmail", "email": "me@gmail.com", "color": "blue" }),
        )
        .unwrap() else {
            panic!("complete_oauth_account must reply");
        };
        let account_id = value["account_id"].as_i64().expect("account_id");
        ctx.writer.flush();

        let reader = ctx.reader.acquire().unwrap();
        let (provider, auth_type, imap_host): (String, String, String) = reader
            .query_row(
                "SELECT provider, auth_type, imap_host FROM accounts WHERE id = ?1",
                [account_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(provider, "gmail");
        assert_eq!(auth_type, "oauth2");
        assert_eq!(imap_host, "imap.gmail.com");

        let moved = ctx
            .keystore
            .get(&oauth_key(account_id))
            .unwrap()
            .expect("moved tokens");
        assert!(moved.contains("at-1"));
        assert_eq!(ctx.keystore.get(&oauth_key(0)).unwrap(), None);
        assert!(ctx.oauth.lock().unwrap().is_none(), "session consumed");
    }

    #[test]
    fn complete_oauth_account_requires_a_pending_session() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(5));
        let err = dispatch(
            &ctx,
            methods::COMPLETE_OAUTH_ACCOUNT,
            &json!({ "display_name": "Gmail", "email": "me@gmail.com" }),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.message.contains("pending"));
    }
}
