//! Accounts domain: create, read, update, and remove mail accounts.
//!
//! The daemon owns all writes (ARCHITECTURE §1); password-class
//! secrets never touch the database — they go to the OS keyring
//! through [`KeyStore`] under the `account:{id}:*` key scheme.

use rusqlite::OptionalExtension;

use crate::infra::db::writer::{WriteOp, WriterActor};
use crate::infra::keyring::{KeyStore, KeyStoreError, imap_password_key, oauth_key};
use crate::infra::oauth::tokens;

/// Transport-layer security for IMAP/SMTP endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Tls,
    StartTls,
    Plaintext,
}

impl Security {
    fn as_str(self) -> &'static str {
        match self {
            Security::Tls => "tls",
            Security::StartTls => "starttls",
            Security::Plaintext => "plaintext",
        }
    }

    /// Parses the schema's wire names; unknown values yield None
    /// (CHECK constraint should make that unreachable for stored rows).
    pub fn parse(raw: &str) -> Option<Security> {
        match raw {
            "tls" => Some(Security::Tls),
            "starttls" => Some(Security::StartTls),
            "plaintext" => Some(Security::Plaintext),
            _ => None,
        }
    }
}

/// How an account authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    /// OAuth2 tokens (Gmail/Outlook); secrets live under the oauth key.
    OAuth2,
    /// Plain IMAP/SMTP login; the secret is the account password.
    Password,
}

impl AuthKind {
    fn as_str(self) -> &'static str {
        match self {
            AuthKind::OAuth2 => "oauth2",
            AuthKind::Password => "password",
        }
    }
}

/// Everything needed to create a password-backed IMAP account.
#[derive(Debug, Clone)]
pub struct ImapAccountDraft {
    pub display_name: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
    pub auth: AuthKind,
}

/// Failures of account creation.
#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("key store rejected the secret: {0}")]
    KeyStore(#[from] KeyStoreError),
    #[error("writer thread is gone")]
    WriterClosed,
}

/// Creates the account row and stores its password secret in the key
/// store. Returns the new account id.
///
/// The DB write goes through the single-writer actor; the secret goes
/// ONLY to the key store (ARCHITECTURE §12). If the key store fails,
/// nothing is written to the database.
pub fn add_imap_account(
    keystore: &dyn KeyStore,
    writer: &WriterActor,
    draft: &ImapAccountDraft,
    password: &str,
) -> Result<i64, AccountError> {
    // Reserve the id first: the key name is derived from it, and the
    // secret must be durable before any row references it.
    let account_id = next_account_id(writer)?;

    keystore.store(&imap_password_key(account_id), password)?;

    writer.enqueue(WriteOp::AccountAdd {
        id: account_id,
        display_name: draft.display_name.clone(),
        email: draft.email.clone(),
        provider: "imap".into(),
        imap_host: draft.imap_host.clone(),
        imap_port: draft.imap_port,
        imap_security: draft.imap_security.as_str().into(),
        smtp_host: draft.smtp_host.clone(),
        smtp_port: draft.smtp_port,
        smtp_security: draft.smtp_security.as_str().into(),
        auth_type: draft.auth.as_str().into(),
    });
    // Account creation is a UI-visible operation: when it returns, the
    // row must be durable (batch window otherwise delays it up to 50ms).
    writer.flush();
    Ok(account_id)
}

/// Keyring id that stages OAuth tokens between the browser callback
/// and `complete_oauth_account`: `oauth_key(0)` = `account:0:oauth`.
pub const PENDING_ACCOUNT_ID: i64 = 0;

