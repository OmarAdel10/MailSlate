//! Threading domain: conversation-level operations over the messages/
//! threads tables (ARCHITECTURE §3.1; PRD F3.3).
//!
//! Messages are already clustered into threads by the writer (Gmail
//! thread IDs where available; otherwise References/In-Reply-To or
//! subject-norm clustering). This module exposes the read side:
//! listing threads for an account and listing one thread's messages.

use crate::infra::db::pool::ReadPool;
use crate::infra::db::queries::{ThreadCursor, messages_in_thread, threads_page};

/// One conversation thread, as listed in a mailbox or conversation
/// view (PRD F3.4).
#[derive(Debug, Clone)]
pub struct Thread {
    /// Row id of the thread.
    pub id: i64,
    /// Account the thread belongs to.
    pub account_id: i64,
    /// Normalized subject (displayed as the conversation subject).
    pub subject: Option<String>,
    /// Date of the newest message in the thread.
    pub last_date: Option<i64>,
    /// Total messages in the thread.
    pub message_count: i64,
    /// Unread messages in the thread.
    pub unread_count: i64,
    /// Whether any message in the thread has attachments.
    pub has_attachments: bool,
    /// Snippet from the newest message.
    pub last_snippet: Option<String>,
}

/// One message within a thread, newest-first.
#[derive(Debug, Clone)]
pub struct ThreadMessage {
    /// Row id of the message.
    pub id: i64,
    /// Message subject.
    pub subject: Option<String>,
    /// JSON-serialized From addresses.
    pub from_json: Option<String>,
    /// Message date as unix epoch.
    pub date_utc: Option<i64>,
    /// Message snippet.
    pub snippet: Option<String>,
    /// Whether the message has attachments.
    pub has_attachments: bool,
    /// Body state: `headers` or `full`.
    pub body_state: String,
}

/// Lists one account's threads newest-first with keyset pagination.
///
/// `cursor = None` starts the first page; otherwise rows strictly
/// after the cursor's position are returned. Never uses `OFFSET`.
pub fn list_threads(
    pool: &ReadPool,
    account_id: i64,
    limit: u32,
    cursor: Option<ThreadCursor>,
) -> rusqlite::Result<Vec<Thread>> {
    let conn = pool
        .acquire()
        .map_err(|err| rusqlite::Error::InvalidColumnName(format!("read pool: {err}")))?;
    let rows = threads_page(&conn, account_id, limit, cursor)?;
    Ok(rows
        .into_iter()
        .map(|r| Thread {
            id: r.id,
            account_id: r.account_id,
            subject: r.subject_norm,
            last_date: r.last_date,
            message_count: r.message_count,
            unread_count: r.unread_count,
            has_attachments: r.has_attachments,
            last_snippet: r.last_snippet,
        })
        .collect())
}

