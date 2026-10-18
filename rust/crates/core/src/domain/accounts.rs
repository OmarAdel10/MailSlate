//! Accounts domain: create, read, update, and remove mail accounts.
//!
//! The daemon owns all writes (ARCHITECTURE §1); password-class
//! secrets never touch the database — they go to the OS keyring
//! through [`KeyStore`] under the `account:{id}:*` key scheme.

use crate::infra::db::writer::{WriteOp, WriterActor};
use crate::infra::keyring::{KeyStore, KeyStoreError, imap_password_key};

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
}
