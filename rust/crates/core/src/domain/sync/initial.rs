//! Initial sync: mailbox discovery and envelope backfill (ARCHITECTURE §9).
//!
//! Drives the IMAP adapter end to end: LIST → mailbox upserts (kind from
//! special-use flags), SELECT → cursor upserts, chunked `1:*` envelope
//! fetch → parsed headers → message/location/thread writes, publishing
//! `sync_progress` on the bus as mailboxes complete.
//!
//! The sync actor (a later slice) is the first in-crate consumer of
//! `initial_sync`; until it lands the cluster is intentionally unused,
//! so dead-code is allowed module-wide.

#![allow(dead_code)]

use crate::domain::events::EventBus;
use crate::infra::db::writer::{LocationInline, WriteOp, WriterActor};
use crate::infra::imap::ImapError;
use crate::infra::imap::connect::Conn;
use crate::infra::imap::mailbox::{fetch_envelope_chunk, list_mailboxes, select_mailbox};
use crate::infra::mime;
use async_imap::Session;
use mailslate_protocol::message::Event;
use serde_json::json;

/// Hands one operation to the single writer; a dead or failed writer
/// aborts the cycle instead of being silently ignored.
fn write(writer: &WriterActor, op: WriteOp) -> Result<(), ImapError> {
    writer
        .enqueue(op)
        .map_err(|err| ImapError::Writer(err.to_string()))
}

/// Maps a discovered mailbox to its canonical `mailboxes.kind` value
/// (schema CHECK). Special-use flags win; INBOX is its own kind; the
/// rest are user folders.
pub(crate) fn kind_for(name: &str, special_use: Option<&str>) -> &'static str {
    match special_use {
        Some("\\Sent") => "sent",
        Some("\\Drafts") => "drafts",
        Some("\\Trash") => "trash",
        Some("\\Junk") => "spam",
        Some("\\Archive") => "archive",
        Some("\\All") => "all",
        Some("\\Flagged") => "important",
        _ if name.eq_ignore_ascii_case("INBOX") => "inbox",
        _ => "user",
    }
}

