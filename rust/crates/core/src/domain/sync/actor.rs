//! Per-account sync actor: owns the connection lifecycle, runs the
//! initial sync, and serves SyncNow / Shutdown commands (ARCHITECTURE §9).
//!
//! The daemon spawns one actor per configured account and holds its
//! [`SyncHandle`]; dispatch forwards `sync_now` requests and reads
//! `current_state` for `get_status`.

use std::sync::Arc;
use std::sync::Mutex;

use crate::domain::accounts::Security;
use crate::domain::events::EventBus;
use crate::domain::sync::engine::SyncState;
use crate::domain::sync::initial::initial_sync;
use crate::infra::db::writer::WriterActor;
use crate::infra::imap::ImapError;
use crate::infra::imap::connect::{connect, login_password, login_xoauth2};
use crate::infra::keyring::{KeyStore, imap_password_key};
use crate::infra::oauth::OAuthError;
use crate::infra::oauth::grant::{self, TokenPoster};
use crate::infra::oauth::providers::{self, Provider};
use crate::infra::oauth::tokens;
use mailslate_protocol::message::Event;
use serde_json::json;
use tokio::sync::mpsc;

/// Commands the actor serves after the initial sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncCmd {
    /// Re-run the sync on the live session.
    SyncNow,
    /// Drop the session and stop the task.
    Shutdown,
}

/// Connection + auth facts for one account, as the daemon reads them
/// from the accounts row and the keyring.
#[derive(Debug, Clone)]
pub struct AccountConfig {
    pub id: i64,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: Security,
    /// "password" or "oauth2" (schema CHECK).
    pub auth_type: String,
    /// DB provider ("gmail" / "outlook" / "imap") — picks the token
    /// endpoint for oauth2 refresh.
    pub provider: String,
}

/// Handle to a running sync actor: sends commands, reads the last
/// published state.
pub struct SyncHandle {
    tx: mpsc::Sender<SyncCmd>,
    state: Arc<Mutex<SyncState>>,
    /// Flipped by [`SyncHandle::try_shutdown`] so an in-flight sync
    /// bails out between mailboxes and fetch chunks instead of
    /// running to completion for an account that was just removed.
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl SyncHandle {
    /// The actor's most recent state (for `get_status`).
    pub fn current_state(&self) -> SyncState {
        self.state.lock().expect("state lock").clone()
    }

    /// Enqueues a re-sync request; false when the actor is gone or
    /// its queue is full (sync dispatch context — no blocking send).
    pub fn try_sync_now(&self) -> bool {
        self.tx.try_send(SyncCmd::SyncNow).is_ok()
    }

    /// Enqueues a shutdown request and flags any in-flight sync to
    /// bail out; false when the actor is gone.
    pub fn try_shutdown(&self) -> bool {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.tx.try_send(SyncCmd::Shutdown).is_ok()
    }
}

/// Why an OAuth refresh failed: the user must re-authorize, or the
/// failure is transient and a retry may succeed.
enum RefreshError {
    Reauth,
    Transient(String),
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_secs() as i64
}

/// Refreshes a stale token snapshot against the account's provider,
/// persists the result, and returns it. `InvalidGrant` (revoked or
/// expired refresh token) maps to re-auth; everything else is
/// transient (network, config, keyring).
async fn refresh_oauth(
    account: &AccountConfig,
    keystore: &Arc<dyn KeyStore>,
    token_poster: &Arc<dyn TokenPoster>,
) -> Result<tokens::TokenSnapshot, RefreshError> {
    let provider = match account.provider.as_str() {
        "gmail" => Provider::Google,
        "outlook" => Provider::Microsoft,
        // oauth2 on a generic imap account is a configuration error.
        _ => return Err(RefreshError::Reauth),
    };
    let client_id = providers::client_id(provider).map_err(|_| RefreshError::Reauth)?;
    let stale = match tokens::load(keystore.as_ref(), account.id) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => return Err(RefreshError::Reauth),
        // A keyring backend error is transient; retry rather than
        // parking the account.
        Err(err) => return Err(RefreshError::Transient(err.to_string())),
    };
    let refreshed = grant::refresh(
        token_poster.as_ref(),
        provider,
        &stale.refresh_token,
        &client_id,
        now_unix(),
    )
    .await
    .map_err(|err| match err {
        OAuthError::InvalidGrant(_) => RefreshError::Reauth,
        other => RefreshError::Transient(other.to_string()),
    })?;
    tokens::save(keystore.as_ref(), account.id, &refreshed)
        .map_err(|err| RefreshError::Transient(err.to_string()))?;
    Ok(refreshed)
}