/// Creates an OAuth2 account from a completed authorization-code flow.
///
/// The token snapshot is staged under [`PENDING_ACCOUNT_ID`] by the
/// daemon's callback waiter; this moves it to the real account's key,
/// removes the staging copy, and writes the row with provider endpoints
/// from the provider profile. `color` (optional) is applied via a
/// follow-up patch because the add op carries no color column.
#[allow(clippy::too_many_arguments)]
pub fn add_oauth_account(
    keystore: &dyn KeyStore,
    writer: &WriterActor,
    provider: &str,
    display_name: &str,
    email: &str,
    color: Option<String>,
    imap: (&str, u16, Security),
    smtp: (&str, u16, Security),
    snapshot: &crate::infra::oauth::tokens::TokenSnapshot,
) -> Result<i64, AccountError> {
    let account_id = next_account_id(writer)?;

    tokens::save(keystore, account_id, snapshot)?;
    tokens::delete(keystore, PENDING_ACCOUNT_ID)?;

    writer.enqueue(WriteOp::AccountAdd {
        id: account_id,
        display_name: display_name.into(),
        email: email.into(),
        provider: provider.into(),
        imap_host: imap.0.into(),
        imap_port: imap.1,
        imap_security: imap.2.as_str().into(),
        smtp_host: smtp.0.into(),
        smtp_port: smtp.1,
        smtp_security: smtp.2.as_str().into(),
        auth_type: AuthKind::OAuth2.as_str().into(),
    });
    if let Some(color) = color {
        writer.enqueue(WriteOp::AccountUpdate {
            id: account_id,
            display_name: None,
            color: Some(Some(color)),
            sync_window_days: None,
        });
    }
    writer.flush();
    Ok(account_id)
}

/// Account ids are process-local counters persisted in `meta`; a
/// restart resumes after the highest stored id.
fn next_account_id(writer: &WriterActor) -> Result<i64, AccountError> {
    // The actor assigns ids monotonically: enqueuing an IdAllocate op
    // would round-trip through the actor, but ids only need uniqueness
    // within this daemon's store. A meta counter written by the actor
    // keeps restart-safety without read-modify-write races here.
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    writer.enqueue(WriteOp::MetaPut {
        key: "accounts.next_id".into(),
        value: (id + 1).to_string(),
    });
    Ok(id)
}

use std::sync::atomic::{AtomicI64, Ordering};
static NEXT_ID: AtomicI64 = AtomicI64::new(1);

/// Resumes the process-local id counter after a restart. The daemon
/// calls this once at startup, before serving any request: the
/// counter would otherwise start back at 1 and the first new
/// account would collide with the previous process's rows (PK
/// panic) and overwrite its keyring secrets.
///
/// Prefers the persisted `accounts.next_id` counter; falls back to
/// one past the highest stored account id (covers stores written
/// before the counter existed).
pub fn resume_next_id(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let stored: Option<i64> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'accounts.next_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .and_then(|raw| raw.parse().ok());
    let highest: i64 = conn.query_row("SELECT COALESCE(MAX(id), 0) FROM accounts", [], |row| {
        row.get(0)
    })?;
    let resumed = stored.unwrap_or(highest + 1).max(1);
    NEXT_ID.store(resumed, Ordering::Relaxed);
    Ok(())
}

/// A read-side summary of one stored account.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountSummary {
    pub id: i64,
    pub display_name: String,
    pub email: String,
    pub provider: String,
    pub auth_type: String,
    pub color: Option<String>,
    pub sync_window_days: i64,
}

/// Connection + auth facts for one account — exactly what the sync
/// actor needs to connect and authenticate.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountConnection {
    pub id: i64,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: Security,
    pub auth_type: String,
    pub provider: String,
}

