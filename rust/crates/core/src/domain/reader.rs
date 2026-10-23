//! Reader domain: individual message access for the conversation reader
//! (ARCHITECTURE §3.1; PRD F3/F4).
//!
//! Exposes the read side: message headers, decoded body, attachments,
//! and read/unread state. The writer owns all persistence; the GUI
//! never touches the database (ARCHITECTURE §1).

use crate::infra::db::pool::ReadPool;
use crate::infra::db::queries::{
    message_attachments, message_body, message_detail, set_message_seen,
};
use rusqlite::Connection;

/// One message's header fields, as displayed in the reader.
#[derive(Debug, Clone)]
pub struct MessageDetail {
    pub id: i64,
    pub account_id: i64,
    pub subject: Option<String>,
    pub from_json: Option<String>,
    pub to_json: Option<String>,
    pub date_utc: Option<i64>,
    pub snippet: Option<String>,
    pub has_attachments: bool,
    pub body_state: String,
}

/// One message's decoded body.
#[derive(Debug, Clone)]
pub struct MessageBody {
    pub message_id: i64,
    pub text_body: Option<String>,
    pub html_clean: Option<String>,
}

/// One attachment summary for the reader's attachment list.
#[derive(Debug, Clone)]
pub struct AttachmentSummary {
    pub id: i64,
    pub part_id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub content_id: Option<String>,
    pub blob_path: Option<String>,
}

/// Returns one message's header fields by id and account.
pub fn get_message(
    pool: &ReadPool,
    account_id: i64,
    message_id: i64,
) -> rusqlite::Result<Option<MessageDetail>> {
    let conn = pool
        .acquire()
        .map_err(|err| rusqlite::Error::InvalidColumnName(format!("read pool: {err}")))?;
    let q = message_detail(&conn, account_id, message_id)?;
    Ok(q.map(|q| MessageDetail {
        id: q.id,
        account_id: q.account_id,
        subject: q.subject,
        from_json: q.from_json,
        to_json: q.to_json,
        date_utc: q.date_utc,
        snippet: q.snippet,
        has_attachments: q.has_attachments,
        body_state: q.body_state,
    }))
}

/// Returns one message's decoded body by id.
pub fn get_message_body(pool: &ReadPool, message_id: i64) -> rusqlite::Result<Option<MessageBody>> {
    let conn = pool
        .acquire()
        .map_err(|err| rusqlite::Error::InvalidColumnName(format!("read pool: {err}")))?;
    let q = message_body(&conn, message_id)?;
    Ok(q.map(|q| MessageBody {
        message_id: q.message_id,
        text_body: q.text_body,
        html_clean: q.html_clean,
    }))
}

/// Returns one message's attachments by id.
pub fn get_attachments(
    pool: &ReadPool,
    message_id: i64,
) -> rusqlite::Result<Vec<AttachmentSummary>> {
    let conn = pool
        .acquire()
        .map_err(|err| rusqlite::Error::InvalidColumnName(format!("read pool: {err}")))?;
    let qs = message_attachments(&conn, message_id)?;
    Ok(qs
        .into_iter()
        .map(|q| AttachmentSummary {
            id: q.id,
            part_id: q.part_id,
            filename: q.filename,
            mime_type: q.mime_type,
            size_bytes: q.size_bytes,
            content_id: q.content_id,
            blob_path: q.blob_path,
        })
        .collect())
}