/// Initial retry delay after a failed attempt (PRD F2.6).
const INITIAL_BACKOFF: std::time::Duration = std::time::Duration::from_secs(1);
/// Upper bound for retry delays (PRD F2.6: cap 60 s).
const MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(60);
/// Ceiling for one IMAP connect or login step (QA round 1: no hang
/// without a timeout).
const IMAP_STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Ceiling for a best-effort logout before reconnecting.
const LOGOUT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Doubles the previous delay and adds clock-derived jitter, capped at
/// [`MAX_BACKOFF`] (PRD F2.6: exponential backoff with jitter, cap 60 s).
fn next_backoff(prev: std::time::Duration) -> std::time::Duration {
    let doubled = prev.saturating_mul(2).min(MAX_BACKOFF);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .subsec_nanos() as u64;
    let span = (doubled.as_nanos() as u64 / 4).max(1);
    doubled
        .saturating_add(std::time::Duration::from_nanos(nanos % span))
        .min(MAX_BACKOFF)
}

/// How one connect → authenticate → initial-sync → serve cycle ended.
enum CycleOutcome {
    /// Shutdown requested, or every handle dropped.
    Shutdown,
    /// Re-authentication required: terminal, no retries (PRD F2.6).
    Reauth,
    /// Transient failure carrying the reason: retry after backoff.
    Retry(String),
}

/// How a backoff wait ended.
enum BackoffWait {
    /// The delay elapsed, or a SyncNow command cut it short.
    Proceed,
    /// Shutdown requested, or every handle dropped.
    Shutdown,
}

/// Waits out the backoff delay while staying responsive to commands:
/// SyncNow cancels the wait (user-requested re-sync retries at once),
/// Shutdown ends the actor (PRD F2.6).
async fn wait_backoff(rx: &mut mpsc::Receiver<SyncCmd>, delay: std::time::Duration) -> BackoffWait {
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);
    tokio::select! {
        _ = &mut sleep => BackoffWait::Proceed,
        cmd = rx.recv() => match cmd {
            Some(SyncCmd::Shutdown) | None => BackoffWait::Shutdown,
            // A user-requested re-sync cuts the backoff short.
            Some(SyncCmd::SyncNow) => BackoffWait::Proceed,
        },
    }
}

/// Runs one full account cycle: connect, authenticate, initial sync,
/// then serve SyncNow commands on the live session. Returns how the
/// cycle ended; the supervision loop in [`spawn_actor`] decides what
/// to do next.
///
/// `backoff` is the supervision loop's current retry delay; it is
/// reset to [`INITIAL_BACKOFF`] once the cycle reaches the serve loop
/// (the account is healthy), so a later transient failure starts from
/// the initial delay instead of the previously accumulated one.
#[allow(clippy::too_many_arguments)]
async fn run_account_cycle(
    account: &AccountConfig,
    keystore: &Arc<dyn KeyStore>,
    writer: &Arc<WriterActor>,
    bus: &Arc<EventBus>,
    token_poster: &Arc<dyn TokenPoster>,
    rx: &mut mpsc::Receiver<SyncCmd>,
    set_state: &(dyn Fn(SyncState) + Send + Sync),
    backoff: &mut std::time::Duration,
    stop: &std::sync::atomic::AtomicBool,
) -> CycleOutcome {
    // Polled between mailboxes and fetch chunks: a removed account's
    // in-flight sync must not keep fetching (and FK-failing writes)
    // until the queued Shutdown is finally observed between cycles.
    let stopping = || stop.load(std::sync::atomic::Ordering::Relaxed);
    set_state(SyncState::Connecting);
    let client = match tokio::time::timeout(
        IMAP_STEP_TIMEOUT,
        connect(&account.imap_host, account.imap_port, account.imap_security),
    )
    .await
    {
        Ok(Ok(client)) => client,
        Ok(Err(err)) => return CycleOutcome::Retry(err.to_string()),
        Err(_) => return CycleOutcome::Retry("imap connect timed out".into()),
    };

    set_state(SyncState::Authenticating);
    let mut session = if account.auth_type == "oauth2" {
        let mut auth = match tokens::load(keystore.as_ref(), account.id) {
            Ok(Some(snapshot)) => snapshot,
            // A missing snapshot means the account was never
            // authorized (or access was revoked): terminal re-auth.
            Ok(None) => return CycleOutcome::Reauth,
            // A keyring backend error is transient (D-Bus hiccup,
            // locked keyring): retry, do not park the account.
            Err(err) => return CycleOutcome::Retry(err.to_string()),
        };
        if !auth.is_fresh(now_unix()) {
            auth = match refresh_oauth(account, keystore, token_poster).await {
                Ok(snapshot) => snapshot,
                Err(RefreshError::Reauth) => return CycleOutcome::Reauth,
                Err(RefreshError::Transient(detail)) => return CycleOutcome::Retry(detail),
            };
        }
        match tokio::time::timeout(
            IMAP_STEP_TIMEOUT,
            login_xoauth2(client, &account.email, &auth.access_token),
        )
        .await
        {
            Ok(Ok(session)) => session,
            // Auth errors stop retrying and raise re-auth (PRD F2.6);
            // transport failures stay transient.
            Ok(Err(ImapError::Auth(_))) => return CycleOutcome::Reauth,
            Ok(Err(err)) => return CycleOutcome::Retry(err.to_string()),
            Err(_) => return CycleOutcome::Retry("xoauth2 login timed out".into()),
        }
    } else {
        let password = match keystore.get(&imap_password_key(account.id)) {
            Ok(Some(password)) => password,
            // No stored password means the account was never set up
            // (or was revoked): terminal re-auth.
            Ok(None) => return CycleOutcome::Reauth,
            // A keyring backend error is transient: retry, do not
            // park the account.
            Err(err) => return CycleOutcome::Retry(err.to_string()),
        };
        match tokio::time::timeout(
            IMAP_STEP_TIMEOUT,
            login_password(client, &account.email, &password),
        )
        .await
        {
            Ok(Ok(session)) => session,
            // Auth errors stop retrying and raise re-auth (PRD F2.6);
            // transport failures stay transient.
            Ok(Err(ImapError::Auth(_))) => return CycleOutcome::Reauth,
            Ok(Err(err)) => return CycleOutcome::Retry(err.to_string()),
            Err(_) => return CycleOutcome::Retry("imap login timed out".into()),
        }
    };

    set_state(SyncState::InitialSync);
    if let Err(err) = initial_sync(&mut session, writer, bus, account.id, &stopping).await {
        let _ = tokio::time::timeout(LOGOUT_TIMEOUT, session.logout()).await;
        return CycleOutcome::Retry(err.to_string());
    }
    set_state(SyncState::Idle);
    // The account is healthy: reset the accumulated backoff so the
    // next transient failure starts from the initial delay again.
    *backoff = INITIAL_BACKOFF;

    loop {
        match rx.recv().await {
            Some(SyncCmd::SyncNow) => {
                set_state(SyncState::Busy);
                if let Err(err) =
                    initial_sync(&mut session, writer, bus, account.id, &stopping).await
                {
                    let _ = tokio::time::timeout(LOGOUT_TIMEOUT, session.logout()).await;
                    return CycleOutcome::Retry(err.to_string());
                }
                set_state(SyncState::Idle);
            }
            Some(SyncCmd::Shutdown) | None => {
                let _ = tokio::time::timeout(LOGOUT_TIMEOUT, session.logout()).await;
                return CycleOutcome::Shutdown;
            }
        }
    }
}

