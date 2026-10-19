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
use crate::infra::imap::connect::{connect, login_password, login_xoauth2};
use crate::infra::keyring::{KeyStore, imap_password_key};
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
}

/// Handle to a running sync actor: sends commands, reads the last
/// published state.
pub struct SyncHandle {
    tx: mpsc::Sender<SyncCmd>,
    state: Arc<Mutex<SyncState>>,
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

    /// Enqueues a shutdown request; false when the actor is gone.
    pub fn try_shutdown(&self) -> bool {
        self.tx.try_send(SyncCmd::Shutdown).is_ok()
    }
}

/// Spawns the sync actor for one account. The task connects,
/// authenticates, runs the initial sync once, then serves commands;
/// every state change publishes an `account_status` event.
pub fn spawn_actor(
    account: AccountConfig,
    keystore: Arc<dyn KeyStore>,
    writer: Arc<WriterActor>,
    bus: Arc<EventBus>,
) -> SyncHandle {
    let (tx, mut rx) = mpsc::channel(16);
    let state = Arc::new(Mutex::new(SyncState::Disconnected));
    let state_for_task = Arc::clone(&state);

    tokio::spawn(async move {
        let set_state = |next: SyncState| {
            *state_for_task.lock().expect("state lock") = next.clone();
            bus.publish(Event::new(
                "account_status",
                json!({"account": account.id, "state": next.as_str()}),
            ));
        };

        set_state(SyncState::Connecting);
        let client =
            match connect(&account.imap_host, account.imap_port, account.imap_security).await {
                Ok(client) => client,
                Err(err) => {
                    set_state(SyncState::Backoff(err.to_string()));
                    return;
                }
            };

        set_state(SyncState::Authenticating);
        let mut session = if account.auth_type == "oauth2" {
            let auth = match tokens::load(keystore.as_ref(), account.id) {
                Ok(Some(snapshot)) => snapshot,
                _ => {
                    set_state(SyncState::ReauthRequired);
                    return;
                }
            };
            match login_xoauth2(client, &account.email, &auth.access_token).await {
                Ok(session) => session,
                Err(err) => {
                    set_state(SyncState::Backoff(err.to_string()));
                    return;
                }
            }
        } else {
            let password = match keystore.get(&imap_password_key(account.id)) {
                Ok(Some(password)) => password,
                _ => {
                    set_state(SyncState::ReauthRequired);
                    return;
                }
            };
            match login_password(client, &account.email, &password).await {
                Ok(session) => session,
                Err(err) => {
                    set_state(SyncState::Backoff(err.to_string()));
                    return;
                }
            }
        };

        set_state(SyncState::InitialSync);
        if let Err(err) = initial_sync(&mut session, &writer, &bus, account.id).await {
            set_state(SyncState::Backoff(err.to_string()));
            return;
        }
        set_state(SyncState::Idle);

        while let Some(cmd) = rx.recv().await {
            match cmd {
                SyncCmd::SyncNow => {
                    set_state(SyncState::Busy);
                    if let Err(err) = initial_sync(&mut session, &writer, &bus, account.id).await {
                        set_state(SyncState::Backoff(err.to_string()));
                        return;
                    }
                    set_state(SyncState::Idle);
                }
                SyncCmd::Shutdown => break,
            }
        }
        let _ = session.logout().await;
        set_state(SyncState::Stopped);
    });

    SyncHandle { tx, state }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::imap::testing::spawn_fake;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn seeded_db(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join(crate::infra::db::DB_FILE_NAME);
        let mut conn = crate::infra::db::open_rw(&path).expect("open");
        crate::infra::db::migrations::run(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO accounts(id, display_name, email, provider, imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, auth_type, created_at)
             VALUES (1, 'Seed', 'seed@example.com', 'imap', '127.0.0.1', 143, 'plaintext', 'smtp.example.com', 465, 'tls', 'password', 0)",
            [],
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
        let db_path = seeded_db(&dir);
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
            },
            std::sync::Arc::clone(&keystore) as std::sync::Arc<dyn crate::infra::keyring::KeyStore>,
            std::sync::Arc::clone(&writer),
            std::sync::Arc::clone(&bus),
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
}
