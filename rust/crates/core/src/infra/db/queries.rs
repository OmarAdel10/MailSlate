//! Read-side queries over the pooled read connections.
//!
//! Pagination is keyset-only (`last_date`, `thread_id`): `OFFSET`
//! degrades linearly with depth and re-reads rows (ADR 13).

use rusqlite::Connection;

/// A keyset cursor pointing at the row AFTER which the next page
/// starts (exclusive on both tuple members).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadCursor {
    pub last_date: i64,
    pub thread_id: i64,
}

/// One row of the thread list projection.
#[derive(Debug, PartialEq)]
pub struct ThreadRow {
    pub id: i64,
    pub account_id: i64,
    pub subject_norm: Option<String>,
    pub last_date: Option<i64>,
    pub message_count: i64,
    pub unread_count: i64,
    pub has_attachments: bool,
    pub last_snippet: Option<String>,
}

/// Lists one account's threads newest-first with keyset pagination.
/// `cursor = None` starts the first page; otherwise rows strictly
/// after the cursor's position are returned. Never uses `OFFSET`.
pub fn threads_page(
    conn: &Connection,
    account_id: i64,
    limit: u32,
    cursor: Option<ThreadCursor>,
) -> rusqlite::Result<Vec<ThreadRow>> {
    match cursor {
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, account_id, subject_norm, last_date, message_count,
                        unread_count, has_attachments, last_snippet
                 FROM threads
                 WHERE account_id = ?1
                 ORDER BY last_date DESC, id DESC
                 LIMIT ?2",
            )?;
            collect(stmt.query_map([account_id, limit as i64], map_row)?)
        }
        Some(cursor) => {
            let mut stmt = conn.prepare(
                "SELECT id, account_id, subject_norm, last_date, message_count,
                        unread_count, has_attachments, last_snippet
                 FROM threads
                 WHERE account_id = ?1
                   AND (last_date < ?2 OR (last_date = ?2 AND id < ?3))
                 ORDER BY last_date DESC, id DESC
                 LIMIT ?4",
            )?;
            collect(stmt.query_map(
                rusqlite::params![account_id, cursor.last_date, cursor.thread_id, limit as i64],
                map_row,
            )?)
        }
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadRow> {
    Ok(ThreadRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        subject_norm: row.get(2)?,
        last_date: row.get(3)?,
        message_count: row.get(4)?,
        unread_count: row.get(5)?,
        has_attachments: row.get(6)?,
        last_snippet: row.get(7)?,
    })
}

/// One row of the thread message projection.
#[derive(Debug, PartialEq)]
pub struct MessageRow {
    pub id: i64,
    pub subject: Option<String>,
    pub from_json: Option<String>,
    pub date_utc: Option<i64>,
    pub snippet: Option<String>,
    pub has_attachments: bool,
    pub body_state: String,
}

/// Lists one thread's messages newest-first with keyset pagination.
/// `cursor = None` starts the first page; otherwise rows strictly
/// after the cursor's position are returned. Never uses `OFFSET`.
pub fn messages_in_thread(
    conn: &Connection,
    thread_id: i64,
    limit: u32,
    cursor: Option<ThreadCursor>,
) -> rusqlite::Result<Vec<MessageRow>> {
    match cursor {
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, subject, from_json, date_utc, snippet,
                        has_attachments, body_state
                 FROM messages
                 WHERE thread_id = ?1
                 ORDER BY date_utc DESC, id DESC
                 LIMIT ?2",
            )?;
            collect_messages(stmt.query_map([thread_id, limit as i64], map_message_row)?)
        }
        Some(cursor) => {
            let mut stmt = conn.prepare(
                "SELECT id, subject, from_json, date_utc, snippet,
                        has_attachments, body_state
                 FROM messages
                 WHERE thread_id = ?1
                   AND (date_utc < ?2 OR (date_utc = ?2 AND id < ?3))
                 ORDER BY date_utc DESC, id DESC
                 LIMIT ?4",
            )?;
            collect_messages(stmt.query_map(
                rusqlite::params![thread_id, cursor.last_date, cursor.thread_id, limit as i64],
                map_message_row,
            )?)
        }
    }
}

fn map_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: row.get(0)?,
        subject: row.get(1)?,
        from_json: row.get(2)?,
        date_utc: row.get(3)?,
        snippet: row.get(4)?,
        has_attachments: row.get(5)?,
        body_state: row.get(6)?,
    })
}

fn collect_messages(
    iterator: impl Iterator<Item = rusqlite::Result<MessageRow>>,
) -> rusqlite::Result<Vec<MessageRow>> {
    iterator.collect()
}