/// Spawns the sync actor for one account under a supervision loop:
/// transient failures retry with exponential backoff and jitter
/// (PRD F2.6), authentication failures park in `reauth_required` until
/// shutdown, and every state change publishes an `account_status`
/// event.
pub fn spawn_actor(
    account: AccountConfig,
    keystore: Arc<dyn KeyStore>,
    writer: Arc<WriterActor>,
    bus: Arc<EventBus>,
    token_poster: Arc<dyn TokenPoster>,
) -> SyncHandle {
    let (tx, mut rx) = mpsc::channel(16);
    let state = Arc::new(Mutex::new(SyncState::Disconnected));
    let state_for_task = Arc::clone(&state);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_for_task = Arc::clone(&stop);

    tokio::spawn(async move {
        let set_state = |next: SyncState| {
            *state_for_task.lock().expect("state lock") = next.clone();
            bus.publish(Event::new(
                "account_status",
                json!({"account": account.id, "state": next.as_str()}),
            ));
        };

        let mut backoff = INITIAL_BACKOFF;
        loop {
            match run_account_cycle(
                &account,
                &keystore,
                &writer,
                &bus,
                &token_poster,
                &mut rx,
                &set_state,
                &mut backoff,
                &stop_for_task,
            )
            .await
            {
                CycleOutcome::Shutdown => break,
                CycleOutcome::Reauth => {
                    set_state(SyncState::ReauthRequired);
                    // Terminal: park until a shutdown arrives.
                    while let Some(cmd) = rx.recv().await {
                        if matches!(cmd, SyncCmd::Shutdown) {
                            break;
                        }
                    }
                    break;
                }
                CycleOutcome::Retry(detail) => {
                    set_state(SyncState::Backoff(detail));
                    match wait_backoff(&mut rx, backoff).await {
                        BackoffWait::Shutdown => break,
                        BackoffWait::Proceed => {}
                    }
                    backoff = next_backoff(backoff);
                }
            }
        }
        set_state(SyncState::Stopped);
    });

    SyncHandle { tx, state, stop }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::imap::testing::spawn_fake;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn seeded_db(dir: &tempfile::TempDir, auth_type: &str, provider: &str) -> std::path::PathBuf {
        let path = dir.path().join(crate::infra::db::DB_FILE_NAME);
        let mut conn = crate::infra::db::open_rw(&path).expect("open");
        crate::infra::db::migrations::run(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO accounts(id, display_name, email, provider, imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, auth_type, created_at)
             VALUES (1, 'Seed', 'seed@example.com', ?1, '127.0.0.1', 143, 'tls', 'smtp.example.com', 465, 'tls', ?2, 0)",
            [provider, auth_type],
        )
        .expect("seed account");
        drop(conn);
        path
    }

    async fn wait_state(handle: &SyncHandle, target: &str, timeout: std::time::Duration) {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if handle.current_state().as_str() == target {
                return;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "state never became {target}, at {:?}",
                    handle.current_state()
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn actor_connects_syncs_and_serves_commands() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "password", "imap");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let fetches = std::sync::Arc::new(AtomicUsize::new(0));
        let fetches_for_fake = std::sync::Arc::clone(&fetches);
        let port = spawn_fake(move |tag: &str, command: &str| {
            if command.starts_with("LOGIN") {
                vec![format!("{tag} OK logged in")]
            } else if command.starts_with("LIST") {
                vec![
                    "* LIST (\\HasNoChildren) \"/\" INBOX".to_string(),
                    format!("{tag} OK list done"),
                ]
            } else if command.starts_with("SELECT") {
                vec![
                    "* 1 EXISTS".to_string(),
                    "* OK [UIDVALIDITY 100] UIDs valid".to_string(),
                    "* OK [UIDNEXT 2] Predicted".to_string(),
                    format!("{tag} OK select done"),
                ]
            } else if command.starts_with("UID FETCH") {
                fetches_for_fake.fetch_add(1, Ordering::SeqCst);
                let headers = "Message-ID: <a@x>\r\nSubject: Hello\r\nFrom: A <a@x>\r\nDate: Mon, 19 Oct 2026 00:00:00 +0000\r\n\r\n";
                vec![
                    format!(
                        "* 1 FETCH (UID 1 FLAGS (\\Seen) RFC822.SIZE 100 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\" BODY[HEADER.FIELDS (DATE FROM TO CC SUBJECT REPLY-TO MESSAGE-ID REFERENCES IN-REPLY-TO)] {{{}}}\r\n{})",
                        headers.len(),
                        headers
                    ),
                    format!("{tag} OK fetch done"),
                ]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        keystore
            .store(&crate::infra::keyring::imap_password_key(1), "pass")
            .expect("store password");

        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());
        let mut feed = bus.subscribe();

        let handle = spawn_actor(
            AccountConfig {
                id: 1,
                email: "seed@example.com".into(),
                imap_host: "127.0.0.1".into(),
                imap_port: port,
                imap_security: crate::domain::accounts::Security::Plaintext,
                auth_type: "password".into(),
                provider: "imap".into(),
            },
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(&handle, "idle", std::time::Duration::from_secs(10)).await;
        assert_eq!(fetches.load(Ordering::SeqCst), 1, "one initial sync round");

        handle.try_sync_now();
        // The state is already idle from the first round; poll the
        // counter until the re-sync's UID FETCH lands.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while fetches.load(Ordering::SeqCst) < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "sync_now never re-synced"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        wait_state(&handle, "idle", std::time::Duration::from_secs(10)).await;

        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;

        let mut statuses = Vec::new();
        while let Ok(event) = feed.try_recv() {
            if event.event == "account_status" {
                statuses.push(event);
            }
        }
        assert!(
            statuses.iter().any(|e| e.data["state"] == "idle"),
            "an idle status must be published, got {:?}",
            statuses
        );
        assert!(
            statuses.iter().any(|e| e.data["state"] == "stopped"),
            "a stopped status must be published"
        );
    }

    /// A keyring whose backend errors on every call (D-Bus down,
    /// locked keyring): every secret lookup fails with `Err`.
    struct FailingKeyStore;

    impl crate::infra::keyring::KeyStore for FailingKeyStore {
        fn store(
            &self,
            _key: &str,
            _secret: &str,
        ) -> Result<(), crate::infra::keyring::KeyStoreError> {
            Err(crate::infra::keyring::KeyStoreError::Unavailable(
                "keyring down".into(),
            ))
        }

        fn get(&self, _key: &str) -> Result<Option<String>, crate::infra::keyring::KeyStoreError> {
            Err(crate::infra::keyring::KeyStoreError::Unavailable(
                "keyring down".into(),
            ))
        }

        fn delete(&self, _key: &str) -> Result<(), crate::infra::keyring::KeyStoreError> {
            Err(crate::infra::keyring::KeyStoreError::Unavailable(
                "keyring down".into(),
            ))
        }
    }

    /// A keyring backend error is transient: the actor must back off
    /// and retry, never park in reauth_required (password accounts
    /// have no reconnect path).
    #[tokio::test]
    async fn a_keyring_failure_backs_off_instead_of_parking_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "password", "imap");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let port = crate::infra::imap::testing::spawn_fake_multi(|tag, command| {
            happy_replies(tag, command)
        })
        .await;

        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            password_account(port),
            std::sync::Arc::new(FailingKeyStore)
                as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        let mut saw_backoff = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if handle.current_state().as_str() == "backoff" {
                saw_backoff += 1;
            }
            assert_ne!(handle.current_state().as_str(), "reauth_required");
            assert_ne!(handle.current_state().as_str(), "stopped");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(saw_backoff >= 2, "backoff must repeat, saw {saw_backoff}");

        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    /// An oauth account whose snapshot cannot be *read* from the
    /// keyring also retries — only a missing snapshot parks.
    #[tokio::test]
    async fn an_oauth_snapshot_load_failure_backs_off() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "oauth2", "gmail");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let port = crate::infra::imap::testing::spawn_fake_multi(|tag, command| {
            happy_replies(tag, command)
        })
        .await;

        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            oauth_account(port),
            std::sync::Arc::new(FailingKeyStore)
                as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        let mut saw_backoff = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if handle.current_state().as_str() == "backoff" {
                saw_backoff += 1;
            }
            assert_ne!(handle.current_state().as_str(), "reauth_required");
            assert_ne!(handle.current_state().as_str(), "stopped");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(saw_backoff >= 2, "backoff must repeat, saw {saw_backoff}");

        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    /// Reaching the serve loop resets the accumulated backoff: a later
    /// transient failure must start from the initial delay again.
    #[tokio::test]
    async fn a_successful_cycle_resets_the_backoff() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "password", "imap");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let port = crate::infra::imap::testing::spawn_fake(happy_replies).await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        keystore
            .store(&crate::infra::keyring::imap_password_key(1), "pass")
            .expect("store password");
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let (tx, mut rx) = mpsc::channel(16);
        tx.try_send(SyncCmd::Shutdown)
            .expect("queue the shutdown ahead of the serve loop");
        let mut backoff = MAX_BACKOFF;
        let set_state = |_next: SyncState| {};
        let outcome = run_account_cycle(
            &password_account(port),
            &(std::sync::Arc::clone(&keystore)
                as std::sync::Arc<dyn crate::infra::keyring::KeyStore>),
            &writer,
            &bus,
            &(std::sync::Arc::new(RefreshPoster)
                as std::sync::Arc<dyn crate::infra::oauth::grant::TokenPoster>),
            &mut rx,
            &set_state,
            &mut backoff,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .await;

        assert!(matches!(outcome, CycleOutcome::Shutdown));
        assert_eq!(
            backoff, INITIAL_BACKOFF,
            "a healthy cycle must reset the backoff"
        );
    }

    /// A token-endpoint fake that hands out a fresh access token.
    struct RefreshPoster;

    impl crate::infra::oauth::grant::TokenPoster for RefreshPoster {
        fn post<'a>(
            &'a self,
            _token_url: &'a str,
            _form: &'a [(&'a str, &'a str)],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            crate::infra::oauth::grant::TokenResponse,
                            crate::infra::oauth::OAuthError,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                Ok(crate::infra::oauth::grant::TokenResponse {
                    access_token: "at-refreshed".into(),
                    refresh_token: Some("rt-2".into()),
                    expires_in: 3600,
                    scope: None,
                    id_token: None,
                })
            })
        }
    }

    /// An oauth2 account with a stale snapshot refreshes before login
    /// and the refreshed token lands back in the keyring.
    #[tokio::test]
    async fn oauth2_actor_refreshes_stale_tokens_before_login() {
        let guard = crate::infra::oauth::providers::ENV_LOCK.lock().await;
        // SAFETY: serialized by ENV_LOCK; no concurrent readers of this
        // key in this test process.
        unsafe { std::env::set_var("MAILSLATE_GOOGLE_CLIENT_ID", "google-app-id") };

        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "oauth2", "gmail");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let port = spawn_fake(|tag: &str, command: &str| {
            if command.starts_with("AUTHENTICATE") {
                vec!["+ ".to_string()]
            } else if command.starts_with("LIST") {
                vec![
                    "* LIST (\\HasNoChildren) \"/\" INBOX".to_string(),
                    format!("{tag} OK list done"),
                ]
            } else if command.starts_with("SELECT") {
                vec!["* 0 EXISTS".to_string(), format!("{tag} OK select done")]
            } else if command.starts_with("UID FETCH") {
                vec![format!("{tag} OK fetch done")]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        crate::infra::oauth::tokens::save(
            keystore.as_ref(),
            1,
            &crate::infra::oauth::tokens::TokenSnapshot {
                access_token: "stale-token".into(),
                refresh_token: "rt-1".into(),
                expires_at: 1, // long expired
                scopes: String::new(),
            },
        )
        .expect("stage stale snapshot");

        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());
        let handle = spawn_actor(
            AccountConfig {
                id: 1,
                email: "me@gmail.com".into(),
                imap_host: "127.0.0.1".into(),
                imap_port: port,
                imap_security: crate::domain::accounts::Security::Plaintext,
                auth_type: "oauth2".into(),
                provider: "gmail".into(),
            },
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(&handle, "idle", std::time::Duration::from_secs(10)).await;

        let refreshed = crate::infra::oauth::tokens::load(keystore.as_ref(), 1)
            .expect("load refreshed")
            .expect("snapshot present");
        assert_eq!(refreshed.access_token, "at-refreshed");
        assert_eq!(refreshed.refresh_token, "rt-2");
        assert!(refreshed.expires_at > 1, "expiry must move forward");

        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;

        // SAFETY: as above.
        unsafe { std::env::remove_var("MAILSLATE_GOOGLE_CLIENT_ID") };
        drop(guard);
    }

    // --- Supervision-loop coverage (PRD F2.6, QA round 1) ---

    fn password_account(port: u16) -> AccountConfig {
        AccountConfig {
            id: 1,
            email: "seed@example.com".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: port,
            imap_security: crate::domain::accounts::Security::Plaintext,
            auth_type: "password".into(),
            provider: "imap".into(),
        }
    }

    fn oauth_account(port: u16) -> AccountConfig {
        AccountConfig {
            id: 1,
            email: "me@gmail.com".into(),
            imap_host: "127.0.0.1".into(),
            imap_port: port,
            imap_security: crate::domain::accounts::Security::Plaintext,
            auth_type: "oauth2".into(),
            provider: "gmail".into(),
        }
    }

    /// Canned answers for a fully working password session.
    fn happy_replies(tag: &str, command: &str) -> Vec<String> {
        if command.starts_with("LOGIN") {
            vec![format!("{tag} OK logged in")]
        } else if command.starts_with("LIST") {
            vec![
                "* LIST (\\HasNoChildren) \"/\" INBOX".to_string(),
                format!("{tag} OK list done"),
            ]
        } else if command.starts_with("SELECT") {
            vec![
                "* 1 EXISTS".to_string(),
                "* OK [UIDVALIDITY 100] UIDs valid".to_string(),
                "* OK [UIDNEXT 2] Predicted".to_string(),
                format!("{tag} OK select done"),
            ]
        } else if command.starts_with("UID FETCH") {
            let headers = "Message-ID: <a@x>\r\nSubject: Hello\r\nFrom: A <a@x>\r\nDate: Mon, 19 Oct 2026 00:00:00 +0000\r\n\r\n";
            vec![
                format!(
                    "* 1 FETCH (UID 1 FLAGS (\\Seen) RFC822.SIZE 100 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\" BODY[HEADER.FIELDS (DATE FROM TO CC SUBJECT REPLY-TO MESSAGE-ID REFERENCES IN-REPLY-TO)] {{{}}}\r\n{})",
                    headers.len(),
                    headers
                ),
                format!("{tag} OK fetch done"),
            ]
        } else {
            vec![format!("{tag} OK")]
        }
    }

    /// A token endpoint that always refuses the refresh (revoked).
    struct ReauthPoster;

    impl crate::infra::oauth::grant::TokenPoster for ReauthPoster {
        fn post<'a>(
            &'a self,
            _token_url: &'a str,
            _form: &'a [(&'a str, &'a str)],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            crate::infra::oauth::grant::TokenResponse,
                            crate::infra::oauth::OAuthError,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                Err(crate::infra::oauth::OAuthError::InvalidGrant(
                    "revoked".into(),
                ))
            })
        }
    }

    /// A token endpoint that is down (transient network failure).
    struct NetworkPoster;

    impl crate::infra::oauth::grant::TokenPoster for NetworkPoster {
        fn post<'a>(
            &'a self,
            _token_url: &'a str,
            _form: &'a [(&'a str, &'a str)],
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<
                            crate::infra::oauth::grant::TokenResponse,
                            crate::infra::oauth::OAuthError,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                Err(crate::infra::oauth::OAuthError::Network(
                    "endpoint down".into(),
                ))
            })
        }
    }

    fn stage_snapshot(keystore: &std::sync::Arc<crate::infra::keyring::FakeKeyStore>) {
        crate::infra::oauth::tokens::save(
            keystore.as_ref(),
            1,
            &crate::infra::oauth::tokens::TokenSnapshot {
                access_token: "stale-token".into(),
                refresh_token: "rt-1".into(),
                expires_at: 1, // long expired
                scopes: String::new(),
            },
        )
        .expect("stage stale snapshot");
    }

    /// A refused TCP port fails fast: 127.0.0.1:1 never listens.
    #[tokio::test]
    async fn connect_failure_backs_off_and_still_shuts_down() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "password", "imap");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));
        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        keystore
            .store(&crate::infra::keyring::imap_password_key(1), "pass")
            .expect("store password");
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            password_account(1),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(&handle, "backoff", std::time::Duration::from_secs(10)).await;
        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn retries_after_a_dropped_connection() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "password", "imap");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let logins = std::sync::Arc::new(AtomicUsize::new(0));
        let logins_for_fake = std::sync::Arc::clone(&logins);
        let port = crate::infra::imap::testing::spawn_fake_multi(move |tag, command| {
            if command.starts_with("LOGIN") && logins_for_fake.fetch_add(1, Ordering::SeqCst) == 0 {
                return vec![]; // drop the first connection
            }
            happy_replies(tag, command)
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        keystore
            .store(&crate::infra::keyring::imap_password_key(1), "pass")
            .expect("store password");
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            password_account(port),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(&handle, "idle", std::time::Duration::from_secs(10)).await;
        assert_eq!(logins.load(Ordering::SeqCst), 2, "second attempt logs in");
        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn missing_password_raises_reauth_and_stays() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "password", "imap");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let port = crate::infra::imap::testing::spawn_fake_multi(|tag, command| {
            happy_replies(tag, command)
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            password_account(port),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(
            &handle,
            "reauth_required",
            std::time::Duration::from_secs(10),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert_eq!(
            handle.current_state().as_str(),
            "reauth_required",
            "reauth is terminal, not retried"
        );
        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn rejected_login_raises_reauth_and_stays() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "password", "imap");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let port = crate::infra::imap::testing::spawn_fake_multi(|tag, command| {
            if command.starts_with("LOGIN") {
                return vec![format!("{tag} NO authentication failed")];
            }
            happy_replies(tag, command)
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        keystore
            .store(&crate::infra::keyring::imap_password_key(1), "pass")
            .expect("store password");
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            password_account(port),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(
            &handle,
            "reauth_required",
            std::time::Duration::from_secs(10),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert_eq!(handle.current_state().as_str(), "reauth_required");
        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn missing_oauth_snapshot_raises_reauth_and_stays() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "oauth2", "gmail");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let port = crate::infra::imap::testing::spawn_fake_multi(|tag, command| {
            happy_replies(tag, command)
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            oauth_account(port),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(
            &handle,
            "reauth_required",
            std::time::Duration::from_secs(10),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert_eq!(handle.current_state().as_str(), "reauth_required");
        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn revoked_refresh_token_raises_reauth_and_stays() {
        let guard = crate::infra::oauth::providers::ENV_LOCK.lock().await;
        // SAFETY: serialized by ENV_LOCK; no concurrent readers of this
        // key in this test process.
        unsafe { std::env::set_var("MAILSLATE_GOOGLE_CLIENT_ID", "google-app-id") };

        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "oauth2", "gmail");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let port = crate::infra::imap::testing::spawn_fake_multi(|tag, command| {
            happy_replies(tag, command)
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        stage_snapshot(&keystore);
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            oauth_account(port),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(ReauthPoster),
        );

        wait_state(
            &handle,
            "reauth_required",
            std::time::Duration::from_secs(10),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert_eq!(handle.current_state().as_str(), "reauth_required");
        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;

        // SAFETY: as above.
        unsafe { std::env::remove_var("MAILSLATE_GOOGLE_CLIENT_ID") };
        drop(guard);
    }

    #[tokio::test]
    async fn transient_refresh_failure_backs_off_and_keeps_retrying() {
        let guard = crate::infra::oauth::providers::ENV_LOCK.lock().await;
        // SAFETY: serialized by ENV_LOCK; no concurrent readers of this
        // key in this test process.
        unsafe { std::env::set_var("MAILSLATE_GOOGLE_CLIENT_ID", "google-app-id") };

        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "oauth2", "gmail");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let port = crate::infra::imap::testing::spawn_fake_multi(|tag, command| {
            happy_replies(tag, command)
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        stage_snapshot(&keystore);
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            oauth_account(port),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(NetworkPoster),
        );

        // The refresh keeps failing transiently, so the actor must keep
        // cycling through backoff without ever going reauth or stopped.
        let mut saw_backoff = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if handle.current_state().as_str() == "backoff" {
                saw_backoff += 1;
            }
            assert_ne!(handle.current_state().as_str(), "reauth_required");
            assert_ne!(handle.current_state().as_str(), "stopped");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(saw_backoff >= 2, "backoff must repeat, saw {saw_backoff}");

        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;

        // SAFETY: as above.
        unsafe { std::env::remove_var("MAILSLATE_GOOGLE_CLIENT_ID") };
        drop(guard);
    }

    #[tokio::test]
    async fn rejected_xoauth2_login_raises_reauth_and_stays() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "oauth2", "gmail");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let sasl = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sasl_for_fake = std::sync::Arc::clone(&sasl);
        let port = crate::infra::imap::testing::spawn_fake_multi(move |tag, command| {
            if command.starts_with("AUTHENTICATE") {
                sasl_for_fake.store(true, Ordering::SeqCst);
                return vec!["+ ".to_string()];
            }
            if sasl_for_fake.swap(false, Ordering::SeqCst) {
                // SASL continuation answer: refuse it.
                return vec![format!("{tag} NO authentication failed")];
            }
            happy_replies(tag, command)
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        crate::infra::oauth::tokens::save(
            keystore.as_ref(),
            1,
            &crate::infra::oauth::tokens::TokenSnapshot {
                access_token: "fresh-token".into(),
                refresh_token: "rt-1".into(),
                expires_at: now_unix() + 3600, // fresh: no refresh round-trip
                scopes: String::new(),
            },
        )
        .expect("stage fresh snapshot");
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            oauth_account(port),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(
            &handle,
            "reauth_required",
            std::time::Duration::from_secs(10),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert_eq!(handle.current_state().as_str(), "reauth_required");
        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn initial_sync_failure_backs_off_and_retries() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "password", "imap");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let selects = std::sync::Arc::new(AtomicUsize::new(0));
        let selects_for_fake = std::sync::Arc::clone(&selects);
        let port = crate::infra::imap::testing::spawn_fake_multi(move |tag, command| {
            if command.starts_with("SELECT") && selects_for_fake.fetch_add(1, Ordering::SeqCst) == 0
            {
                // Drop the connection while SELECT is in flight: a
                // transport failure must back off and retry. (A `NO`
                // refusal is skipped per mailbox instead - initial.rs.)
                return vec![];
            }
            happy_replies(tag, command)
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        keystore
            .store(&crate::infra::keyring::imap_password_key(1), "pass")
            .expect("store password");
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            password_account(port),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(&handle, "idle", std::time::Duration::from_secs(10)).await;
        assert_eq!(selects.load(Ordering::SeqCst), 2, "second attempt selects");
        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn sync_now_failure_reconnects_and_resyncs() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "password", "imap");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let fetches = std::sync::Arc::new(AtomicUsize::new(0));
        let fetches_for_fake = std::sync::Arc::clone(&fetches);
        let selects = std::sync::Arc::new(AtomicUsize::new(0));
        let selects_for_fake = std::sync::Arc::clone(&selects);
        let port = crate::infra::imap::testing::spawn_fake_multi(move |tag, command| {
            if command.starts_with("SELECT") && selects_for_fake.fetch_add(1, Ordering::SeqCst) == 1
            {
                // Second SELECT (the SyncNow re-sync) loses its
                // connection; the actor must reconnect and try again.
                return vec![];
            }
            if command.starts_with("UID FETCH") {
                fetches_for_fake.fetch_add(1, Ordering::SeqCst);
            }
            happy_replies(tag, command)
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        keystore
            .store(&crate::infra::keyring::imap_password_key(1), "pass")
            .expect("store password");
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            password_account(port),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(&handle, "idle", std::time::Duration::from_secs(10)).await;
        handle.try_sync_now();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while fetches.load(Ordering::SeqCst) < 2 {
            assert!(
                std::time::Instant::now() < deadline,
                "re-sync never completed"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        wait_state(&handle, "idle", std::time::Duration::from_secs(10)).await;
        assert!(selects.load(Ordering::SeqCst) >= 3, "third cycle selects");
        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn sync_now_during_backoff_retries_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir, "password", "imap");
        let writer = std::sync::Arc::new(crate::infra::db::writer::WriterActor::spawn(&db_path));

        let logins = std::sync::Arc::new(AtomicUsize::new(0));
        let logins_for_fake = std::sync::Arc::clone(&logins);
        let port = crate::infra::imap::testing::spawn_fake_multi(move |tag, command| {
            if command.starts_with("LOGIN") && logins_for_fake.fetch_add(1, Ordering::SeqCst) == 0 {
                return vec![]; // drop the first connection
            }
            happy_replies(tag, command)
        })
        .await;

        let keystore = std::sync::Arc::new(crate::infra::keyring::FakeKeyStore::new());
        keystore
            .store(&crate::infra::keyring::imap_password_key(1), "pass")
            .expect("store password");
        let bus = std::sync::Arc::new(crate::domain::events::EventBus::new());

        let handle = spawn_actor(
            password_account(port),
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
            std::sync::Arc::new(RefreshPoster),
        );

        wait_state(&handle, "backoff", std::time::Duration::from_secs(10)).await;
        let started = std::time::Instant::now();
        handle.try_sync_now();
        wait_state(&handle, "idle", std::time::Duration::from_secs(10)).await;
        assert!(
            started.elapsed() < std::time::Duration::from_millis(800),
            "sync_now must cut the backoff short, took {:?}",
            started.elapsed()
        );
        handle.try_shutdown();
        wait_state(&handle, "stopped", std::time::Duration::from_secs(5)).await;
    }

    #[test]
    fn next_backoff_doubles_up_to_the_cap() {
        let mut backoff = INITIAL_BACKOFF;
        for _ in 0..10 {
            backoff = next_backoff(backoff);
            assert!(backoff <= MAX_BACKOFF);
        }
        assert_eq!(backoff, MAX_BACKOFF, "the delay must reach the cap");
    }

    #[test]
    fn next_backoff_adds_jitter_within_bounds() {
        for _ in 0..100 {
            let next = next_backoff(std::time::Duration::from_secs(1));
            assert!(next >= std::time::Duration::from_secs(2));
            assert!(next <= std::time::Duration::from_millis(2500));
        }
    }
}