/// Lists every account's sync inputs (daemon spawns one actor each).
pub fn get_account_connections(
    conn: &rusqlite::Connection,
) -> rusqlite::Result<Vec<AccountConnection>> {
    let mut stmt = conn.prepare(
        "SELECT id, email, imap_host, imap_port, imap_security, auth_type, provider
         FROM accounts
         ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AccountConnection {
            id: row.get(0)?,
            email: row.get(1)?,
            imap_host: row.get(2)?,
            imap_port: row.get::<_, i64>(3)? as u16,
            imap_security: Security::parse(row.get::<_, String>(4)?.as_str())
                .unwrap_or(Security::Tls),
            auth_type: row.get(5)?,
            provider: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// Lists every stored account, newest last (insertion order by id).
pub fn get_accounts(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<AccountSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, display_name, email, provider, auth_type, color, sync_window_days
         FROM accounts
         ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AccountSummary {
            id: row.get(0)?,
            display_name: row.get(1)?,
            email: row.get(2)?,
            provider: row.get(3)?,
            auth_type: row.get(4)?,
            color: row.get(5)?,
            sync_window_days: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// A partial update: `None` leaves the field untouched; for color,
/// `Some(None)` clears it.
#[derive(Debug, Clone, Default)]
pub struct AccountPatch {
    pub display_name: Option<String>,
    pub color: Option<Option<String>>,
    pub sync_window_days: Option<i64>,
}

/// Patches an account's mutable fields. `None` leaves a field
/// untouched; for color, `Some(None)` clears it. Durability on
/// return: the patch is committed by the time this returns.
pub fn update_account(
    writer: &WriterActor,
    account_id: i64,
    patch: AccountPatch,
) -> Result<(), AccountError> {
    writer.enqueue(WriteOp::AccountUpdate {
        id: account_id,
        display_name: patch.display_name,
        color: patch.color,
        sync_window_days: patch.sync_window_days,
    });
    writer.flush();
    Ok(())
}

/// Removes an account: the row goes (children cascade) and both
/// keyring keys are scrubbed best-effort. The keep-local offline
/// variant is a later-slice concern (GUI semantics, phase 7).
pub fn remove_account(
    keystore: &dyn KeyStore,
    writer: &WriterActor,
    account_id: i64,
) -> Result<(), AccountError> {
    // Scrub secrets first: if the DB delete fails the keyring is
    // already clean and a retry cannot leave a live orphan secret.
    let _ = keystore.delete(&imap_password_key(account_id));
    let _ = keystore.delete(&oauth_key(account_id));
    writer.enqueue(WriteOp::AccountRemove { id: account_id });
    // Removal is UI-visible: the row (and cascaded children) must be
    // durable by the time this returns.
    writer.flush();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::migrations;
    use crate::infra::keyring::{FakeKeyStore, imap_password_key};
    use tempfile::tempdir;

    fn seeded_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join(crate::infra::db::DB_FILE_NAME);
        let mut conn = crate::infra::db::open_rw(&path).expect("open seed db");
        migrations::run(&mut conn).expect("migrate seed db");
        (dir, path)
    }

    /// Adding a password-backed IMAP account stores the row AND the
    /// secret in the key store — the secret must never touch the DB.
    #[test]
    fn add_imap_account_stores_secret_and_row() {
        let (_dir, path) = seeded_db();
        let keystore = FakeKeyStore::new();
        let writer = WriterActor::spawn(&path);

        let draft = ImapAccountDraft {
            display_name: "Work".into(),
            email: "user@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            auth: AuthKind::Password,
        };

        let account_id =
            add_imap_account(&keystore, &writer, &draft, "secret-pw").expect("account creation");

        // The row exists with the right provider/auth/hosts.
        let conn = crate::infra::db::open_readonly(&path).expect("reopen");
        let (provider, auth_type, imap_host, port, security): (
            String,
            String,
            String,
            i64,
            String,
        ) = conn
            .query_row(
                "SELECT provider, auth_type, imap_host, imap_port, imap_security
                 FROM accounts WHERE id = ?1",
                [account_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("account row must exist");

        assert_eq!(provider, "imap");
        assert_eq!(auth_type, "password");
        assert_eq!(imap_host, "imap.example.com");
        assert_eq!(port, 993);
        assert_eq!(security, "tls");

        // The secret landed in the key store.
        assert_eq!(
            keystore
                .get(&imap_password_key(account_id))
                .expect("key get"),
            Some("secret-pw".into())
        );
    }

    /// The account list reports every stored account with its summary
    /// fields, including schema defaults (no color, one-year window).
    #[test]
    fn get_accounts_lists_every_account_with_defaults() {
        let (_dir, path) = seeded_db();
        let keystore = FakeKeyStore::new();
        let writer = WriterActor::spawn(&path);

        let first = ImapAccountDraft {
            display_name: "Work".into(),
            email: "work@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            auth: AuthKind::Password,
        };
        let second = ImapAccountDraft {
            display_name: "Personal".into(),
            email: "home@example.com".into(),
            imap_host: "imap2.example.com".into(),
            imap_port: 143,
            imap_security: Security::StartTls,
            smtp_host: "smtp2.example.com".into(),
            smtp_port: 587,
            smtp_security: Security::StartTls,
            auth: AuthKind::Password,
        };
        let first_id = add_imap_account(&keystore, &writer, &first, "pw-1").expect("first account");
        let second_id =
            add_imap_account(&keystore, &writer, &second, "pw-2").expect("second account");

        let conn = crate::infra::db::open_readonly(&path).expect("reopen");
        let accounts = get_accounts(&conn).expect("account list");

        assert_eq!(accounts.len(), 2, "both accounts must be listed");
        let by_id: std::collections::HashMap<i64, &AccountSummary> =
            accounts.iter().map(|a| (a.id, a)).collect();
        let work = by_id.get(&first_id).expect("first account listed");
        assert_eq!(work.display_name, "Work");
        assert_eq!(work.email, "work@example.com");
        assert_eq!(work.provider, "imap");
        assert_eq!(work.auth_type, "password");
        assert_eq!(work.color, None, "no color assigned yet");
        assert_eq!(work.sync_window_days, 365, "schema default window");
        let home = by_id.get(&second_id).expect("second account listed");
        assert_eq!(home.display_name, "Personal");
        assert_eq!(home.email, "home@example.com");
    }

    /// A fresh database lists no accounts.
    #[test]
    fn get_accounts_on_fresh_db_is_empty() {
        let (_dir, path) = seeded_db();
        let conn = crate::infra::db::open_readonly(&path).expect("reopen");
        let accounts = get_accounts(&conn).expect("account list");
        assert!(accounts.is_empty(), "fresh db must list nothing");
    }

    /// Removing an account deletes its row (children cascade) and
    /// scrubs both keyring keys best-effort.
    #[test]
    fn remove_account_deletes_row_and_keyring_entries() {
        let (_dir, path) = seeded_db();
        let keystore = FakeKeyStore::new();
        let writer = WriterActor::spawn(&path);

        let draft = ImapAccountDraft {
            display_name: "Work".into(),
            email: "work@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            auth: AuthKind::Password,
        };
        let account_id =
            add_imap_account(&keystore, &writer, &draft, "secret-pw").expect("account");

        // An OAuth key may also exist for other provider kinds; both go.
        keystore
            .store(&oauth_key(account_id), "oauth-json")
            .expect("store oauth key");

        remove_account(&keystore, &writer, account_id).expect("remove");

        let conn = crate::infra::db::open_readonly(&path).expect("reopen");
        assert!(
            get_accounts(&conn).expect("list").is_empty(),
            "account row must be gone"
        );
        assert_eq!(
            keystore
                .get(&imap_password_key(account_id))
                .expect("pw get"),
            None,
            "imap password key must be scrubbed"
        );
        assert_eq!(
            keystore.get(&oauth_key(account_id)).expect("oauth get"),
            None,
            "oauth key must be scrubbed"
        );
    }

    /// update_account patches only the fields the caller set: unset
    /// fields keep their stored values, and an explicit Some(None)
    /// clears the color.
    #[test]
    fn update_account_patches_only_given_fields() {
        let (_dir, path) = seeded_db();
        let keystore = FakeKeyStore::new();
        let writer = WriterActor::spawn(&path);
        let draft = ImapAccountDraft {
            display_name: "Work".into(),
            email: "user@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            auth: AuthKind::Password,
        };
        let account_id = add_imap_account(&keystore, &writer, &draft, "pw").expect("create");

        // 1. Set color + window; display name untouched.
        update_account(
            &writer,
            account_id,
            AccountPatch {
                display_name: None,
                color: Some(Some("blue".into())),
                sync_window_days: Some(90),
            },
        )
        .expect("update 1");
        let conn = crate::infra::db::open_readonly(&path).expect("reopen");
        let listed = get_accounts(&conn).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].display_name, "Work", "unset field untouched");
        assert_eq!(listed[0].color.as_deref(), Some("blue"));
        assert_eq!(listed[0].sync_window_days, 90);
        drop(conn);

        // 2. Rename + explicitly clear color; window untouched.
        update_account(
            &writer,
            account_id,
            AccountPatch {
                display_name: Some("Renamed".into()),
                color: Some(None),
                sync_window_days: None,
            },
        )
        .expect("update 2");
        let conn = crate::infra::db::open_readonly(&path).expect("reopen 2");
        let listed = get_accounts(&conn).expect("list 2");
        assert_eq!(listed[0].display_name, "Renamed");
        assert_eq!(listed[0].color, None, "Some(None) must clear the color");
        assert_eq!(listed[0].sync_window_days, 90, "unset field untouched");
    }

    #[test]
    fn add_oauth_account_moves_staged_tokens_and_creates_row() {
        use crate::infra::keyring::oauth_key;
        use crate::infra::oauth::tokens::{self, TokenSnapshot};

        let (_dir, path) = seeded_db();
        let keystore = FakeKeyStore::new();
        let writer = WriterActor::spawn(&path);
        let snapshot = TokenSnapshot {
            access_token: "at-o".into(),
            refresh_token: "rt-o".into(),
            expires_at: 9_999,
            scopes: "scope-1".into(),
        };
        tokens::save(&keystore, PENDING_ACCOUNT_ID, &snapshot).expect("stage snapshot");

        let account_id = add_oauth_account(
            &keystore,
            &writer,
            "gmail",
            "Gmail",
            "me@gmail.com",
            Some("blue".into()),
            ("imap.gmail.com", 993, Security::Tls),
            ("smtp.gmail.com", 587, Security::StartTls),
            &snapshot,
        )
        .expect("oauth account created");

        writer.flush();
        let moved = keystore
            .get(&oauth_key(account_id))
            .expect("keystore readable")
            .expect("tokens re-keyed to the real account");
        assert!(moved.contains("at-o"), "moved snapshot: {moved}");
        assert_eq!(
            keystore.get(&oauth_key(PENDING_ACCOUNT_ID)).unwrap(),
            None,
            "staged key scrubbed"
        );

        let conn = crate::infra::db::open_readonly(&path).expect("reopen");
        let (provider, auth_type, color, imap_host): (String, String, Option<String>, String) =
            conn.query_row(
                "SELECT provider, auth_type, color, imap_host FROM accounts WHERE id = ?1",
                [account_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("account row");
        assert_eq!(provider, "gmail");
        assert_eq!(auth_type, "oauth2");
        assert_eq!(imap_host, "imap.gmail.com");
        assert_eq!(color.as_deref(), Some("blue"));
    }

    #[test]
    fn security_parses_wire_names() {
        assert_eq!(Security::parse("tls"), Some(Security::Tls));
        assert_eq!(Security::parse("starttls"), Some(Security::StartTls));
        assert_eq!(Security::parse("plaintext"), Some(Security::Plaintext));
        assert_eq!(Security::parse("bogus"), None);
    }

    #[test]
    fn account_connections_carry_sync_inputs() {
        let (_dir, path) = seeded_db();
        let keystore = FakeKeyStore::new();
        let writer = WriterActor::spawn(&path);
        let draft = ImapAccountDraft {
            display_name: "Work".into(),
            email: "work@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            imap_security: Security::StartTls,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            smtp_security: Security::StartTls,
            auth: AuthKind::Password,
        };
        let account_id = add_imap_account(&keystore, &writer, &draft, "pw").expect("account");

        let conn = crate::infra::db::open_readonly(&path).expect("reopen");
        let connections = get_account_connections(&conn).expect("connections");
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].id, account_id);
        assert_eq!(connections[0].email, "work@example.com");
        assert_eq!(connections[0].imap_host, "imap.example.com");
        assert_eq!(connections[0].imap_port, 993);
        assert_eq!(connections[0].imap_security, Security::StartTls);
        assert_eq!(connections[0].auth_type, "password");
        assert_eq!(connections[0].provider, "imap");
    }

    /// `resume_next_id` seeds the process-global counter, so tests
    /// that store and read it must not interleave.
    static NEXT_ID_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn draft() -> ImapAccountDraft {
        ImapAccountDraft {
            display_name: "Work".into(),
            email: "user@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            auth: AuthKind::Password,
        }
    }

    /// A restart resumes the persisted counter: the first id handed
    /// out after `resume_next_id` must never collide with rows or
    /// keyring entries written by the previous process (QA: id reuse
    /// overwrites the account:1 secret and panics the PK insert).
    #[test]
    fn resume_next_id_continues_the_persisted_counter() {
        let _guard = NEXT_ID_LOCK.lock().expect("next id lock");
        let (_dir, path) = seeded_db();
        let conn = crate::infra::db::open_rw(&path).expect("open seed db");
        conn.execute(
            "INSERT INTO accounts(
                id, display_name, email, provider,
                imap_host, imap_port, imap_security,
                smtp_host, smtp_port, smtp_security, auth_type, created_at
             ) VALUES (1, 'Work', 'user@example.com', 'imap',
                       'imap.example.com', 993, 'tls',
                       'smtp.example.com', 465, 'tls', 'password', 0)",
            [],
        )
        .expect("seed account row");
        conn.execute(
            "INSERT INTO meta(key, value) VALUES ('accounts.next_id', '5')",
            [],
        )
        .expect("seed counter");
        drop(conn);

        resume_next_id(&crate::infra::db::open_readonly(&path).expect("reopen"))
            .expect("resume counter");

        let keystore = FakeKeyStore::new();
        let writer = WriterActor::spawn(&path);
        let account_id =
            add_imap_account(&keystore, &writer, &draft(), "secret-pw").expect("add account");
        assert_eq!(account_id, 5);
    }

    /// Without the meta counter (e.g. a store written by an older
    /// build), the counter falls back to one past the highest row.
    #[test]
    fn resume_next_id_falls_back_to_the_highest_row() {
        let _guard = NEXT_ID_LOCK.lock().expect("next id lock");
        let (_dir, path) = seeded_db();
        let conn = crate::infra::db::open_rw(&path).expect("open seed db");
        conn.execute(
            "INSERT INTO accounts(
                id, display_name, email, provider,
                imap_host, imap_port, imap_security,
                smtp_host, smtp_port, smtp_security, auth_type, created_at
             ) VALUES (3, 'Work', 'user@example.com', 'imap',
                       'imap.example.com', 993, 'tls',
                       'smtp.example.com', 465, 'tls', 'password', 0)",
            [],
        )
        .expect("seed account row");
        drop(conn);

        resume_next_id(&crate::infra::db::open_readonly(&path).expect("reopen"))
            .expect("resume counter");

        let keystore = FakeKeyStore::new();
        let writer = WriterActor::spawn(&path);
        let account_id =
            add_imap_account(&keystore, &writer, &draft(), "secret-pw").expect("add account");
        assert_eq!(account_id, 4);
    }
}
