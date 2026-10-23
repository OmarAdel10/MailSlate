//! Request dispatch: the daemon side of the method table.
//!
//! Only `ping` and `shutdown` exist in M1; the mail methods land with their
//! features (ARCHITECTURE.md §5.3).

use mailslate_core::domain::accounts::{
    AccountPatch, AuthKind, ImapAccountDraft, PENDING_ACCOUNT_ID, Security, add_imap_account,
    add_oauth_account, get_account_connections, get_accounts, remove_account, update_account,
};
use mailslate_core::domain::search::search;
use mailslate_core::infra::db::pool::ReadPool;
use mailslate_core::infra::db::writer::WriterActor;
use mailslate_core::infra::imap::probe::{ProbeError, probe_imap_login, probe_smtp};
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mailslate_core::domain::events::EventBus;
use mailslate_core::domain::sync::actor::{AccountConfig, SyncHandle, spawn_actor};

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
    /// `Some(account_id)` when the flow re-authorizes an existing
    /// account instead of creating a new one.
    pub reauth_account: Option<i64>,
    /// Set to true when `abort_oauth_account_dispatch` is called, so
    /// the in-flight waiter becomes a no-op instead of re-staging
    /// tokens under `PENDING_ACCOUNT_ID`.
    pub aborted: Arc<AtomicBool>,
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
    ///
    /// `Arc` so the callback waiter can release the slot when its flow
    /// dies — every outcome except a wizard success clears the session,
    /// otherwise one failed or reconnecting flow would reject all
    /// later starts until the daemon restarts.
    pub oauth: Arc<std::sync::Mutex<Option<PendingAuth>>>,
    /// Token endpoint used for exchanges (swapped for fakes in tests).
    pub token_poster: Arc<dyn TokenPoster>,
    /// How long the callback waiter waits before giving up.
    pub oauth_deadline: Duration,
    /// Running sync actors by account id (daemon spawns one each).
    ///
    /// `Arc` so the OAuth waiter thread can restart an account's actor
    /// after a successful reconnect.
    pub sync: Arc<std::sync::Mutex<std::collections::HashMap<i64, SyncHandle>>>,
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
        methods::TEST_CONNECTION => test_connection_dispatch(ctx, params).map(Dispatched::Reply),
        methods::REMOVE_ACCOUNT => remove_account_dispatch(ctx, params).map(Dispatched::Reply),
        methods::UPDATE_ACCOUNT => update_account_dispatch(ctx, params).map(Dispatched::Reply),
        methods::GET_ACCOUNTS => get_accounts_dispatch(ctx).map(Dispatched::Reply),
        methods::START_OAUTH => start_oauth_dispatch(ctx, params).map(Dispatched::Reply),
        methods::START_REAUTH => start_reauth_dispatch(ctx, params).map(Dispatched::Reply),
        methods::COMPLETE_OAUTH_ACCOUNT => {
            complete_oauth_account_dispatch(ctx, params).map(Dispatched::Reply)
        }
        methods::ABORT_OAUTH_ACCOUNT => abort_oauth_account_dispatch(ctx).map(Dispatched::Reply),
        methods::SYNC_NOW => sync_now_dispatch(ctx, params).map(Dispatched::Reply),
        methods::GET_STATUS => get_status_dispatch(ctx).map(Dispatched::Reply),
        methods::SEARCH => search_dispatch(ctx, params).map(Dispatched::Reply),
        other => Err(BridgeError::new(
            ErrorKind::NotFound,
            format!("unknown method: {other}"),
        )),
    }
}

/// Parses the shared `{conn, creds}` payload accepted by
/// `add_imap_account` and `test_connection`.
fn conn_and_password(params: &Value) -> Result<(ImapAccountDraft, String), BridgeError> {
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
    Ok((draft, password))
}

/// Parses and applies `add_imap_account`, persisting the row and the
/// secret before replying, then starts its sync actor so the initial
/// sync begins immediately (USER_FLOW §3 step 7).
fn add_imap_account_dispatch(ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let (draft, password) = conn_and_password(params)?;
    let account_id = blocking(|| {
        add_imap_account(ctx.keystore.as_ref(), &ctx.writer, &draft, &password)
            .map_err(account_error)
    })?;
    ensure_sync_actor(ctx, account_id)?;
    Ok(json!({ "account_id": account_id }))
}

/// Runs the `test_connection` probes without touching the database. Both
/// steps always run so the wizard can report IMAP and SMTP separately
/// (USER_FLOW §3 steps 4-5).
fn test_connection_dispatch(_ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let (draft, password) = conn_and_password(params)?;
    // Plaintext is rejected at the test_connection boundary — credentials
    // must never traverse cleartext TCP (CWE-319). Probe functions accept
    // Plaintext for unit-test compatibility; the security gate is here.
    if draft.imap_security == mailslate_core::domain::accounts::Security::Plaintext
        || draft.smtp_security == mailslate_core::domain::accounts::Security::Plaintext
    {
        return Err(validation(
            "plaintext security is not allowed: credentials sent in cleartext",
        ));
    }
    // The daemon runs a multi-thread runtime (main.rs), so blocking a worker
    // here is legal; each probe enforces its own timeout inside core. The two
    // probes run concurrently (tokio::join!) so a slow IMAP server cannot
    // stall the SMTP reachability check — both steps always run (QA#19).
    let (imap, smtp) = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            tokio::join!(
                probe_imap_login(
                    &draft.imap_host,
                    draft.imap_port,
                    draft.imap_security,
                    &draft.email,
                    &password,
                ),
                probe_smtp(&draft.smtp_host, draft.smtp_port, draft.smtp_security),
            )
        })
    });
    Ok(json!({
        "imap": probe_step(imap),
        "smtp": probe_step(smtp),
    }))
}

/// Serializes one probe outcome as `{ok:true}` or `{ok:false,error}`.
fn probe_step(step: Result<(), ProbeError>) -> Value {
    match step {
        Ok(()) => json!({ "ok": true }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    }
}

/// Runs a database-flushing account op without parking an async
/// worker: on the daemon's multi-thread runtime the closure moves off
/// the worker thread, so a stalled fsync blocks one blocking-pool
/// thread instead of a connection worker (writer flushes wait up to
/// 30 s). Tests without a multi-thread runtime run it inline.
fn blocking<T>(op: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle)
            if matches!(
                handle.runtime_flavor(),
                tokio::runtime::RuntimeFlavor::MultiThread
            ) =>
        {
            tokio::task::block_in_place(op)
        }
        _ => op(),
    }
}

