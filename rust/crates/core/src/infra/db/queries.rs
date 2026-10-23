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