/// Lists one thread's messages newest-first with keyset pagination.
/// `cursor = None` starts the first page.
pub fn list_thread_messages(
    pool: &ReadPool,
    thread_id: i64,
    limit: u32,
    cursor: Option<ThreadCursor>,
) -> rusqlite::Result<Vec<ThreadMessage>> {
    let conn = pool
        .acquire()
        .map_err(|err| rusqlite::Error::InvalidColumnName(format!("read pool: {err}")))?;
    let rows = messages_in_thread(&conn, thread_id, limit, cursor)?;
    Ok(rows
        .into_iter()
        .map(|r| ThreadMessage {
            id: r.id,
            subject: r.subject,
            from_json: r.from_json,
            date_utc: r.date_utc,
            snippet: r.snippet,
            has_attachments: r.has_attachments,
            body_state: r.body_state,
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

    /// Seeds a minimal schema + two threads with messages and returns
    /// the pool.
    fn test_pool() -> (TempDir, ReadPool, i64, i64) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join(DB_FILE_NAME);
        let mut conn = open_rw(&path).expect("open rw");
        migrations::run(&mut conn).expect("migrate");

        // Seed one account.
        conn.execute(
            "INSERT INTO accounts (id, display_name, email, provider, imap_host, imap_port,
                                   imap_security, smtp_host, smtp_port, smtp_security,
                                   auth_type, created_at)
             VALUES (?1, 'Work', 'work@example.com', 'imap', 'imap.example.com', 993,
                     'tls', 'smtp.example.com', 465, 'tls', 'password', 0)",
            params![42],
        )
        .expect("seed account");

        // Seed two threads with messages.
        conn.execute(
            "INSERT INTO threads (id, account_id, subject_norm, last_date,
                                  message_count, unread_count, has_attachments, last_snippet)
             VALUES (?1, 42, 'Project Alpha', 2000, 2, 1, 0, 'alpha snippet')",
            params![100],
        )
        .expect("seed thread 100");
        conn.execute(
            "INSERT INTO threads (id, account_id, subject_norm, last_date,
                                  message_count, unread_count, has_attachments, last_snippet)
             VALUES (?1, 42, 'Weekly Report', 1000, 1, 0, 0, 'report snippet')",
            params![101],
        )
        .expect("seed thread 101");

        // Messages for thread 100 (newest first).
        conn.execute(
            "INSERT INTO messages (id, account_id, dedupe_key, subject, from_json,
                                  date_utc, snippet, has_attachments, body_state, thread_id)
             VALUES (?1, 42, 'm-2', 'Alpha update',
                     '[{\"name\":\"Alice\",\"address\":\"alice@example.com\"}]',
                     2001, 'update snippet', 0, 'full', 100)",
            params![2],
        )
        .expect("seed message 2");
        conn.execute(
            "INSERT INTO messages (id, account_id, dedupe_key, subject, from_json,
                                  date_utc, snippet, has_attachments, body_state, thread_id)
             VALUES (?1, 42, 'm-1', 'Alpha start',
                     '[{\"name\":\"Bob\",\"address\":\"bob@example.com\"}]',
                     2000, 'start snippet', 0, 'headers', 100)",
            params![1],
        )
        .expect("seed message 1");

        // Message for thread 101.
        conn.execute(
            "INSERT INTO messages (id, account_id, dedupe_key, subject, from_json,
                                  date_utc, snippet, has_attachments, body_state, thread_id)
             VALUES (?1, 42, 'm-3', 'Weekly report',
                     '[{\"name\":\"Carol\",\"address\":\"carol@example.com\"}]',
                     1000, 'report snippet', 1, 'full', 101)",
            params![3],
        )
        .expect("seed message 3");

        drop(conn);
        let pool = ReadPool::open(&path).expect("open pool");
        (dir, pool, 100, 101)
    }

    #[test]
    fn list_threads_returns_both_threads() {
        let (_dir, pool, _t1, _t2) = test_pool();
        let threads = list_threads(&pool, 42, 50, None).expect("query");
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, 100);
        assert_eq!(threads[0].subject.as_deref(), Some("Project Alpha"));
        assert_eq!(threads[0].message_count, 2);
        assert_eq!(threads[0].unread_count, 1);
        assert_eq!(threads[1].id, 101);
        assert_eq!(threads[1].subject.as_deref(), Some("Weekly Report"));
        assert_eq!(threads[1].message_count, 1);
    }

    #[test]
    fn list_threads_scoped_to_one_account() {
        let (_dir, pool, _t1, _t2) = test_pool();
        let threads = list_threads(&pool, 999, 50, None).expect("query");
        assert!(threads.is_empty());
    }

    #[test]
    fn list_thread_messages_returns_thread_messages() {
        let (_dir, pool, t1, _t2) = test_pool();
        let messages = list_thread_messages(&pool, t1, 10, None).expect("query");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].id, 2);
        assert_eq!(messages[0].subject.as_deref(), Some("Alpha update"));
        assert_eq!(messages[1].id, 1);
        assert_eq!(messages[1].subject.as_deref(), Some("Alpha start"));
        assert!(messages[1].has_attachments == false);
    }

    #[test]
    fn list_thread_messages_empty_for_unknown_thread() {
        let (_dir, pool, _t1, _t2) = test_pool();
        let messages = list_thread_messages(&pool, 9999, 10, None).expect("query");
        assert!(messages.is_empty());
    }

    #[test]
    fn list_thread_messages_pages_with_cursor() {
        let (_dir, pool, t1, _t2) = test_pool();
        let first = list_thread_messages(&pool, t1, 1, None).expect("page 1");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, 2);
        let cursor = ThreadCursor {
            last_date: first[0].date_utc.expect("date"),
            thread_id: first[0].id,
        };
        let second = list_thread_messages(&pool, t1, 1, Some(cursor)).expect("page 2");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, 1);
    }
}