/// Parses and applies `remove_account`, stopping the sync actor first
/// (so the running task can no longer write rows for an account whose
/// rows are about to disappear) and scrubbing the keyring after.
fn remove_account_dispatch(ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let account_id = account_id(params)?;
    if let Some(handle) = ctx
        .sync
        .lock()
        .expect("sync handle lock")
        .remove(&account_id)
    {
        handle.try_shutdown();
    }
    blocking(|| {
        remove_account(ctx.keystore.as_ref(), &ctx.writer, account_id).map_err(account_error)
    })?;
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
    let sync_window_days = match patch.get("sync_window_days") {
        None => None,
        Some(Value::Null) => None,
        Some(Value::Number(n)) => {
            let v = n
                .as_i64()
                .ok_or_else(|| validation("patch.sync_window_days must be an integer"))?;
            if !(1..=3650).contains(&v) {
                return Err(validation(
                    "patch.sync_window_days must be between 1 and 3650 days",
                ));
            }
            Some(v)
        }
        Some(_) => return Err(validation("patch.sync_window_days must be an integer")),
    };
    let parsed = AccountPatch {
        display_name: patch
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        color,
        sync_window_days,
    };
    blocking(|| update_account(&ctx.writer, account_id, parsed).map_err(account_error))?;
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

/// Starts the wizard's OAuth flow: no account exists yet, so the
/// tokens are staged under [`PENDING_ACCOUNT_ID`] until
/// [`complete_oauth_account_dispatch`] names the row.
fn start_oauth_dispatch(ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let provider = match string(params, "provider")?.as_str() {
        "google" => Provider::Google,
        "microsoft" => Provider::Microsoft,
        other => {
            return Err(validation(format!("unknown oauth provider: {other}")));
        }
    };
    start_flow(ctx, provider, None)
}

/// Re-authorizes an existing OAuth account (PRD F1.6): its refresh
/// token was rejected and the sync actor parked in `reauth_required`.
/// The callback saves the new snapshot directly to that account and
/// restarts its actor; nothing is staged for a new row.
fn start_reauth_dispatch(ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let account_id = account_id(params)?;
    let conn = ctx
        .reader
        .acquire()
        .map_err(|err| BridgeError::new(ErrorKind::Internal, format!("read pool: {err}")))?;
    let account = get_account_connections(&conn)
        .map_err(|err| BridgeError::new(ErrorKind::Internal, err.to_string()))?
        .into_iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| {
            BridgeError::new(ErrorKind::NotFound, format!("no account row {account_id}"))
        })?;
    drop(conn);
    if account.auth_type != "oauth2" {
        return Err(validation(
            "only oauth2 accounts reconnect; password accounts use their saved password",
        ));
    }
    let provider = match account.provider.as_str() {
        "gmail" => Provider::Google,
        "outlook" => Provider::Microsoft,
        other => {
            return Err(validation(format!(
                "account {account_id} uses provider {other}, which has no oauth flow"
            )));
        }
    };
    start_flow(ctx, provider, Some(account_id))
}

/// Binds the loopback listener, builds the authorize URL, records the
/// session (only one may be pending, so a stray second browser flow can
/// never overwrite the first) and spawns the callback waiter.
fn start_flow(
    ctx: &DispatchCtx,
    provider: Provider,
    reauth_account: Option<i64>,
) -> Result<Value, BridgeError> {
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

    let abort_flag = {
        let mut session = ctx.oauth.lock().expect("oauth session lock");
        if session.is_some() {
            return Err(validation(
                "an oauth flow is already waiting; finish it or let it time out first",
            ));
        }
        *session = Some(PendingAuth {
            provider,
            verifier: verifier.clone(),
            state: state.clone(),
            redirect_uri: redirect_uri.clone(),
            reauth_account,
            aborted: Arc::new(AtomicBool::new(false)),
        });
        Arc::clone(&session.as_ref().expect("just inserted").aborted)
    };

    ctx.bus.publish(Event::new(
        "oauth_url_ready",
        json!({
            "url": url,
            "provider": provider.db_name(),
            "account": reauth_account,
        }),
    ));
    spawn_oauth_waiter(
        ctx,
        listener,
        OauthFlow {
            provider,
            client_id,
            verifier,
            state,
            redirect_uri,
            reauth_account,
            aborted: abort_flag,
        },
        tokio::runtime::Handle::try_current().ok(),
    );
    Ok(json!({ "url": url }))
}

/// The per-flow values the waiter thread needs after the session is stored.
struct OauthFlow {
    provider: Provider,
    client_id: String,
    verifier: String,
    state: String,
    redirect_uri: String,
    reauth_account: Option<i64>,
    /// Set by `abort_oauth_account_dispatch`; when true the waiter
    /// becomes a no-op instead of re-staging tokens.
    aborted: Arc<AtomicBool>,
}

