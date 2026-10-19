//! Single-writer actor: all mutating DB access goes through one
//! dedicated thread that applies queued operations in batched
//! transactions (ARCHITECTURE §12: one SQLite writer; WAL readers
//! never block it).

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use rusqlite::Connection;

use crate::infra::db::{self, migrations};

/// How many operations fit into one batched transaction target.
pub const BATCH_OPS: usize = 500;

/// How long the writer waits before flushing a partial batch.
pub const BATCH_WINDOW: Duration = Duration::from_millis(50);

/// Operations the writer applies. The set grows with upcoming
/// phases (accounts, messages, mutations); meta writes cover the
/// first behavior slice: enqueue → durable, in order.
pub enum WriteOp {
    /// Upserts one `meta` key/value row.
    MetaPut { key: String, value: String },
    /// Inserts one account row (phase 3).
    AccountAdd {
        id: i64,
        display_name: String,
        email: String,
        provider: String,
        imap_host: String,
        imap_port: u16,
        imap_security: String,
        smtp_host: String,
        smtp_port: u16,
        smtp_security: String,
        auth_type: String,
    },
    /// Internal: ack after every queued op before it is committed.
    Flush(mpsc::Sender<()>),
    /// Deletes one account row (children cascade via FK).
    AccountRemove { id: i64 },
    /// Patches one account's mutable fields; `None` fields keep the
    /// stored value, `Some(None)` on color clears it.
    AccountUpdate {
        id: i64,
        display_name: Option<String>,
        color: Option<Option<String>>,
        sync_window_days: Option<i64>,
    },
    /// Upserts one mailbox row keyed by (account_id, name) — the
    /// sync engine's LIST/SELECT projection.
    ///
    /// Constructed by the sync engine slices; allowed dead until
    /// they land.
    #[allow(dead_code)]
    MailboxUpsert {
        account_id: i64,
        name: String,
        kind: String,
        special_use: Option<String>,
        uidvalidity: Option<u32>,
        uidnext: Option<u32>,
        highest_modseq: Option<u64>,
    },
    /// Upserts one message row keyed by (account_id, dedupe_key).
    /// Never downgrades `body_state`: it only ever moves
    /// headers→full via a dedicated op later.
    ///
    /// Constructed by the sync engine slices; allowed dead until
    /// they land.
    #[allow(dead_code)]
    MessageUpsert {
        account_id: i64,
        dedupe_key: String,
        message_id_hdr: Option<String>,
        thread_id: Option<i64>,
        subject: Option<String>,
        from_json: String,
        to_json: String,
        cc_json: String,
        reply_to_json: String,
        date_utc: Option<i64>,
        size_bytes: Option<i64>,
        snippet: Option<String>,
    },
    /// Upserts one message_locations row keyed by (mailbox_id, uid).
    ///
    /// Constructed by the sync engine slices; allowed dead until
    /// they land.
    #[allow(dead_code)]
    LocationUpsert {
        message_id: i64,
        mailbox_id: i64,
        uid: u32,
        seen: bool,
        flagged: bool,
        answered: bool,
        draft: bool,
        deleted: bool,
        keywords_json: String,
        modseq: Option<u64>,
    },
    /// Upserts one thread row. With a Gmail thrid it dedupes on the
    /// partial unique index (last_date moves forward, None fields
    /// keep stored values); without one it inserts a fresh row —
    /// generic JWZ clustering is the sync engine's job.
    ///
    /// Constructed by the sync engine slices; allowed dead until
    /// they land.
    #[allow(dead_code)]
    ThreadUpsert {
        account_id: i64,
        gmail_thread_id: Option<String>,
        root_message_id: Option<String>,
        subject_norm: Option<String>,
        last_date: i64,
    },
}

/// Handle for enqueueing writes; `Drop` joins the actor after
/// draining what is already queued.
pub struct WriterActor {
    tx: mpsc::Sender<WriteOp>,
}

impl WriterActor {
    /// Starts the actor thread against the database at `db_path`
    /// (it must already exist and be migrated).
    pub fn spawn(db_path: &Path) -> Self {
        let conn = open_writer(db_path);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || writer_loop(conn, rx));
        Self { tx }
    }

    /// Queues one operation; returns once the actor has buffered it
    /// (delivery, not durability, at this point).
    pub fn enqueue(&self, op: WriteOp) {
        self.tx.send(op).expect("writer thread alive");
    }

    /// Blocks until every operation enqueued before this call has
    /// been committed (FIFO channel order guarantees the ack lands
    /// behind the ops it covers).
    pub fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(WriteOp::Flush(tx))
            .expect("writer thread alive");
        rx.recv().expect("writer acks its batches");
    }
}