fn collect(
    iterator: impl Iterator<Item = rusqlite::Result<ThreadRow>>,
) -> rusqlite::Result<Vec<ThreadRow>> {
    iterator.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn seeded_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(crate::infra::db::DB_FILE_NAME);
        let mut conn = crate::infra::db::open_rw(&path).expect("open rw");
        crate::infra::db::migrations::run(&mut conn).expect("migrate");
        for account in [42, 7] {
            conn.execute(
                "INSERT INTO accounts (id, display_name, email, provider,
                                       imap_host, imap_port, imap_security,
                                       smtp_host, smtp_port, smtp_security,
                                       auth_type, created_at)
                 VALUES (?1, 'Test', 'test@example.com', 'imap',
                         'imap.example.com', 993, 'tls',
                         'smtp.example.com', 465, 'tls',
                         'password', 0)",
                rusqlite::params![account],
            )
            .expect("seed account");
        }
        for i in 0..7 {
            conn.execute(
                "INSERT INTO threads (id, account_id, subject_norm, last_date,
                                      message_count, unread_count, has_attachments)
                 VALUES (?1, 42, ?2, ?3, 2, 1, 0)",
                rusqlite::params![1000 + i, format!("s{i}"), 1000 + i],
            )
            .expect("seed thread");
        }
        // Tie-break pair: identical last_date, distinct ids.
        conn.execute(
            "INSERT INTO threads (id, account_id, subject_norm, last_date,
                                  message_count, unread_count, has_attachments)
             VALUES (2000, 42, 'tie-a', 500, 1, 0, 0)",
            [],
        )
        .expect("seed tie a");
        conn.execute(
            "INSERT INTO threads (id, account_id, subject_norm, last_date,
                                  message_count, unread_count, has_attachments)
             VALUES (2001, 42, 'tie-b', 500, 1, 0, 0)",
            [],
        )
        .expect("seed tie b");
        // Seed messages for thread 1000 (newest-first by date_utc desc).
        for i in 0..3 {
            conn.execute(
                "INSERT INTO messages (id, account_id, dedupe_key, subject,
                                      from_json, date_utc, snippet,
                                      has_attachments, body_state, thread_id)
                 VALUES (?1, 42, ?2, 'Thread subject',
                         '[{\"name\":\"Alice\",\"address\":\"alice@example.com\"}]',
                         ?3, 'Snippet for message', 0, 'full', 1000)",
                rusqlite::params![2000 + i, format!("msg-1000-{i}"), 5000 - i as i64],
            )
            .expect("seed message");
        }
        (dir, conn)
    }

    /// First page (no cursor) returns the newest threads in
    /// last_date-desc order.
    #[test]
    fn first_page_lists_newest_first() {
        let (_dir, conn) = seeded_db();
        let page = threads_page(&conn, 42, 3, None).expect("query");
        assert_eq!(page.len(), 3);
        assert_eq!(page[0].id, 1006);
        assert_eq!(page[1].id, 1005);
        assert_eq!(page[2].id, 1004);
    }

    /// Pages chain through the keyset cursor with no gaps and no
    /// repeats; a same-last_date pair is split by id descending.
    #[test]
    fn keyset_cursor_pages_without_overlap() {
        let (_dir, conn) = seeded_db();
        let first = threads_page(&conn, 42, 4, None).expect("page 1");
        let cursor = ThreadCursor {
            last_date: first[3].last_date.expect("seeded date"),
            thread_id: first[3].id,
        };
        let second = threads_page(&conn, 42, 4, Some(cursor)).expect("page 2");
        let cursor2 = ThreadCursor {
            last_date: second[3].last_date.expect("seeded date"),
            thread_id: second[3].id,
        };
        let third = threads_page(&conn, 42, 4, Some(cursor2)).expect("page 3");

        let mut all: Vec<i64> = first
            .iter()
            .chain(&second)
            .chain(&third)
            .map(|r| r.id)
            .collect();
        all.sort_unstable_by_key(|&id| std::cmp::Reverse(id));
        assert_eq!(
            all,
            vec![2001, 2000, 1006, 1005, 1004, 1003, 1002, 1001, 1000]
        );
    }

    /// A page never crosses accounts and stops cleanly past the end.
    #[test]
    fn pages_are_scoped_to_one_account() {
        let (_dir, conn) = seeded_db();
        conn.execute(
            "INSERT INTO threads (id, account_id, subject_norm, last_date,
                                  message_count, unread_count, has_attachments)
             VALUES (3000, 7, 'other', 9999, 1, 0, 0)",
            [],
        )
        .expect("seed other account");
        let page = threads_page(&conn, 42, 50, None).expect("query");
        assert!(page.iter().all(|r| r.account_id == 42));
        assert_eq!(page.len(), 9);
    }

    /// messages_in_thread returns the thread's messages newest-first.
    #[test]
    fn messages_in_thread_lists_newest_first() {
        let (_dir, conn) = seeded_db();
        let page = messages_in_thread(&conn, 1000, 10, None).expect("query");
        assert_eq!(page.len(), 3);
        assert_eq!(page[0].id, 2000);
        assert_eq!(page[1].id, 2001);
        assert_eq!(page[2].id, 2002);
    }

    /// messages_in_thread pages through the keyset cursor without
    /// overlap or repeats.
    #[test]
    fn messages_in_thread_pages_without_overlap() {
        let (_dir, conn) = seeded_db();
        let first = messages_in_thread(&conn, 1000, 2, None).expect("page 1");
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].id, 2000);
        assert_eq!(first[1].id, 2001);
        let cursor = ThreadCursor {
            last_date: first[1].date_utc.expect("seeded date"),
            thread_id: first[1].id,
        };
        let second = messages_in_thread(&conn, 1000, 2, Some(cursor)).expect("page 2");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, 2002);
    }

    /// messages_in_thread returns an empty page for a thread with no
    /// messages and for a nonexistent thread.
    #[test]
    fn messages_in_thread_empty_for_unknown_thread() {
        let (_dir, conn) = seeded_db();
        let page = messages_in_thread(&conn, 9999, 10, None).expect("query");
        assert!(page.is_empty());
    }
}