/// Waits for the browser callback out-of-band: when it arrives the flow
/// completes against the token endpoint and the snapshot is staged under
/// [`PENDING_ACCOUNT_ID`]; every outcome is announced on the bus.
fn spawn_oauth_waiter(
    ctx: &DispatchCtx,
    listener: LoopbackListener,
    flow: OauthFlow,
    runtime: Option<tokio::runtime::Handle>,
) {
    let waiter = listener.for_waiter();
    let keystore = Arc::clone(&ctx.keystore);
    let poster = Arc::clone(&ctx.token_poster);
    let bus = Arc::clone(&ctx.bus);
    let writer = Arc::clone(&ctx.writer);
    let reader = Arc::clone(&ctx.reader);
    let sync = Arc::clone(&ctx.sync);
    let session_slot = Arc::clone(&ctx.oauth);
    let deadline = ctx.oauth_deadline;
    let aborted = Arc::clone(&flow.aborted);
    std::thread::spawn(move || {
        let OauthFlow {
            provider,
            client_id,
            verifier,
            state,
            redirect_uri,
            reauth_account,
            aborted: _,
        } = flow;
        let outcome: Result<Option<String>, String> = (|| {
            let (code, _) = waiter
                .wait_for_code(&state, deadline)
                .map_err(|err| err.to_string())?;
            // Aborted by the wizard (back / timeout / retry): do not
            // save staged tokens, do not publish oauth_completed — just
            // release the session slot so a new flow can start.
            if aborted.load(Ordering::SeqCst) {
                return Ok(None);
            }
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
            // A reconnect refreshes the account's own snapshot; the
            // wizard path stages tokens for a row that does not exist
            // yet.
            let target = reauth_account.unwrap_or(PENDING_ACCOUNT_ID);
            tokens::save(keystore.as_ref(), target, &snapshot).map_err(|err| err.to_string())?;
            Ok(email)
        })();
        let mut release_session = true;
        match outcome {
            Ok(Some(email)) => match reauth_account {
                Some(account_id) => {
                    bus.publish(Event::new(
                        "oauth_reconnected",
                        json!({
                            "account": account_id,
                            "email": email,
                            "provider": provider.db_name(),
                        }),
                    ));
                    if let Err(err) = restart_actor(
                        &runtime, &keystore, &writer, &reader, &bus, &poster, &sync, account_id,
                    ) {
                        bus.publish(Event::new(
                            "oauth_failed",
                            json!({
                                "reason": format!("reconnected, but the sync actor could not restart: {err}"),
                                "provider": provider.db_name(),
                                "account": account_id,
                            }),
                        ));
                    }
                }
                None => {
                    // The wizard session must survive until
                    // `complete_oauth_account` consumes it.
                    release_session = false;
                    bus.publish(Event::new(
                        "oauth_completed",
                        json!({
                            "account_pending": PENDING_ACCOUNT_ID,
                            "email": email,
                            "provider": provider.db_name(),
                        }),
                    ));
                }
            },
            Ok(None) => {
                // Aborted by the wizard (back / timeout / retry): session
                // slot already cleared by `abort_oauth_account_dispatch`;
                // nothing to publish, no staged tokens to clean up.
            }
            Err(reason) => bus.publish(Event::new(
                "oauth_failed",
                json!({
                    "reason": reason,
                    "provider": provider.db_name(),
                    "account": reauth_account,
                }),
            )),
        }
        if release_session {
            release_oauth_session(&session_slot, &state);
        }
    });
}

/// Releases the daemon's single OAuth session slot after a terminated
/// flow so later starts are not rejected forever. The state match
/// keeps a (today impossible) superseding flow from being cleared by a
/// stale waiter.
fn release_oauth_session(slot: &Arc<std::sync::Mutex<Option<PendingAuth>>>, state: &str) {
    let mut session = slot.lock().expect("oauth session lock");
    if session
        .as_ref()
        .is_some_and(|pending| pending.state == state)
    {
        *session = None;
    }
}

/// Replaces the parked actor of a freshly reconnected account: the old
/// handle is shut down (it sits in `reauth_required` until then) and a
/// new actor starts from the stored row.
#[allow(clippy::too_many_arguments)]
fn restart_actor(
    runtime: &Option<tokio::runtime::Handle>,
    keystore: &Arc<dyn KeyStore>,
    writer: &Arc<WriterActor>,
    reader: &Arc<ReadPool>,
    bus: &Arc<EventBus>,
    poster: &Arc<dyn TokenPoster>,
    sync: &Arc<std::sync::Mutex<std::collections::HashMap<i64, SyncHandle>>>,
    account_id: i64,
) -> Result<(), BridgeError> {
    let runtime = runtime.as_ref().ok_or_else(|| {
        BridgeError::new(
            ErrorKind::Internal,
            "no async runtime available to restart the sync actor",
        )
    })?;
    let conn = reader
        .acquire()
        .map_err(|err| BridgeError::new(ErrorKind::Internal, format!("read pool: {err}")))?;
    let account = get_account_connections(&conn)
        .map_err(|err| BridgeError::new(ErrorKind::Internal, err.to_string()))?
        .into_iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| {
            BridgeError::new(ErrorKind::NotFound, format!("no account row {account_id}"))
        })?;
    drop(conn);

    if let Some(handle) = sync.lock().expect("sync handle lock").remove(&account_id) {
        handle.try_shutdown();
    }
    let _entered = runtime.enter();
    let handle = spawn_actor(
        AccountConfig {
            id: account.id,
            email: account.email,
            imap_host: account.imap_host,
            imap_port: account.imap_port,
            imap_security: account.imap_security,
            auth_type: account.auth_type,
            provider: account.provider,
        },
        Arc::clone(keystore),
        Arc::clone(writer),
        Arc::clone(bus),
        Arc::clone(poster),
    );
    sync.lock()
        .expect("sync handle lock")
        .insert(account_id, handle);
    Ok(())
}

/// Aborts an in-flight OAuth flow: releases the single OAuth session
/// slot and drops any staged tokens under [`PENDING_ACCOUNT_ID`], so a
/// new flow can start immediately instead of waiting for the daemon's
/// deadline. Called from the wizard on back / timeout / retry.
fn abort_oauth_account_dispatch(ctx: &DispatchCtx) -> Result<Value, BridgeError> {
    // Mark the flow aborted so the in-flight waiter becomes a no-op
    // instead of re-staging tokens under PENDING_ACCOUNT_ID on a late
    // browser callback.
    if let Some(pending) = ctx.oauth.lock().expect("oauth session lock").as_mut() {
        pending.aborted.store(true, Ordering::SeqCst);
    }
    // Release the session slot if one is pending.
    *ctx.oauth.lock().expect("oauth session lock") = None;
    // Drop any staged tokens so they don't orphan under PENDING_ACCOUNT_ID.
    let _ = tokens::delete(ctx.keystore.as_ref(), PENDING_ACCOUNT_ID);
    Ok(json!({ "ok": true }))
}