fn open_writer(db_path: &Path) -> Connection {
    let mut conn = db::open_rw(db_path).expect("writer db open");
    migrations::run(&mut conn).expect("writer db migrations");
    conn
}

/// Stolen from mod.rs — GLM PORTER: keep in sync with the real one so
/// scratch does not drift.
fn writer_loop(mut conn: Connection, rx: mpsc::Receiver<WriteOp>) {
    // Batched transactions: up to BATCH_OPS ops or BATCH_WINDOW,
    // whichever comes first, then one COMMIT. WAL readers see each
    // batch atomically.
    let mut batch: Vec<WriteOp> = Vec::with_capacity(BATCH_OPS);
    loop {
        match rx.recv_timeout(BATCH_WINDOW) {
            Ok(op) => batch.push(op),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                flush_batch(&mut conn, &batch);
                return;
            }
        }
        // Opportunistic drain: grab whatever else is ready without
        // blocking, up to the batch size.
        while batch.len() < BATCH_OPS {
            match rx.try_recv() {
                Ok(op) => batch.push(op),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    flush_batch(&mut conn, &batch);
                    return;
                }
            }
        }
        flush_batch(&mut conn, &batch);
        batch.clear();
    }
}

/// Applies one batch inside a single transaction. Returns true when
/// the batch contained a `Flush` marker (ack handled, loop continues).
fn flush_batch(conn: &mut Connection, batch: &[WriteOp]) -> bool {
    if batch.is_empty() {
        return false;
    }
    let tx = conn.transaction().expect("writer transaction");
    for op in batch {
        match op {
            WriteOp::MetaPut { key, value } => {
                tx.execute(
                    "INSERT INTO meta(key, value) VALUES(?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [key, value],
                )
                .expect("meta upsert");
            }
            WriteOp::AccountAdd {
                id,
                display_name,
                email,
                provider,
                imap_host,
                imap_port,
                imap_security,
                smtp_host,
                smtp_port,
                smtp_security,
                auth_type,
            } => {
                tx.execute(
                    "INSERT INTO accounts(
                        id, display_name, email, provider,
                        imap_host, imap_port, imap_security,
                        smtp_host, smtp_port, smtp_security, auth_type, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, unixepoch())",
                    rusqlite::params![
                        id,
                        display_name,
                        email,
                        provider,
                        imap_host,
                        *imap_port as i64,
                        imap_security,
                        smtp_host,
                        *smtp_port as i64,
                        smtp_security,
                        auth_type
                    ],
                )
                .expect("account insert");
            }
            WriteOp::AccountRemove { id } => {
                // Children cascade via FK (mailboxes, messages,
                // locations, bodies, attachments, tags, outbox).
                tx.execute("DELETE FROM accounts WHERE id = ?1", [id])
                    .expect("account delete");
            }
            WriteOp::AccountUpdate {
                id,
                display_name,
                color,
                sync_window_days,
            } => {
                // COALESCE keeps stored values for None; the color's
                // outer None = untouched, inner None = clear to NULL
                // (?3 marks presence, ?4 carries the value-or-NULL).
                tx.execute(
                    "UPDATE accounts SET
                        display_name = COALESCE(?2, display_name),
                        color = CASE WHEN ?3 THEN ?4 ELSE color END,
                        sync_window_days = COALESCE(?5, sync_window_days)
                     WHERE id = ?1",
                    rusqlite::params![
                        id,
                        display_name,
                        color.is_some(),
                        color.as_ref().cloned().flatten(),
                        sync_window_days
                    ],
                )
                .expect("account update");
            }
            WriteOp::MailboxUpsert {
                account_id,
                name,
                kind,
                special_use,
                uidvalidity,
                uidnext,
                highest_modseq,
            } => {
                tx.execute(
                    "INSERT INTO mailboxes(
                        account_id, name, kind, special_use,
                        uidvalidity, uidnext, highest_modseq
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(account_id, name) DO UPDATE SET
                        kind = excluded.kind,
                        special_use = excluded.special_use,
                        uidvalidity = excluded.uidvalidity,
                        uidnext = excluded.uidnext,
                        highest_modseq = excluded.highest_modseq",
                    rusqlite::params![
                        account_id,
                        name,
                        kind,
                        special_use,
                        uidvalidity.map(|v| v as i64),
                        uidnext.map(|v| v as i64),
                        highest_modseq.map(|v| v as i64)
                    ],
                )
                .expect("mailbox upsert");
            }
            WriteOp::MessageUpsert {
                account_id,
                dedupe_key,
                message_id_hdr,
                thread_id,
                subject,
                from_json,
                to_json,
                cc_json,
                reply_to_json,
                date_utc,
                size_bytes,
                snippet,
            } => {
                // body_state is deliberately absent from the UPDATE:
                // a header re-sync must never downgrade a fetched body.
                tx.execute(
                    "INSERT INTO messages(
                        account_id, dedupe_key, message_id_hdr, thread_id,
                        subject, from_json, to_json, cc_json, reply_to_json,
                        date_utc, size_bytes, snippet
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT(account_id, dedupe_key) DO UPDATE SET
                        message_id_hdr = excluded.message_id_hdr,
                        thread_id = excluded.thread_id,
                        subject = excluded.subject,
                        from_json = excluded.from_json,
                        to_json = excluded.to_json,
                        cc_json = excluded.cc_json,
                        reply_to_json = excluded.reply_to_json,
                        date_utc = excluded.date_utc,
                        size_bytes = excluded.size_bytes,
                        snippet = excluded.snippet",
                    rusqlite::params![
                        account_id,
                        dedupe_key,
                        message_id_hdr,
                        thread_id,
                        subject,
                        from_json,
                        to_json,
                        cc_json,
                        reply_to_json,
                        date_utc,
                        size_bytes,
                        snippet
                    ],
                )
                .expect("message upsert");
            }
            WriteOp::LocationUpsert {
                message_id,
                mailbox_id,
                uid,
                seen,
                flagged,
                answered,
                draft,
                deleted,
                keywords_json,
                modseq,
            } => {
                tx.execute(
                    "INSERT INTO message_locations(
                        message_id, mailbox_id, uid,
                        seen, flagged, answered, draft, deleted,
                        keywords_json, modseq
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(mailbox_id, uid) DO UPDATE SET
                        message_id = excluded.message_id,
                        seen = excluded.seen,
                        flagged = excluded.flagged,
                        answered = excluded.answered,
                        draft = excluded.draft,
                        deleted = excluded.deleted,
                        keywords_json = excluded.keywords_json,
                        modseq = excluded.modseq",
                    rusqlite::params![
                        message_id,
                        mailbox_id,
                        *uid as i64,
                        *seen as i64,
                        *flagged as i64,
                        *answered as i64,
                        *draft as i64,
                        *deleted as i64,
                        keywords_json,
                        modseq.map(|v| v as i64)
                    ],
                )
                .expect("location upsert");
            }
            WriteOp::ThreadUpsert {
                account_id,
                gmail_thread_id,
                root_message_id,
                subject_norm,
                last_date,
            } => {
                match gmail_thread_id {
                    // Partial unique index target: only thrid rows
                    // conflict; last_date moves forward, identity
                    // fields keep their stored values on None.
                    Some(thrid) => {
                        tx.execute(
                            "INSERT INTO threads(
                                account_id, gmail_thread_id, root_message_id,
                                subject_norm, last_date
                             ) VALUES (?1, ?2, ?3, ?4, ?5)
                             ON CONFLICT(account_id, gmail_thread_id)
                                 WHERE gmail_thread_id IS NOT NULL
                             DO UPDATE SET
                                root_message_id = COALESCE(excluded.root_message_id, threads.root_message_id),
                                subject_norm = COALESCE(excluded.subject_norm, threads.subject_norm),
                                last_date = MAX(threads.last_date, excluded.last_date)",
                            rusqlite::params![
                                account_id,
                                thrid,
                                root_message_id,
                                subject_norm,
                                last_date
                            ],
                        )
                        .expect("gmail thread upsert");
                    }
                    // No thrid: plain insert — clustering across
                    // references is decided by the sync engine.
                    None => {
                        tx.execute(
                            "INSERT INTO threads(
                                account_id, root_message_id, subject_norm, last_date
                             ) VALUES (?1, ?2, ?3, ?4)",
                            rusqlite::params![account_id, root_message_id, subject_norm, last_date],
                        )
                        .expect("generic thread insert");
                    }
                }
            }
            WriteOp::Flush(ack) => {
                // Commit what precedes the marker, then ack: the ack
                // sender only observes the batch boundary it asked for.
                tx.commit().expect("writer commit");
                let _ = ack.send(());
                return true;
            }
        }
    }
    tx.commit().expect("writer commit");
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hundreds of queued operations all land durably, in enqueue
    /// order, once the actor has drained them.
    #[test]
    fn writer_applies_queued_operations() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(super::super::DB_FILE_NAME);
        // The db must be migrated before spawn (spawn opens rw+migrate;
        // double migration is a no-op anyway).
        let mut seed = crate::infra::db::open_rw(&path).expect("open seed");
        migrations::run(&mut seed).expect("migrate seed");
        drop(seed);

        let writer = WriterActor::spawn(&path);
        for i in 0..1000 {
            writer.enqueue(WriteOp::MetaPut {
                key: format!("key-{i}"),
                value: format!("value-{i}"),
            });
        }
        writer.flush();

        let conn = crate::infra::db::open_readonly(&path).expect("re-open");
        for i in 0..1000 {
            let value: String = conn
                .query_row(
                    "SELECT value FROM meta WHERE key = ?1",
                    [format!("key-{i}")],
                    |row| row.get(0),
                )
                .expect("queued op must be applied");
            assert_eq!(value, format!("value-{i}"), "op {i} must be durable");
        }
    }

    /// Seeds one account row directly (FK target for sync ops).
    fn seed_account(path: &std::path::Path, id: i64) {
        let mut conn = crate::infra::db::open_rw(path).expect("seed open");
        migrations::run(&mut conn).expect("seed migrate");
        conn.execute(
            "INSERT INTO accounts(
                id, display_name, email, provider,
                imap_host, imap_port, imap_security,
                smtp_host, smtp_port, smtp_security, auth_type, created_at
             ) VALUES (?1, 'Seed', 'seed@example.com', 'imap',
                'imap.example.com', 993, 'tls',
                'smtp.example.com', 465, 'tls', 'password', 0)",
            [id],
        )
        .expect("seed account");
        drop(conn);
    }

    #[test]
    fn mailbox_upsert_inserts_then_updates_in_place() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(super::super::DB_FILE_NAME);
        seed_account(&path, 1);

        let writer = WriterActor::spawn(&path);
        writer.enqueue(WriteOp::MailboxUpsert {
            account_id: 1,
            name: "INBOX".into(),
            kind: "inbox".into(),
            special_use: None,
            uidvalidity: Some(42),
            uidnext: Some(10),
            highest_modseq: Some(7),
        });
        writer.flush();
        writer.enqueue(WriteOp::MailboxUpsert {
            account_id: 1,
            name: "INBOX".into(),
            kind: "inbox".into(),
            special_use: Some("\\Sent".into()),
            uidvalidity: Some(42),
            uidnext: Some(11),
            highest_modseq: Some(9),
        });
        writer.flush();

        let conn = crate::infra::db::open_readonly(&path).expect("re-open");
        let (count, uidnext, highest, special): (i64, i64, i64, Option<String>) = conn
            .query_row(
                "SELECT COUNT(*), MAX(uidnext), MAX(highest_modseq), MAX(special_use)
                 FROM mailboxes WHERE account_id = 1 AND name = 'INBOX'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("mailbox row");
        assert_eq!(count, 1, "upsert must not duplicate");
        assert_eq!(uidnext, 11);
        assert_eq!(highest, 9);
        assert_eq!(special.as_deref(), Some("\\Sent"));
    }

    #[test]
    fn message_upsert_dedupes_by_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(super::super::DB_FILE_NAME);
        seed_account(&path, 1);

        let writer = WriterActor::spawn(&path);
        writer.enqueue(WriteOp::MailboxUpsert {
            account_id: 1,
            name: "INBOX".into(),
            kind: "inbox".into(),
            special_use: None,
            uidvalidity: None,
            uidnext: None,
            highest_modseq: None,
        });
        writer.enqueue(WriteOp::ThreadUpsert {
            account_id: 1,
            gmail_thread_id: Some("gm-1".into()),
            root_message_id: None,
            subject_norm: Some("hello".into()),
            last_date: 100,
        });
        writer.flush();

        let conn = crate::infra::db::open_readonly(&path).expect("re-open");
        let thread_id: i64 = conn
            .query_row(
                "SELECT id FROM threads WHERE gmail_thread_id = 'gm-1'",
                [],
                |row| row.get(0),
            )
            .expect("thread row");
        drop(conn);

        let message = |subject: &str, date_utc: i64| WriteOp::MessageUpsert {
            account_id: 1,
            dedupe_key: "mid:abc@example.com".into(),
            message_id_hdr: Some("abc@example.com".into()),
            thread_id: Some(thread_id),
            subject: Some(subject.into()),
            from_json: "[]".into(),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            reply_to_json: "[]".into(),
            date_utc: Some(date_utc),
            size_bytes: Some(1024),
            snippet: Some("body text".into()),
        };
        writer.enqueue(message("Hello", 500));
        writer.flush();
        writer.enqueue(message("Hello v2", 600));
        writer.flush();

        let conn = crate::infra::db::open_readonly(&path).expect("re-open");
        let (count, subject, date_utc, thread, body_state): (i64, String, i64, i64, String) = conn
            .query_row(
                "SELECT COUNT(*), MAX(subject), MAX(date_utc), MAX(thread_id), MAX(body_state)
                 FROM messages WHERE account_id = 1 AND dedupe_key = 'mid:abc@example.com'",
                [],
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
            .expect("message row");
        assert_eq!(count, 1, "dedupe key must not duplicate");
        assert_eq!(subject, "Hello v2");
        assert_eq!(date_utc, 600);
        assert_eq!(thread, thread_id);
        assert_eq!(body_state, "headers");
    }

    #[test]
    fn location_upsert_reuses_the_slot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(super::super::DB_FILE_NAME);
        seed_account(&path, 1);

        let writer = WriterActor::spawn(&path);
        writer.enqueue(WriteOp::MailboxUpsert {
            account_id: 1,
            name: "INBOX".into(),
            kind: "inbox".into(),
            special_use: None,
            uidvalidity: None,
            uidnext: None,
            highest_modseq: None,
        });
        writer.enqueue(WriteOp::MessageUpsert {
            account_id: 1,
            dedupe_key: "mid:loc@example.com".into(),
            message_id_hdr: Some("loc@example.com".into()),
            thread_id: None,
            subject: None,
            from_json: "[]".into(),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            reply_to_json: "[]".into(),
            date_utc: None,
            size_bytes: None,
            snippet: None,
        });
        writer.flush();

        let conn = crate::infra::db::open_readonly(&path).expect("re-open");
        let mailbox_id: i64 = conn
            .query_row("SELECT id FROM mailboxes WHERE name = 'INBOX'", [], |row| {
                row.get(0)
            })
            .expect("mailbox row");
        let message_id: i64 = conn
            .query_row(
                "SELECT id FROM messages WHERE dedupe_key = 'mid:loc@example.com'",
                [],
                |row| row.get(0),
            )
            .expect("message row");
        drop(conn);

        let location = |seen: bool, flagged: bool, modseq: u64| WriteOp::LocationUpsert {
            message_id,
            mailbox_id,
            uid: 5,
            seen,
            flagged,
            answered: false,
            draft: false,
            deleted: false,
            keywords_json: "[]".into(),
            modseq: Some(modseq),
        };
        writer.enqueue(location(true, false, 10));
        writer.flush();
        writer.enqueue(location(false, true, 11));
        writer.flush();

        let conn = crate::infra::db::open_readonly(&path).expect("re-open");
        let (count, seen, flagged, modseq): (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), MAX(seen), MAX(flagged), MAX(modseq)
                 FROM message_locations WHERE mailbox_id = ?1 AND uid = 5",
                [mailbox_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("location row");
        assert_eq!(count, 1, "same (mailbox, uid) slot must be reused");
        assert_eq!(seen, 0);
        assert_eq!(flagged, 1);
        assert_eq!(modseq, 11);
    }

    #[test]
    fn thread_upsert_gmail_path_dedupes_by_thrid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(super::super::DB_FILE_NAME);
        seed_account(&path, 1);

        let writer = WriterActor::spawn(&path);
        writer.enqueue(WriteOp::ThreadUpsert {
            account_id: 1,
            gmail_thread_id: Some("gm-9".into()),
            root_message_id: Some("<r1>".into()),
            subject_norm: Some("s".into()),
            last_date: 100,
        });
        writer.flush();
        writer.enqueue(WriteOp::ThreadUpsert {
            account_id: 1,
            gmail_thread_id: Some("gm-9".into()),
            root_message_id: None,
            subject_norm: None,
            last_date: 200,
        });
        writer.flush();
        writer.enqueue(WriteOp::ThreadUpsert {
            account_id: 1,
            gmail_thread_id: None,
            root_message_id: Some("<r2>".into()),
            subject_norm: Some("t".into()),
            last_date: 50,
        });
        writer.flush();

        let conn = crate::infra::db::open_readonly(&path).expect("re-open");
        let (count, last_date, root): (i64, i64, Option<String>) = conn
            .query_row(
                "SELECT COUNT(*), MAX(last_date), MAX(root_message_id)
                 FROM threads WHERE account_id = 1 AND gmail_thread_id = 'gm-9'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("gmail thread row");
        assert_eq!(count, 1, "thrid must dedupe");
        assert_eq!(last_date, 200, "last_date moves forward");
        assert_eq!(
            root.as_deref(),
            Some("<r1>"),
            "None fields keep stored values"
        );

        let generic: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE account_id = 1 AND root_message_id = '<r2>'",
                [],
                |row| row.get(0),
            )
            .expect("generic thread row");
        assert_eq!(generic, 1);
    }
}