/// Fetch priority per ARCHITECTURE §9: Inbox first, then Sent/Drafts/
/// Archive, then user folders (and All/Flagged), then Spam/Trash.
pub(crate) fn sync_rank(kind: &str) -> u8 {
    match kind {
        "inbox" => 0,
        "sent" | "drafts" | "archive" => 1,
        "spam" | "trash" => 3,
        _ => 2,
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_secs() as i64
}

/// Ceiling for one IMAP protocol step inside the initial sync (QA
/// round 1: a hung server must not park the sync actor forever).
const STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Ceiling for one mailbox's envelope fetch — large mailboxes
/// legitimately take minutes (PRD: 5k messages < 5 min on broadband).
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// How many UIDs one `UID FETCH` request may cover — keeps any single
/// request bounded so a huge mailbox never blocks one request (QA#13).
const FETCH_CHUNK: u32 = 500;

/// Runs one protocol step under a deadline, mapping an elapsed timeout
/// to a protocol error so the actor's backoff handles it like any
/// other transient failure.
async fn timed<T>(
    label: &str,
    limit: std::time::Duration,
    step: impl std::future::Future<Output = Result<T, ImapError>>,
) -> Result<T, ImapError> {
    tokio::time::timeout(limit, step)
        .await
        .map_err(|_| ImapError::Protocol(format!("{label} timed out")))?
}

fn flag_set(flags: &[String], name: &str) -> bool {
    flags.iter().any(|f| f == name)
}

/// Runs the full initial sync for one account over an authenticated
/// session: discovery, cursor capture, envelope backfill (a single
/// `1:*` fetch per mailbox — rows carry uids, no SEARCH round-trip),
/// and a `sync_progress` event per completed mailbox.
pub(crate) async fn initial_sync(
    session: &mut Session<Conn>,
    writer: &WriterActor,
    bus: &EventBus,
    account_id: i64,
) -> Result<(), ImapError> {
    let mut mailboxes = timed("list mailboxes", STEP_TIMEOUT, list_mailboxes(session)).await?;
    mailboxes.sort_by_key(|m| sync_rank(kind_for(&m.name, m.special_use.as_deref())));
    let total = mailboxes.len();

    for (index, mailbox) in mailboxes.iter().enumerate() {
        let kind = kind_for(&mailbox.name, mailbox.special_use.as_deref());
        let special_use = mailbox.special_use.clone();
        let name = mailbox.name.clone();

        write(
            writer,
            WriteOp::MailboxUpsert {
                account_id,
                name: name.clone(),
                kind: kind.into(),
                special_use: special_use.clone(),
                uidvalidity: None,
                uidnext: None,
                highest_modseq: None,
            },
        )?;

        let selected = match timed(
            "select mailbox",
            STEP_TIMEOUT,
            select_mailbox(session, &name),
        )
        .await
        {
            Ok(selected) => selected,
            // A mailbox the server refuses to SELECT (ACL, vanished,
            // or unselectable despite the attributes) must not abort
            // the other mailboxes' sync.
            Err(_) => continue,
        };
        write(
            writer,
            WriteOp::MailboxUpsert {
                account_id,
                name: name.clone(),
                kind: kind.into(),
                special_use,
                uidvalidity: selected.uid_validity,
                uidnext: selected.uid_next,
                highest_modseq: selected.highest_modseq,
            },
        )?;

        let rows = timed("envelope fetch", FETCH_TIMEOUT, async {
            let mut all = Vec::new();
            let max_uid = selected.uid_next.map(|n| n.saturating_sub(1)).unwrap_or(0);
            if max_uid >= 1 {
                // Chunked UID ranges: every chunk is its own bounded
                // FETCH; a giant mailbox never stalls one request.
                for start in (1..=max_uid).step_by(FETCH_CHUNK as usize) {
                    let end = (start + FETCH_CHUNK - 1).min(max_uid);
                    let chunk = fetch_envelope_chunk(session, &format!("{start}:{end}")).await?;
                    all.extend(chunk);
                }
            } else {
                // uid_next unavailable: one full-range fetch, the
                // pre-chunking behavior.
                all = fetch_envelope_chunk(session, "1:*").await?;
            }
            Ok(all)
        })
        .await?;
        let now = now_unix();
        for row in rows {
            let header = row.header.clone().unwrap_or_default();
            let record = mime::parse_header(&header);
            let dedupe_key = match record.message_id_hdr.as_deref() {
                Some(id) => format!("mid:{id}"),
                None => format!("imap:{}:{}", name, row.uid.unwrap_or(0)),
            };
            let flags = row.flags.clone();
            let location = LocationInline {
                mailbox_name: name.clone(),
                uid: row.uid.unwrap_or(0),
                seen: flag_set(&flags, "\\Seen"),
                flagged: flag_set(&flags, "\\Flagged"),
                answered: flag_set(&flags, "\\Answered"),
                draft: flag_set(&flags, "\\Draft"),
                deleted: flag_set(&flags, "\\Deleted"),
                keywords_json: "[]".into(),
                modseq: None,
            };
            write(
                writer,
                WriteOp::MessageUpsert {
                    account_id,
                    dedupe_key,
                    message_id_hdr: record.message_id_hdr.clone(),
                    gmail_thread_id: None,
                    root_message_id: record.root_message_id.clone(),
                    subject_norm: mime::subject_norm(record.subject.as_deref()),
                    subject: record.subject.clone(),
                    from_json: record.from_json.clone(),
                    to_json: record.to_json.clone(),
                    cc_json: record.cc_json.clone(),
                    reply_to_json: record.reply_to_json.clone(),
                    date_utc: Some(mime::resolve_date_utc(row.internal_date, &record, now)),
                    size_bytes: row.size.map(i64::from),
                    snippet: None,
                    location: Some(location),
                },
            )?;
        }
        write(writer, WriteOp::ThreadCountersRecompute { account_id })?;

        bus.publish(Event::new(
            "sync_progress",
            json!({"account": account_id, "mailbox": name, "phase": "initial", "done": index + 1, "total": total}),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::imap::testing::{fake_session, spawn_fake};
    use mailslate_protocol::message::Event;
    use std::cell::Cell;

    fn seeded_db(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join(crate::infra::db::DB_FILE_NAME);
        let mut conn = crate::infra::db::open_rw(&path).expect("open");
        crate::infra::db::migrations::run(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO accounts(id, display_name, email, provider, imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, auth_type, created_at)
             VALUES (1, 'Seed', 'seed@example.com', 'imap', 'imap.example.com', 993, 'tls', 'smtp.example.com', 465, 'tls', 'password', 0)",
            [],
        )
        .expect("seed account");
        drop(conn);
        path
    }

    fn fetch_line(
        uid: u32,
        seen: bool,
        size: u32,
        msg_id: &str,
        subject: &str,
        references: &str,
    ) -> String {
        let headers = format!(
            "Message-ID: <{msg_id}>\r\nSubject: {subject}\r\nFrom: Alice <alice@x>\r\nDate: Mon, 19 Oct 2026 00:00:00 +0000\r\nReferences: {references}\r\n\r\n"
        );
        let flags_field = if seen { "\\Seen" } else { "" };
        format!(
            "* 1 FETCH (UID {uid} FLAGS ({flags_field}) RFC822.SIZE {size} INTERNALDATE \"17-Jul-1996 02:44:25 -0700\" BODY[HEADER.FIELDS (DATE FROM TO CC SUBJECT REPLY-TO MESSAGE-ID REFERENCES IN-REPLY-TO)] {{{}}}\r\n{})",
            headers.len(),
            headers
        )
    }

    fn fetch_line_without_message_id(uid: u32, size: u32) -> String {
        // No Message-ID header: the dedupe key must fall back to the
        // mailbox/uid pair.
        let headers =
            "Subject: Headerless\r\nFrom: Bob <bob@x>\r\nDate: Mon, 19 Oct 2026 00:00:00 +0000\r\n\r\n"
                .to_string();
        format!(
            "* 1 FETCH (UID {uid} FLAGS () RFC822.SIZE {size} INTERNALDATE \"17-Jul-1996 02:44:25 -0700\" BODY[HEADER.FIELDS (DATE FROM TO CC SUBJECT REPLY-TO MESSAGE-ID REFERENCES IN-REPLY-TO)] {{{}}}\r\n{})",
            headers.len(),
            headers
        )
    }

    #[test]
    fn kinds_map_from_special_use_and_inbox() {
        assert_eq!(kind_for("INBOX", None), "inbox");
        assert_eq!(kind_for("Sent", Some("\\Sent")), "sent");
        assert_eq!(kind_for("Drafts", Some("\\Drafts")), "drafts");
        assert_eq!(kind_for("Trash", Some("\\Trash")), "trash");
        assert_eq!(kind_for("Junk", Some("\\Junk")), "spam");
        assert_eq!(kind_for("Archive", Some("\\Archive")), "archive");
        assert_eq!(kind_for("All Mail", Some("\\All")), "all");
        assert_eq!(kind_for("Flagged", Some("\\Flagged")), "important");
        assert_eq!(kind_for("Projects", None), "user");
    }

    #[test]
    fn priority_orders_inbox_first_and_trash_last() {
        // Inbox → Sent/Drafts/Archive → user (and all/important) → Spam/Trash.
        // Ties keep their input order (stable sort).
        let mut kinds = ["user", "trash", "archive", "inbox", "spam", "sent"];
        kinds.sort_by_key(|k| sync_rank(k));
        assert_eq!(kinds, ["inbox", "archive", "sent", "user", "trash", "spam"]);
    }

    #[tokio::test]
    async fn initial_sync_skips_unselectable_mailboxes_and_keeps_going() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir);
        let writer = crate::infra::db::writer::WriterActor::spawn(&db_path);

        let selected = Cell::new("");
        let gmail_selects = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let selects = std::sync::Arc::clone(&gmail_selects);
        let port = spawn_fake(move |tag: &str, command: &str| {
            if command.starts_with("LOGIN") {
                vec![format!("{tag} OK logged in")]
            } else if command.starts_with("LIST") {
                vec![
                    "* LIST (\\HasChildren \\Noselect) \"/\" \"[Gmail]\"".to_string(),
                    "* LIST (\\HasNoChildren) \"/\" INBOX".to_string(),
                    "* LIST (\\HasNoChildren) \"/\" \"Broken\"".to_string(),
                    "* LIST (\\HasNoChildren \\Archive) \"/\" \"Archive\"".to_string(),
                    format!("{tag} OK list done"),
                ]
            } else if command.starts_with("SELECT") {
                if command.contains("[Gmail]") {
                    selects.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                if command.contains("Broken") {
                    // ACL-denied or vanished mailbox: the sync must skip
                    // it instead of aborting every other mailbox.
                    vec![format!("{tag} NO [CANNOT] not selectable")]
                } else {
                    selected.set(if command.contains("Archive") {
                        "Archive"
                    } else {
                        "INBOX"
                    });
                    vec![
                        "* 1 EXISTS".to_string(),
                        "* OK [UIDVALIDITY 100] UIDs valid".to_string(),
                        "* OK [UIDNEXT 2] Predicted".to_string(),
                        format!("{tag} OK select done"),
                    ]
                }
            } else if command.starts_with("UID FETCH") {
                let (uid, id) = if selected.get() == "Archive" {
                    (7, "s1")
                } else {
                    (1, "r1")
                };
                vec![
                    fetch_line(uid, false, 500, id, "Subject", ""),
                    format!("{tag} OK fetch done"),
                ]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        let mut session = fake_session(port).await;
        let bus = crate::domain::events::EventBus::new();

        initial_sync(&mut session, &writer, &bus, 1)
            .await
            .expect("one unselectable mailbox must not abort the sync");
        writer.flush().expect("flush");
        drop(session);

        assert_eq!(
            gmail_selects.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the \\Noselect container must never be selected"
        );
        let conn = crate::infra::db::open_readonly(&db_path).expect("readonly");
        let messages: i64 = conn
            .query_row(
                "SELECT count(*) FROM messages WHERE account_id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(messages, 2, "both selectable mailboxes must be imported");
        let gmail_rows: i64 = conn
            .query_row(
                "SELECT count(*) FROM mailboxes WHERE account_id = 1 AND name = '[Gmail]'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(gmail_rows, 0, "the container is not a mailbox row");
    }

    #[tokio::test]
    async fn initial_sync_upserts_mailboxes_messages_and_threads() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir);
        let writer = crate::infra::db::writer::WriterActor::spawn(&db_path);

        let selected = Cell::new(0usize);
        let port = spawn_fake(move |tag: &str, command: &str| {
            if command.starts_with("LOGIN") {
                vec![format!("{tag} OK logged in")]
            } else if command.starts_with("LIST") {
                vec![
                    "* LIST (\\HasNoChildren) \"/\" INBOX".to_string(),
                    "* LIST (\\HasNoChildren \\Sent) \"/\" \"Sent\"".to_string(),
                    format!("{tag} OK list done"),
                ]
            } else if command.starts_with("SELECT") {
                let is_inbox = command.contains("INBOX");
                selected.set(if is_inbox { 0 } else { 1 });
                let (exists, validity, uidnext) = if is_inbox { (2, 100, 3) } else { (1, 200, 2) };
                vec![
                    format!("* {exists} EXISTS"),
                    format!("* OK [UIDVALIDITY {validity}] UIDs valid"),
                    format!("* OK [UIDNEXT {uidnext}] Predicted"),
                    format!("{tag} OK select done"),
                ]
            } else if command.starts_with("UID FETCH") {
                if selected.get() == 0 {
                    vec![
                        fetch_line(1, true, 500, "a@x", "Re: Hello", "<r1>"),
                        fetch_line(2, false, 600, "b@x", "Re: Hello", "<r2>"),
                        format!("{tag} OK fetch done"),
                    ]
                } else {
                    vec![
                        fetch_line(7, false, 700, "c@x", "Status report", "<r9>"),
                        format!("{tag} OK fetch done"),
                    ]
                }
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        let mut session = fake_session(port).await;
        let bus = crate::domain::events::EventBus::new();
        let mut feed = bus.subscribe();

        initial_sync(&mut session, &writer, &bus, 1)
            .await
            .expect("initial sync");

        writer.flush().expect("flush");
        drop(session);

        let conn = crate::infra::db::open_readonly(&db_path).expect("readonly");
        let (name, kind, validity, uidnext): (String, String, i64, i64) = conn
            .query_row(
                "SELECT name, kind, uidvalidity, uidnext FROM mailboxes WHERE account_id = 1 AND name = 'INBOX'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((name.as_str(), kind.as_str()), ("INBOX", "inbox"));
        assert_eq!((validity, uidnext), (100, 3));

        let sent_kind: String = conn
            .query_row(
                "SELECT kind FROM mailboxes WHERE account_id = 1 AND name = 'Sent'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sent_kind, "sent");

        let messages: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(messages, 3);

        let threads: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE account_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            threads, 2,
            "a@x and b@x cluster by subject_norm; c@x stands alone"
        );

        let threaded: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id = 1 AND thread_id IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(threaded, 3);

        let (locations, seen): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(seen) FROM message_locations",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((locations, seen), (3, 1));
        drop(conn);

        let mut progress = Vec::new();
        while let Ok(event) = feed.try_recv() {
            if event.event == "sync_progress" {
                progress.push(event);
            }
        }
        assert!(
            progress
                .iter()
                .any(|e: &Event| e.data["mailbox"] == "INBOX" && e.data["phase"] == "initial"),
            "expected an INBOX sync_progress event, got {progress:?}"
        );
        assert!(progress.iter().any(|e| e.data["mailbox"] == "Sent"));
    }

    /// A SELECT without `[UIDNEXT]` falls back to one full-range
    /// fetch, and a message without a Message-ID header dedupes on
    /// its mailbox/uid pair instead of the header.
    #[tokio::test]
    async fn initial_sync_fetches_the_full_range_without_uidnext_and_dedupes_without_message_id() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir);
        let writer = crate::infra::db::writer::WriterActor::spawn(&db_path);

        let port = spawn_fake(|tag: &str, command: &str| {
            if command.starts_with("LOGIN") {
                vec![format!("{tag} OK logged in")]
            } else if command.starts_with("LIST") {
                vec![
                    "* LIST (\\HasNoChildren) \"/\" INBOX".to_string(),
                    format!("{tag} OK list done"),
                ]
            } else if command.starts_with("SELECT") {
                // Deliberately no [UIDNEXT] code: the envelope fetch
                // must fall back to the full `1:*` range.
                vec![
                    "* 1 EXISTS".to_string(),
                    "* OK [UIDVALIDITY 100] UIDs valid".to_string(),
                    format!("{tag} OK select done"),
                ]
            } else if command.starts_with("UID FETCH") {
                vec![
                    fetch_line_without_message_id(5, 123),
                    format!("{tag} OK fetch done"),
                ]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        let mut session = fake_session(port).await;
        let bus = crate::domain::events::EventBus::new();

        initial_sync(&mut session, &writer, &bus, 1)
            .await
            .expect("initial sync");
        writer.flush().expect("flush");
        drop(session);

        let conn = crate::infra::db::open_readonly(&db_path).expect("readonly");
        let (dedupe_key, message_id_hdr): (String, Option<String>) = conn
            .query_row(
                "SELECT dedupe_key, message_id_hdr FROM messages WHERE account_id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(dedupe_key, "imap:INBOX:5", "uid-based fallback key");
        assert_eq!(message_id_hdr, None);
    }

    #[tokio::test]
    async fn envelope_fetch_is_chunked_at_500_uids() {
        use std::sync::{Arc, Mutex};
        let fetched: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let fetched_clone = Arc::clone(&fetched);
        let call_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let count_clone = Arc::clone(&call_count);
        let dir = tempfile::tempdir().unwrap();
        let db_path = seeded_db(&dir);
        let writer = crate::infra::db::writer::WriterActor::spawn(&db_path);

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
                    "* 1200 EXISTS".to_string(),
                    "* OK [UIDVALIDITY 10] UIDs valid".to_string(),
                    "* OK [UIDNEXT 1201] Predicted".to_string(),
                    format!("{tag} OK select done"),
                ]
            } else if command.starts_with("UID FETCH") {
                let set = command
                    .strip_prefix("UID FETCH ")
                    .and_then(|rest| rest.split_whitespace().next())
                    .unwrap_or("?");
                fetched_clone.lock().unwrap().push(set.to_string());
                let call = *count_clone.lock().unwrap();
                *count_clone.lock().unwrap() = call + 1;
                let mut rows = (1..=2)
                    .map(|uid| {
                        fetch_line(
                            uid,
                            false,
                            100,
                            &format!("c{call}-{uid}@x"),
                            "S",
                            &format!("<c{call}-{uid}>"),
                        )
                    })
                    .collect::<Vec<_>>();
                rows.push(format!("{tag} OK fetch done"));
                rows
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        let mut session = fake_session(port).await;
        let bus = crate::domain::events::EventBus::new();
        initial_sync(&mut session, &writer, &bus, 1)
            .await
            .expect("initial sync");
        writer.flush().expect("flush");

        let sets = fetched.lock().unwrap().clone();
        assert_eq!(
            sets,
            vec!["1:500", "501:1000", "1001:1200"],
            "expected three 500-UID chunks plus a trailing partial chunk, got {sets:?}"
        );

        let conn = crate::infra::db::open_readonly(&db_path).expect("readonly");
        let messages: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            messages, 6,
            "2 rows per chunk x 3 chunks, all distinct message ids"
        );
        drop(conn);
    }
}
