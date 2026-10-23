//! Mailbox domain: conversation mailbox listing (ARCHITECTURE §3.1; PRD F3).
//!
//! Mailboxes are upserted by the sync engine during initial sync
//! (ARCHITECTURE §9). This module exposes the read side: listing one
//! account's mailboxes sorted by sort_order, kind, name.

use crate::infra::db::pool::ReadPool;
use crate::infra::db::queries::{MailboxCursor, mailboxes_page};

/// One mailbox, as listed in the mailbox sidebar / conversation view.
#[derive(Debug, Clone)]
pub struct MailboxSummary {
    /// Row id of the mailbox.
    pub id: i64,
    /// Account the mailbox belongs to.
    pub account_id: i64,
    /// IMAP mailbox name (e.g. "INBOX", "Sent", "[Gmail]/All Mail").
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

/// Lists one account's mailboxes newest-first? No — sorted by
/// sort_order ASC, kind ASC, name ASC, with keyset pagination.
/// `cursor = None` starts the first page.
pub fn list_mailboxes(
    pool: &ReadPool,
    account_id: i64,
    limit: u32,
    cursor: Option<MailboxCursor>,
) -> rusqlite::Result<Vec<MailboxSummary>> {
    let conn = pool
        .acquire()
        .map_err(|err| rusqlite::Error::InvalidColumnName(format!("read pool: {err}")))?;
    let rows = mailboxes_page(&conn, account_id, limit, cursor)?;
    Ok(rows
        .into_iter()
        .map(|r| MailboxSummary {
            id: r.id,
            account_id: r.account_id,
            name: r.name,
            kind: r.kind,
            special_use: r.special_use,
            color: r.color,
            sort_order: r.sort_order,
            uidvalidity: r.uidvalidity,
            uidnext: r.uidnext,
            highest_modseq: r.highest_modseq,
            last_synced_at: r.last_synced_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::pool::ReadPool;
    use crate::infra::db::{DB_FILE_NAME, migrations, open_rw};
    use rusqlite::params;
    use tempfile::TempDir;

    fn test_pool() -> (TempDir, ReadPool) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(DB_FILE_NAME);
        // Seed via a read-write connection first.
        {
            let mut conn = open_rw(&path).expect("open rw");
            migrations::run(&mut conn).expect("migrate");
            conn.execute(
                "INSERT INTO accounts (id, display_name, email, provider, imap_host, imap_port,
                                       imap_security, smtp_host, smtp_port, smtp_security,
                                       auth_type, created_at)
                 VALUES (?1, 'Work', 'work@example.com', 'imap', 'imap.example.com', 993,
                         'tls', 'smtp.example.com', 465, 'tls', 'password', 0)",
                params![42],
            )
            .expect("seed account");
            drop(conn);
        }
        let pool = ReadPool::open(&path).expect("open pool");
        (dir, pool)
    }

    fn seed_mailbox(conn: &rusqlite::Connection, id: i64, name: &str, kind: &str, sort_order: i64) {
        conn.execute(
            "INSERT INTO mailboxes (id, account_id, name, kind, special_use, color,
                                  sort_order, uidvalidity, uidnext, highest_modseq,
                                  last_synced_at)
             VALUES (?1, 42, ?2, ?3, NULL, NULL, ?4, NULL, NULL, NULL, NULL)",
            rusqlite::params![id, name, kind, sort_order],
        )
        .expect("seed mailbox");
    }

    #[test]
    fn list_mailboxes_returns_seeded_mailboxes() {
        let (dir, pool) = test_pool();
        let conn = open_rw(&dir.path().join(DB_FILE_NAME)).expect("open rw");
        seed_mailbox(&conn, 1, "INBOX", "inbox", 0);
        seed_mailbox(&conn, 2, "Sent", "sent", 10);
        seed_mailbox(&conn, 3, "Drafts", "drafts", 20);
        drop(conn);

        let mailboxes = list_mailboxes(&pool, 42, 50, None).expect("query");
        assert_eq!(mailboxes.len(), 3);
        assert_eq!(mailboxes[0].name, "INBOX");
        assert_eq!(mailboxes[0].kind, "inbox");
        assert_eq!(mailboxes[1].name, "Sent");
        assert_eq!(mailboxes[2].name, "Drafts");
        assert!(mailboxes.iter().all(|m| m.account_id == 42));
    }

    #[test]
    fn list_mailboxes_scoped_to_one_account() {
        let (_dir, pool) = test_pool();
        let mailboxes = list_mailboxes(&pool, 999, 50, None).expect("query");
        assert!(mailboxes.is_empty());
    }

    #[test]
    fn list_mailboxes_pages_with_cursor() {
        let (dir, pool) = test_pool();
        let conn = open_rw(&dir.path().join(DB_FILE_NAME)).expect("open rw");
        seed_mailbox(&conn, 1, "A", "user", 0);
        seed_mailbox(&conn, 2, "B", "user", 10);
        seed_mailbox(&conn, 3, "C", "user", 20);
        drop(conn);

        let first = list_mailboxes(&pool, 42, 1, None).expect("page 1");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "A");
        let cursor = MailboxCursor {
            sort_order: first[0].sort_order,
            kind: first[0].kind.clone(),
            name: first[0].name.clone(),
        };
        let second = list_mailboxes(&pool, 42, 1, Some(cursor)).expect("page 2");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].name, "B");
    }
}