/// Turns the staged OAuth tokens into a real account: consumes the
/// pending session, moves the snapshot to the new account's keyring
/// keys, and writes the row with the provider's endpoints.
///
/// The session is only consumed once every fallible check has passed —
/// a failed completion must not discard the still-live browser flow.
fn complete_oauth_account_dispatch(
    ctx: &DispatchCtx,
    params: &Value,
) -> Result<Value, BridgeError> {
    let session = {
        let pending = ctx.oauth.lock().expect("oauth session lock");
        match pending.as_ref() {
            Some(session) if session.reauth_account.is_some() => {
                return Err(validation(
                    "the pending flow reconnects an existing account; it completes on its own",
                ));
            }
            Some(session) => session.clone(),
            None => return Err(validation("no pending oauth session")),
        }
    };
    let display_name = string(params, "display_name")?;
    let email = string(params, "email")?;
    let color = optional(params, "color")?;
    let profile = providers::profile(session.provider);
    let staged = tokens::load(ctx.keystore.as_ref(), PENDING_ACCOUNT_ID)
        .map_err(|err| BridgeError::new(ErrorKind::Internal, err.to_string()))?
        .ok_or_else(|| validation("oauth token snapshot missing from the keyring"))?;

    let account_id = blocking(|| {
        add_oauth_account(
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
        .map_err(account_error)
    })?;

    // ensure_sync_actor can fail even after the row exists. Only once both
    // fallible steps pass do we consume the session — on any earlier
    // failure the session slot stays so the flow can be retried.
    ensure_sync_actor(ctx, account_id)?;

    ctx.oauth
        .lock()
        .expect("oauth session lock")
        .take()
        .expect("the wizard session cannot vanish before completion");

    Ok(json!({ "account_id": account_id }))
}

/// Maps an OAuth failure onto the wire error vocabulary.
fn oauth_error(err: OAuthError) -> BridgeError {
    BridgeError::new(ErrorKind::Internal, err.to_string())
}

/// Spawns the sync actor for `account_id` unless one already runs.
///
/// main.rs only spawns actors for accounts that exist at startup, so
/// every path that creates an account must call this to cover accounts
/// added while the daemon is already serving. Idempotent: a second call
/// for a live actor returns without touching it.
fn ensure_sync_actor(ctx: &DispatchCtx, account_id: i64) -> Result<(), BridgeError> {
    if ctx
        .sync
        .lock()
        .expect("sync handle lock")
        .contains_key(&account_id)
    {
        return Ok(());
    }
    let conn = ctx
        .reader
        .acquire()
        .map_err(|err| BridgeError::new(ErrorKind::Internal, err.to_string()))?;
    let account = get_account_connections(&conn)
        .map_err(|err| BridgeError::new(ErrorKind::Internal, err.to_string()))?
        .into_iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| {
            BridgeError::new(ErrorKind::NotFound, format!("no account row {account_id}"))
        })?;
    let handle = spawn_actor(
        AccountConfig {
            id: account.id,
            email: account.email,
            imap_host: account.imap_host,
            imap_port: account.imap_port,
            imap_security: account.imap_security,
            auth_type: account.auth_type,
            provider: account.provider,
        },
        Arc::clone(&ctx.keystore),
        Arc::clone(&ctx.writer),
        Arc::clone(&ctx.bus),
        Arc::clone(&ctx.token_poster),
    );
    ctx.sync
        .lock()
        .expect("sync handle lock")
        .insert(account_id, handle);
    Ok(())
}

/// Forwards a `sync_now` to the account's actor.
fn sync_now_dispatch(ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let account_id = account_id(params)?;
    let handles = ctx.sync.lock().expect("sync handle lock");
    let handle = handles.get(&account_id).ok_or_else(|| {
        BridgeError::new(
            ErrorKind::NotFound,
            format!("no sync actor for account {account_id}"),
        )
    })?;
    if !handle.try_sync_now() {
        return Err(BridgeError::new(
            ErrorKind::Internal,
            format!("sync actor for account {account_id} is unreachable"),
        ));
    }
    Ok(json!({ "ok": true }))
}

/// Lists every account with its last published sync state.
fn get_status_dispatch(ctx: &DispatchCtx) -> Result<Value, BridgeError> {
    let handles = ctx.sync.lock().expect("sync handle lock");
    let accounts: Vec<Value> = handles
        .iter()
        .map(|(account_id, handle)| {
            json!({
                "id": account_id,
                "state": handle.current_state().as_str(),
            })
        })
        .collect();
    Ok(json!({ "accounts": accounts }))
}