/// Marks a message as read or unread in all its mailbox locations.
/// Takes a read-write connection (writes go through the WriterActor).
pub fn mark_read(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
    seen: bool,
) -> rusqlite::Result<usize> {
    set_message_seen(conn, account_id, message_id, seen)
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
        conn.execute(
            "INSERT INTO threads (id, account_id, subject_norm, last_date,
                                  message_count, unread_count, has_attachments)
             VALUES (?1, 42, 'Test thread', 1000, 1, 0, 0)",
            rusqlite::params![100],
        )
        .expect("seed thread");
        conn.execute(
            "INSERT INTO messages (id, account_id, dedupe_key, subject, from_json,
                                  to_json, date_utc, snippet, has_attachments,
                                  body_state, thread_id)
             VALUES (?1, 42, 'msg-reader', 'Reader subject',
                     '[{\"name\":\"Alice\",\"address\":\"alice@example.com\"}]',
                     '[{\"name\":\"Bob\",\"address\":\"bob@example.com\"}]',
                     1000, 'reader snippet', 1, 'full', 100)",
            rusqlite::params![1],
        )
        .expect("seed message");
        conn.execute(
            "INSERT INTO message_bodies (message_id, text_body, html_clean, fetched_at)
             VALUES (?1, 'Plain text body', '<b>HTML body</b>', 1000)",
            rusqlite::params![1],
        )
        .expect("seed body");
        conn.execute(
            "INSERT INTO mailboxes (id, account_id, name, kind, special_use, color,
                                  sort_order, uidvalidity, uidnext, highest_modseq,
                                  last_synced_at)
             VALUES (?1, 42, 'INBOX', 'inbox', NULL, NULL, 0, NULL, NULL, NULL, NULL)",
            rusqlite::params![1],
        )
        .expect("seed mailbox");
        conn.execute(
            "INSERT INTO message_locations (message_id, mailbox_id, uid, seen,
                                          flagged, answered, draft, deleted,
                                          keywords_json, modseq)
             VALUES (?1, 1, 1, 0, 0, 0, 0, 0, '[]', NULL)",
            rusqlite::params![1],
        )
        .expect("seed location");
        conn.execute(
            "INSERT INTO attachments (id, message_id, part_id, filename,
                                      mime_type, size_bytes, content_id, blob_path)
             VALUES (?1, 1, 'part-1', 'report.pdf', 'application/pdf',
                     1024, '<cid-1>', 'ab/cd1234')",
            rusqlite::params![1],
        )
        .expect("seed attachment");
        drop(conn);
        let pool = ReadPool::open(&path).expect("open pool");
        (dir, pool)
    }

    #[test]
    fn get_message_returns_detail() {
        let (_dir, pool) = test_pool();
        let detail = get_message(&pool, 42, 1).expect("query").expect("row");
        assert_eq!(detail.id, 1);
        assert_eq!(detail.account_id, 42);
        assert_eq!(detail.subject.as_deref(), Some("Reader subject"));
        assert_eq!(detail.body_state, "full");
        assert!(detail.has_attachments);
    }

    #[test]
    fn get_message_none_for_unknown() {
        let (_dir, pool) = test_pool();
        let detail = get_message(&pool, 42, 999).expect("query");
        assert!(detail.is_none());
    }

    #[test]
    fn get_message_body_returns_body() {
        let (_dir, pool) = test_pool();
        let body = get_message_body(&pool, 1).expect("query").expect("row");
        assert_eq!(body.message_id, 1);
        assert_eq!(body.text_body.as_deref(), Some("Plain text body"));
        assert_eq!(body.html_clean.as_deref(), Some("<b>HTML body</b>"));
    }

    #[test]
    fn get_attachments_returns_attachments() {
        let (_dir, pool) = test_pool();
        let attachments = get_attachments(&pool, 1).expect("query");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].filename.as_deref(), Some("report.pdf"));
        assert_eq!(attachments[0].mime_type.as_deref(), Some("application/pdf"));
        assert_eq!(attachments[0].blob_path.as_deref(), Some("ab/cd1234"));
    }

    #[test]
    fn mark_read_marks_as_read() {
        let (dir, _pool) = test_pool();
        let conn = open_rw(&dir.path().join(DB_FILE_NAME)).expect("open rw");
        let count = mark_read(&conn, 42, 1, true).expect("update");
        assert_eq!(count, 1);
    }

    #[test]
    fn mark_read_scoped_to_account() {
        let (dir, _pool) = test_pool();
        let conn = open_rw(&dir.path().join(DB_FILE_NAME)).expect("open rw");
        let count = mark_read(&conn, 999, 1, true).expect("update");
        assert_eq!(count, 0);
    }
}