/// One message's header fields, returned to the reader.
#[derive(Debug, PartialEq)]
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
#[derive(Debug, PartialEq)]
pub struct MessageBody {
    pub message_id: i64,
    pub text_body: Option<String>,
    pub html_clean: Option<String>,
}

/// One attachment summary for the reader's attachment list.
#[derive(Debug, PartialEq)]
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
pub fn message_detail(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
) -> rusqlite::Result<Option<MessageDetail>> {
    let mut stmt = conn.prepare(
        "SELECT id, account_id, subject, from_json, to_json, date_utc,
                 snippet, has_attachments, body_state
         FROM messages
         WHERE id = ?1 AND account_id = ?2",
    )?;
    match stmt.query_row(rusqlite::params![message_id, account_id], |row| {
        Ok(MessageDetail {
            id: row.get(0)?,
            account_id: row.get(1)?,
            subject: row.get(2)?,
            from_json: row.get(3)?,
            to_json: row.get(4)?,
            date_utc: row.get(5)?,
            snippet: row.get(6)?,
            has_attachments: row.get(7)?,
            body_state: row.get(8)?,
        })
    }) {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Returns one message's decoded body by id.
pub fn message_body(conn: &Connection, message_id: i64) -> rusqlite::Result<Option<MessageBody>> {
    let mut stmt = conn.prepare(
        "SELECT message_id, text_body, html_clean
         FROM message_bodies
         WHERE message_id = ?1",
    )?;
    match stmt.query_row(rusqlite::params![message_id], |row| {
        Ok(MessageBody {
            message_id: row.get(0)?,
            text_body: row.get(1)?,
            html_clean: row.get(2)?,
        })
    }) {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Returns one message's attachments by id.
pub fn message_attachments(
    conn: &Connection,
    message_id: i64,
) -> rusqlite::Result<Vec<AttachmentSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, part_id, filename, mime_type, size_bytes,
                 content_id, blob_path
         FROM attachments
         WHERE message_id = ?1
         ORDER BY id",
    )?;
    let rows = stmt.query_map(rusqlite::params![message_id], |row| {
        Ok(AttachmentSummary {
            id: row.get(0)?,
            part_id: row.get(1)?,
            filename: row.get(2)?,
            mime_type: row.get(3)?,
            size_bytes: row.get(4)?,
            content_id: row.get(5)?,
            blob_path: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// Marks a message as read or unread in all its mailbox locations.
pub fn set_message_seen(
    conn: &Connection,
    account_id: i64,
    message_id: i64,
    seen: bool,
) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE message_locations
         SET seen = ?1
         WHERE message_id = ?2
           AND message_id IN (SELECT id FROM messages WHERE account_id = ?3)",
        rusqlite::params![seen as i64, message_id, account_id],
    )
}

#[cfg(test)]
mod reader_tests {
    use super::*;

    fn reader_seeded_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(crate::infra::db::DB_FILE_NAME);
        let mut conn = crate::infra::db::open_rw(&path).expect("open rw");
        crate::infra::db::migrations::run(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO accounts (id, display_name, email, provider, imap_host, imap_port,
                                   imap_security, smtp_host, smtp_port, smtp_security,
                                   auth_type, created_at)
             VALUES (?1, 'Work', 'work@example.com', 'imap', 'imap.example.com', 993,
                     'tls', 'smtp.example.com', 465, 'tls', 'password', 0)",
            rusqlite::params![42],
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
        (dir, conn)
    }

    /// message_detail returns the message header fields.
    #[test]
    fn message_detail_returns_message() {
        let (_dir, conn) = reader_seeded_db();
        let detail = message_detail(&conn, 42, 1).expect("query").expect("row");
        assert_eq!(detail.id, 1);
        assert_eq!(detail.account_id, 42);
        assert_eq!(detail.subject.as_deref(), Some("Reader subject"));
        assert_eq!(detail.body_state, "full");
        assert!(detail.has_attachments);
    }

    /// message_detail returns None for a nonexistent message.
    #[test]
    fn message_detail_none_for_unknown() {
        let (_dir, conn) = reader_seeded_db();
        let detail = message_detail(&conn, 42, 999).expect("query");
        assert!(detail.is_none());
    }

    /// message_detail is scoped to one account.
    #[test]
    fn message_detail_scoped_to_account() {
        let (_dir, conn) = reader_seeded_db();
        let detail = message_detail(&conn, 999, 1).expect("query");
        assert!(detail.is_none());
    }

    /// message_body returns the decoded body.
    #[test]
    fn message_body_returns_body() {
        let (_dir, conn) = reader_seeded_db();
        let body = message_body(&conn, 1).expect("query").expect("row");
        assert_eq!(body.message_id, 1);
        assert_eq!(body.text_body.as_deref(), Some("Plain text body"));
        assert_eq!(body.html_clean.as_deref(), Some("<b>HTML body</b>"));
    }

    /// message_body returns None when no body exists.
    #[test]
    fn message_body_none_when_absent() {
        let (_dir, conn) = reader_seeded_db();
        conn.execute(
            "INSERT INTO messages (id, account_id, dedupe_key, subject,
                                  from_json, date_utc, body_state, thread_id)
             VALUES (?1, 42, 'msg-nobody', 'No body', '[]', 2000, 'headers', 100)",
            rusqlite::params![2],
        )
        .expect("seed message without body");
        let body = message_body(&conn, 2).expect("query");
        assert!(body.is_none());
    }

    /// message_attachments returns the attachment list.
    #[test]
    fn message_attachments_returns_attachments() {
        let (_dir, conn) = reader_seeded_db();
        let attachments = message_attachments(&conn, 1).expect("query");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, 1);
        assert_eq!(attachments[0].part_id, "part-1");
        assert_eq!(attachments[0].filename.as_deref(), Some("report.pdf"));
        assert_eq!(attachments[0].mime_type.as_deref(), Some("application/pdf"));
        assert_eq!(attachments[0].size_bytes, Some(1024));
        assert_eq!(attachments[0].blob_path.as_deref(), Some("ab/cd1234"));
    }

    /// message_attachments returns empty for a message with none.
    #[test]
    fn message_attachments_empty() {
        let (_dir, conn) = reader_seeded_db();
        conn.execute(
            "INSERT INTO messages (id, account_id, dedupe_key, subject,
                                  from_json, date_utc, body_state, thread_id)
             VALUES (?1, 42, 'msg-noatt', 'No attachments', '[]', 2000,
                     'headers', 100)",
            rusqlite::params![3],
        )
        .expect("seed message without attachments");
        let attachments = message_attachments(&conn, 3).expect("query");
        assert!(attachments.is_empty());
    }

    /// set_message_seen marks the message as read.
    #[test]
    fn set_message_seen_marks_read() {
        let (_dir, conn) = reader_seeded_db();
        let count = set_message_seen(&conn, 42, 1, true).expect("update");
        assert_eq!(count, 1);
        let seen: i64 = conn
            .query_row(
                "SELECT seen FROM message_locations WHERE message_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(seen, 1);
    }

    /// set_message_seen marks the message as unread.
    #[test]
    fn set_message_seen_marks_unread() {
        let (_dir, conn) = reader_seeded_db();
        let count = set_message_seen(&conn, 42, 1, false).expect("update");
        assert_eq!(count, 1);
        let seen: i64 = conn
            .query_row(
                "SELECT seen FROM message_locations WHERE message_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(seen, 0);
    }

    /// set_message_seen does nothing for a message in another account.
    #[test]
    fn set_message_seen_scoped_to_account() {
        let (_dir, conn) = reader_seeded_db();
        let count = set_message_seen(&conn, 999, 1, true).expect("update");
        assert_eq!(count, 0);
        let seen: i64 = conn
            .query_row(
                "SELECT seen FROM message_locations WHERE message_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(seen, 0);
    }
}