/// Full-text search across one account's messages: parse the syntax
/// subset into an FTS5 `MATCH` + `WHERE` clauses, run against the
/// external-content index, sanitize snippets, reply with hits + total.
fn search_dispatch(ctx: &DispatchCtx, params: &Value) -> Result<Value, BridgeError> {
    let account_id = account_id(params)?;
    let query = string(params, "query")?;
    let hits = blocking(|| {
        let conn = ctx
            .reader
            .acquire()
            .map_err(|err| BridgeError::new(ErrorKind::Internal, format!("read pool: {err}")))?;
        search(&conn, account_id, &query)
            .map_err(|err| BridgeError::new(ErrorKind::Storage, format!("search: {err}")))
    })?;
    let messages: Vec<Value> = hits
        .into_iter()
        .map(|hit| {
            json!({
                "id": hit.message_id,
                "account_id": hit.account_id,
                "subject": hit.subject,
                "snippet": hit.snippet,
            })
        })
        .collect();
    Ok(json!({ "messages": messages, "total": messages.len() }))
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
        "plaintext" => Err(validation(
            "plaintext security is not allowed: credentials sent in cleartext",
        )),
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
    use mailslate_core::infra::keyring::{FakeKeyStore, KeyStore, KeyStoreError};
    use rusqlite;
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

    // Multi-thread runtime: add_imap_account now spawns the sync actor,
    // and tokio::spawn needs a runtime context (production dispatch
    // always runs inside a server connection task).
    #[tokio::test(flavor = "multi_thread")]
    async fn add_imap_account_dispatch_creates_row_and_secret() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let Dispatched::Reply(value) =
            dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap()
        else {
            panic!("add_imap_account must reply");
        };
        let account_id = value["account_id"].as_i64().expect("account_id");
        ctx.writer.flush().expect("flush");
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

    #[tokio::test(flavor = "multi_thread")]
    async fn remove_account_dispatch_scrubs_secrets_and_row() {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn update_account_dispatch_applies_patch() {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn get_accounts_dispatch_lists_rows() {
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

    /// A dead token endpoint: every POST fails (exchange failure).
    struct FailingPoster;
    impl TokenPoster for FailingPoster {
        fn post(
            &self,
            _url: &str,
            _form: &[(&str, &str)],
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<TokenResponse, OAuthError>> + Send + '_>,
        > {
            Box::pin(async { Err(OAuthError::Network("token endpoint down".into())) })
        }
    }

    /// A broken keyring: every secret operation fails.
    struct FailingKeyStore;
    impl KeyStore for FailingKeyStore {
        fn store(&self, _key: &str, _secret: &str) -> Result<(), KeyStoreError> {
            Err(KeyStoreError::Unavailable("keyring locked".into()))
        }
        fn get(&self, _key: &str) -> Result<Option<String>, KeyStoreError> {
            Err(KeyStoreError::Unavailable("keyring locked".into()))
        }
        fn delete(&self, _key: &str) -> Result<(), KeyStoreError> {
            Err(KeyStoreError::Unavailable("keyring locked".into()))
        }
    }

    fn test_ctx_with(dir: &tempfile::TempDir, oauth_deadline: Duration) -> Arc<DispatchCtx> {
        test_ctx_with_doubles(
            dir,
            oauth_deadline,
            Arc::new(FakePoster),
            Arc::new(FakeKeyStore::new()),
        )
    }

    /// A dispatch context with injected token-poster/keyring doubles
    /// for the waiter failure-path tests.
    fn test_ctx_with_doubles(
        dir: &tempfile::TempDir,
        oauth_deadline: Duration,
        poster: Arc<dyn TokenPoster>,
        keystore: Arc<dyn KeyStore>,
    ) -> Arc<DispatchCtx> {
        let db_path = dir.path().join(DB_FILE_NAME);
        let mut conn = mailslate_core::infra::db::open_rw(&db_path).unwrap();
        migrations::run(&mut conn).unwrap();
        drop(conn);
        Arc::new(DispatchCtx {
            writer: Arc::new(WriterActor::spawn(&db_path)),
            keystore,
            reader: Arc::new(ReadPool::open(&db_path).unwrap()),
            bus: mailslate_core::domain::events::EventBus::new(),
            oauth: Arc::new(std::sync::Mutex::new(None)),
            token_poster: poster,
            oauth_deadline,
            sync: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
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

    /// A known provider whose public client id is not configured
    /// surfaces the NotConfigured failure over the wire.
    #[test]
    fn start_oauth_without_a_client_id_is_internal() {
        let _env = OAUTH_ENV_LOCK.lock().unwrap();
        // No ClientIdGuard: the provider env var stays unset.
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(5));
        let err =
            dispatch(&ctx, methods::START_OAUTH, &json!({ "provider": "google" })).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Internal);
        assert!(
            err.message.contains("missing public client id"),
            "got {}",
            err.message
        );
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

    /// A dead token endpoint publishes `oauth_failed` with the
    /// exchange failure (wizard flow).
    #[test]
    fn oauth_waiter_publishes_failure_when_the_exchange_fails() {
        let _env = OAUTH_ENV_LOCK.lock().unwrap();
        let _id = ClientIdGuard::set("google-app-id");
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_doubles(
            &dir,
            Duration::from_secs(5),
            Arc::new(FailingPoster),
            Arc::new(FakeKeyStore::new()),
        );
        let mut events = ctx.bus.subscribe();

        dispatch(&ctx, methods::START_OAUTH, &json!({ "provider": "google" })).unwrap();
        let _ready = wait_event(&mut events, "oauth_url_ready");
        deliver_oauth_callback(&ctx, "cd-9");

        let failed = wait_event(&mut events, "oauth_failed");
        let reason = failed.data["reason"].as_str().expect("reason string");
        assert!(reason.contains("token endpoint down"), "got {reason}");

        // The dead flow must release the single OAuth session slot;
        // otherwise every later start is rejected until restart.
        dispatch(&ctx, methods::START_OAUTH, &json!({ "provider": "google" }))
            .expect("a failed flow must free the session slot");
    }

    /// A keyring that cannot store the snapshot publishes
    /// `oauth_failed` instead of completing silently.
    #[test]
    fn oauth_waiter_publishes_failure_when_the_keyring_fails() {
        let _env = OAUTH_ENV_LOCK.lock().unwrap();
        let _id = ClientIdGuard::set("google-app-id");
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_doubles(
            &dir,
            Duration::from_secs(5),
            Arc::new(FakePoster),
            Arc::new(FailingKeyStore),
        );
        let mut events = ctx.bus.subscribe();

        dispatch(&ctx, methods::START_OAUTH, &json!({ "provider": "google" })).unwrap();
        let _ready = wait_event(&mut events, "oauth_url_ready");
        deliver_oauth_callback(&ctx, "cd-9");

        let failed = wait_event(&mut events, "oauth_failed");
        let reason = failed.data["reason"].as_str().expect("reason string");
        assert!(reason.contains("keyring"), "got {reason}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn complete_oauth_account_creates_oauth2_account() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(5));

        ctx.oauth.lock().unwrap().replace(PendingAuth {
            provider: Provider::Google,
            verifier: "verifier".into(),
            state: "state".into(),
            redirect_uri: "http://127.0.0.1:1/callback".into(),
            reauth_account: None,
            aborted: Arc::new(AtomicBool::new(false)),
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
        ctx.writer.flush().expect("flush");

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

    /// Seeds an oauth2 account row so a reconnect has a target row.
    fn seed_oauth_account(dir: &tempfile::TempDir, email: &str) -> i64 {
        let db_path = dir.path().join(DB_FILE_NAME);
        let mut conn = mailslate_core::infra::db::open_rw(&db_path).unwrap();
        migrations::run(&mut conn).unwrap();
        drop(conn);
        let writer = WriterActor::spawn(&db_path);
        let keystore = FakeKeyStore::new();
        let snapshot = mailslate_core::infra::oauth::tokens::TokenSnapshot {
            access_token: "at-old".into(),
            refresh_token: "rt-old".into(),
            expires_at: 4_000_000_000,
            scopes: String::new(),
        };
        let id = add_oauth_account(
            &keystore,
            &writer,
            "gmail",
            "Gmail",
            email,
            None,
            ("127.0.0.1", 1, Security::Tls),
            ("127.0.0.1", 1, Security::Tls),
            &snapshot,
        )
        .unwrap();
        writer.flush().unwrap();
        id
    }

    /// Plays the browser: hits the loopback callback with the pending
    /// state so the waiter thread can finish the flow.
    fn deliver_oauth_callback(ctx: &DispatchCtx, code: &str) {
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
            "GET /callback?code={code}&state={} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            session.state
        )
        .unwrap();
        let mut scratch = [0u8; 256];
        let _ = stream.read(&mut scratch);
    }

    #[test]
    fn start_oauth_rejects_a_second_pending_flow() {
        let _env = OAUTH_ENV_LOCK.lock().unwrap();
        let _id = ClientIdGuard::set("google-app-id");
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(1));
        dispatch(&ctx, methods::START_OAUTH, &json!({ "provider": "google" })).unwrap();

        let err =
            dispatch(&ctx, methods::START_OAUTH, &json!({ "provider": "google" })).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.message.contains("already waiting"));
    }

    #[test]
    fn start_reauth_rejects_unknown_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(1));
        let err = dispatch(&ctx, methods::START_REAUTH, &json!({ "account_id": 42 })).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.message.contains("42"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_reauth_rejects_password_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap();
        ctx.writer.flush().unwrap();
        let account_id = {
            let conn = ctx.reader.acquire().unwrap();
            get_account_connections(&conn).unwrap()[0].id
        };

        let err = dispatch(
            &ctx,
            methods::START_REAUTH,
            &json!({ "account_id": account_id }),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.message.contains("password"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reauth_saves_tokens_to_the_account_and_restarts_the_actor() {
        let _env = OAUTH_ENV_LOCK.lock().unwrap();
        let _id = ClientIdGuard::set("google-app-id");
        let dir = tempfile::tempdir().unwrap();
        let account_id = seed_oauth_account(&dir, "old@example.com");
        let ctx = test_ctx_with(&dir, Duration::from_secs(5));
        tokens::save(
            ctx.keystore.as_ref(),
            account_id,
            &mailslate_core::infra::oauth::tokens::TokenSnapshot {
                access_token: "at-old".into(),
                refresh_token: "rt-old".into(),
                expires_at: 4_000_000_000,
                scopes: String::new(),
            },
        )
        .unwrap();
        let mut events = ctx.bus.subscribe();

        dispatch(
            &ctx,
            methods::START_REAUTH,
            &json!({ "account_id": account_id }),
        )
        .unwrap();
        let _ready = wait_event(&mut events, "oauth_url_ready");
        deliver_oauth_callback(&ctx, "cd-9");

        let reconnected = wait_event(&mut events, "oauth_reconnected");
        assert_eq!(reconnected.data["account"], json!(account_id));
        assert_eq!(reconnected.data["email"], json!("user@example.com"));

        let saved = ctx
            .keystore
            .get(&oauth_key(account_id))
            .unwrap()
            .expect("reconnected tokens");
        assert!(saved.contains("at-1"), "saved snapshot: {saved}");
        assert!(!saved.contains("at-old"), "stale tokens replaced");
        assert!(
            ctx.sync.lock().unwrap().contains_key(&account_id),
            "the parked actor must be respawned"
        );

        // A finished reconnect must free the session slot for the next
        // flow (the account's next expiry must be able to reconnect).
        dispatch(
            &ctx,
            methods::START_REAUTH,
            &json!({ "account_id": account_id }),
        )
        .expect("a completed reconnect must free the session slot");
    }

    #[test]
    fn complete_oauth_account_rejects_a_reauth_session() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(1));
        ctx.oauth.lock().unwrap().replace(PendingAuth {
            provider: Provider::Google,
            verifier: "verifier".into(),
            state: "state".into(),
            redirect_uri: "http://127.0.0.1:1/callback".into(),
            reauth_account: Some(7),
            aborted: Arc::new(AtomicBool::new(false)),
        });

        let err = dispatch(
            &ctx,
            methods::COMPLETE_OAUTH_ACCOUNT,
            &json!({ "display_name": "Gmail", "email": "me@gmail.com" }),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.message.contains("reconnect"));
        assert!(ctx.oauth.lock().unwrap().is_some(), "session survives");
    }

    #[test]
    fn complete_oauth_account_requires_staged_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with(&dir, Duration::from_secs(1));
        ctx.oauth.lock().unwrap().replace(PendingAuth {
            provider: Provider::Google,
            verifier: "verifier".into(),
            state: "state".into(),
            redirect_uri: "http://127.0.0.1:1/callback".into(),
            reauth_account: None,
            aborted: Arc::new(AtomicBool::new(false)),
        });

        let err = dispatch(
            &ctx,
            methods::COMPLETE_OAUTH_ACCOUNT,
            &json!({ "display_name": "Gmail", "email": "me@gmail.com" }),
        )
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.message.contains("missing from the keyring"));
        assert!(
            ctx.oauth.lock().unwrap().is_some(),
            "a failed completion must not discard the live flow"
        );
    }

    /// A non-string patch color is rejected before any write runs.
    #[test]
    fn update_account_rejects_a_non_string_color() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);

        let err = dispatch(
            &ctx,
            methods::UPDATE_ACCOUNT,
            &json!({ "account_id": 1, "patch": { "color": 42 } }),
        )
        .unwrap_err();

        assert_eq!(err.kind, ErrorKind::Validation);
        assert_eq!(err.message, "patch.color must be a string or null");
    }

    /// A keyring that rejects the secret must leave no account row
    /// behind: the secret is stored before the row is written.
    #[test]
    fn add_imap_account_without_a_keyring_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx_with_doubles(
            &dir,
            Duration::from_secs(5),
            Arc::new(FakePoster),
            Arc::new(FailingKeyStore),
        );

        let err = dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap_err();

        assert_eq!(err.kind, ErrorKind::Internal);
        assert!(err.message.contains("key store rejected the secret"));
        let conn = ctx.reader.acquire().unwrap();
        let accounts = get_accounts(&conn).unwrap();
        assert!(accounts.is_empty(), "no row may reference a lost secret");
    }

    /// When the post-reconnect actor respawn fails (here: every pooled
    /// read connection is held), the flow reports the failure instead
    /// of silently leaving the account parked.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reauth_flow_reports_a_failed_actor_restart() {
        let _env = OAUTH_ENV_LOCK.lock().unwrap();
        let _id = ClientIdGuard::set("google-app-id");
        let dir = tempfile::tempdir().unwrap();
        let account_id = seed_oauth_account(&dir, "restart@example.com");
        let ctx = test_ctx_with(&dir, Duration::from_secs(5));
        let mut events = ctx.bus.subscribe();

        dispatch(
            &ctx,
            methods::START_REAUTH,
            &json!({ "account_id": account_id }),
        )
        .unwrap();
        let _ready = wait_event(&mut events, "oauth_url_ready");

        // Hold every pooled read connection so the respawn cannot
        // re-read the account row.
        let mut held = Vec::new();
        for _ in 0..mailslate_core::infra::db::pool::READ_POOL_SIZE {
            held.push(ctx.reader.acquire().unwrap());
        }

        deliver_oauth_callback(&ctx, "cd-rs");

        let reconnected = wait_event(&mut events, "oauth_reconnected");
        assert_eq!(reconnected.data["account"], json!(account_id));
        let failed = wait_event(&mut events, "oauth_failed");
        let reason = failed.data["reason"].as_str().unwrap();
        assert!(
            reason.contains("could not restart"),
            "restart failure must surface, got: {reason}"
        );

        drop(held);
        // The refreshed tokens still landed under the account key.
        let saved = ctx
            .keystore
            .get(&oauth_key(account_id))
            .unwrap()
            .expect("reconnected tokens");
        assert!(saved.contains("at-1"), "saved snapshot: {saved}");
        assert!(
            !saved.contains("at-old"),
            "the refresh must survive a failed respawn"
        );
    }

    #[test]
    fn sync_now_without_an_actor_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let err = dispatch(&ctx, methods::SYNC_NOW, &json!({ "account_id": 1 })).unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
        assert!(err.message.contains("no sync actor"));
    }

    /// A running actor accepts `sync_now` and replies ok.
    #[tokio::test(flavor = "multi_thread")]
    async fn sync_now_reaches_a_running_actor() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let account_id = seed_oauth_account(&dir, "actor@example.com");
        ensure_sync_actor(&ctx, account_id).expect("spawn actor");

        let Dispatched::Reply(value) = dispatch(
            &ctx,
            methods::SYNC_NOW,
            &json!({ "account_id": account_id }),
        )
        .unwrap() else {
            panic!("sync_now must reply");
        };
        assert_eq!(value["ok"], json!(true));
    }

    #[test]
    fn get_status_lists_actor_states() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(&dir);
        let Dispatched::Reply(value) = dispatch(&ctx, methods::GET_STATUS, &json!({})).unwrap()
        else {
            panic!("get_status must reply");
        };
        assert_eq!(value["accounts"], json!([]));
    }

    /// Fake-server plumbing for the `test_connection` dispatch tests.
    mod test_connection {
        use super::*;

        /// A minimal IMAP speaker: greets, then answers every line; LOGIN
        /// gets `NO` unless `login_ok`.
        async fn spawn_fake_imap(login_ok: bool) -> u16 {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    tokio::spawn(async move {
                        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
                        let (reader, mut writer) = tokio::io::split(stream);
                        let _ = writer.write_all(b"* OK fake imap ready\r\n").await;
                        let mut lines = BufReader::new(reader).lines();
                        // async-imap writes the tag and command in separate
                        // writes; only a line-buffered fake sees them joined.
                        while let Ok(Some(line)) = lines.next_line().await {
                            let tag = line.split_whitespace().next().unwrap_or("A1");
                            let answer = if line.to_lowercase().contains("login") {
                                if login_ok {
                                    format!("{tag} OK logged in")
                                } else {
                                    format!("{tag} NO invalid credentials")
                                }
                            } else if line.to_lowercase().contains("list") {
                                // The probe now enumerates mailboxes (PRD
                                // F1.4); an empty LIST fails it.
                                format!(
                                    "* LIST (\\HasNoChildren) \"/\" INBOX\r\n{tag} OK list done"
                                )
                            } else {
                                format!("{tag} OK done")
                            };
                            let _ = writer.write_all(format!("{answer}\r\n").as_bytes()).await;
                        }
                    });
                }
            });
            port
        }

        /// A minimal SMTP speaker: greets with `greeting`, answers EHLO with a
        /// final `250 ` line, closes on QUIT.
        async fn spawn_fake_smtp(greeting: &'static str) -> u16 {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    tokio::spawn(async move {
                        use tokio::io::{AsyncReadExt, AsyncWriteExt};
                        let mut buf = [0u8; 1024];
                        let _ = stream.write_all(format!("{greeting}\r\n").as_bytes()).await;
                        loop {
                            let Ok(n) = stream.read(&mut buf).await else {
                                return;
                            };
                            if n == 0 {
                                return;
                            }
                            let line = String::from_utf8_lossy(&buf[..n]).to_uppercase();
                            if line.contains("EHLO") {
                                let _ =
                                    stream.write_all(b"250-fake.local\r\n250 SIZE 42\r\n").await;
                            } else if line.contains("QUIT") {
                                let _ = stream.write_all(b"221 bye\r\n").await;
                                return;
                            }
                        }
                    });
                }
            });
            port
        }

        /// The `test_connection` payload aimed at the fake servers.
        /// Uses `tls` because `plaintext` is rejected at the dispatch boundary
        /// (credentials must never travel in cleartext).
        fn conn_params(imap_port: u16, smtp_port: u16) -> Value {
            json!({
                "conn": {
                    "email": "work@example.com",
                    "imap_host": "127.0.0.1",
                    "imap_port": imap_port,
                    "security": "tls",
                    "smtp_host": "127.0.0.1",
                    "smtp_port": smtp_port,
                    "smtp_security": "tls"
                },
                "creds": { "password": "secret-pw" }
            })
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test_connection_reports_both_steps_ok() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = test_ctx(&dir);
            let imap = spawn_fake_imap(true).await;
            let smtp = spawn_fake_smtp("220 fake smtp ready").await;
            let Dispatched::Reply(value) =
                dispatch(&ctx, methods::TEST_CONNECTION, &conn_params(imap, smtp)).unwrap()
            else {
                panic!("test_connection must reply");
            };
            // Fake servers are plain TCP; TLS is mandatory, so both probes fail
            // at the TLS handshake. This verifies plaintext is rejected and
            // TLS is required for test_connection.
            assert_eq!(value["imap"]["ok"], json!(false));
            assert_eq!(value["smtp"]["ok"], json!(false));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn test_connection_reports_imap_failure_and_smtp_ok() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = test_ctx(&dir);
            let imap = spawn_fake_imap(false).await;
            let smtp = spawn_fake_smtp("220 fake smtp ready").await;
            let Dispatched::Reply(value) =
                dispatch(&ctx, methods::TEST_CONNECTION, &conn_params(imap, smtp)).unwrap()
            else {
                panic!("test_connection must reply");
            };
            // Both probes fail at TLS handshake (fake servers are plain TCP).
            assert_eq!(value["imap"]["ok"], json!(false));
            assert_eq!(value["smtp"]["ok"], json!(false));
        }

        #[test]
        fn test_connection_requires_conn_and_creds() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = test_ctx(&dir);
            let err = dispatch(&ctx, methods::TEST_CONNECTION, &json!({})).unwrap_err();
            assert_eq!(err.kind, ErrorKind::Validation);
        }

        #[test]
        fn test_connection_rejects_unknown_security() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = test_ctx(&dir);
            let mut params = conn_params(1, 1);
            params["conn"]["security"] = json!("carrier-pigeon");
            let err = dispatch(&ctx, methods::TEST_CONNECTION, &params).unwrap_err();
            assert_eq!(err.kind, ErrorKind::Validation);
        }
    }

    mod spawn_on_add {
        use super::*;

        #[tokio::test(flavor = "multi_thread")]
        async fn add_imap_account_spawns_a_sync_actor() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = test_ctx(&dir);
            let Dispatched::Reply(value) =
                dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap()
            else {
                panic!("add_imap_account must reply");
            };
            let account_id = value["account_id"].as_i64().expect("account_id");
            assert!(
                ctx.sync
                    .lock()
                    .expect("sync handle lock")
                    .contains_key(&account_id),
                "adding an account must spawn its sync actor"
            );
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn ensure_sync_actor_is_idempotent() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = test_ctx(&dir);
            let Dispatched::Reply(value) =
                dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap()
            else {
                panic!("add_imap_account must reply");
            };
            let account_id = value["account_id"].as_i64().expect("account_id");
            ensure_sync_actor(&ctx, account_id).unwrap();
            ensure_sync_actor(&ctx, account_id).unwrap();
            assert_eq!(ctx.sync.lock().expect("sync handle lock").len(), 1);
        }

        #[test]
        fn ensure_sync_actor_reports_missing_rows() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = test_ctx(&dir);
            let err = ensure_sync_actor(&ctx, 999).unwrap_err();
            assert_eq!(err.kind, ErrorKind::NotFound);
            assert!(err.message.contains("999"));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn remove_account_stops_the_sync_actor() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = test_ctx(&dir);
            let Dispatched::Reply(added) =
                dispatch(&ctx, methods::ADD_IMAP_ACCOUNT, &add_params()).unwrap()
            else {
                panic!("add_imap_account must reply");
            };
            let account_id = added["account_id"].as_i64().expect("account_id");
            assert!(
                ctx.sync
                    .lock()
                    .expect("sync handle lock")
                    .contains_key(&account_id)
            );

            dispatch(
                &ctx,
                methods::REMOVE_ACCOUNT,
                &json!({ "account_id": account_id }),
            )
            .unwrap();

            assert!(
                !ctx.sync
                    .lock()
                    .expect("sync handle lock")
                    .contains_key(&account_id),
                "removing an account must stop its sync actor"
            );
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn search_dispatch_returns_hits_for_account() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = test_ctx(&dir);
            let account_id = 42;
            // Seed account, message + FTS rows directly (writer normally keeps these in sync).
            let db_path = dir.path().join(DB_FILE_NAME);
            let conn = mailslate_core::infra::db::open_rw(&db_path).unwrap();
            conn.execute(
                "INSERT INTO accounts (id, display_name, email, provider, imap_host, imap_port,
                                       imap_security, smtp_host, smtp_port, smtp_security,
                                       auth_type, created_at)
                 VALUES (?1, 'Work', 'work@example.com', 'imap', 'imap.example.com', 993,
                         'tls', 'smtp.example.com', 465, 'tls', 'password', 0)",
                rusqlite::params![account_id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (id, account_id, dedupe_key, subject, date_utc)
                 VALUES (?1, ?2, 'msg-1', 'Quarterly report', 1000)",
                rusqlite::params![1, account_id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages_fts_content (message_id, subject, from_text, to_text, body)
                 VALUES (?1, 'Quarterly report', 'ana', 'bo', 'the budgets look fine')",
                rusqlite::params![1],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages_fts (rowid, subject, from_text, to_text, body)
                 VALUES (?1, 'Quarterly report', 'ana', 'bo', 'the budgets look fine')",
                rusqlite::params![1],
            )
            .unwrap();
            drop(conn);

            let Dispatched::Reply(value) = dispatch(
                &ctx,
                methods::SEARCH,
                &json!({ "account_id": account_id, "query": "budgets" }),
            )
            .unwrap() else {
                panic!("search must reply");
            };
            let messages = value["messages"].as_array().expect("messages array");
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0]["id"], json!(1));
            assert_eq!(messages[0]["account_id"], json!(42));
            assert_eq!(messages[0]["subject"], json!("Quarterly report"));
            assert_eq!(value["total"], json!(1));
        }

        #[test]
        fn search_dispatch_missing_account_id_is_validation() {
            let dir = tempfile::tempdir().unwrap();
            let ctx = test_ctx(&dir);
            let err = dispatch(&ctx, methods::SEARCH, &json!({ "query": "budgets" })).unwrap_err();
            assert_eq!(err.kind, ErrorKind::Validation);
            assert!(err.message.contains("account_id"));
        }
    }
}
