//! Single-writer actor: all mutating DB access goes through one
//! dedicated thread that applies queued operations in batched
//! transactions (ARCHITECTURE §12: one SQLite writer; WAL readers
//! never block it).

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension};

use crate::infra::db::{self, migrations};

/// How many operations fit into one batched transaction target.
pub const BATCH_OPS: usize = 500;

/// How long the writer waits before flushing a partial batch.
pub const BATCH_WINDOW: Duration = Duration::from_millis(50);

/// One message_locations slot carried inline with a
/// [`WriteOp::MessageUpsert`], keyed by mailbox NAME so the writer
/// can resolve both foreign ids inside its own transaction (the
/// sync actor cannot read row ids back synchronously).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LocationInline {
    pub mailbox_name: String,
    pub uid: u32,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub draft: bool,
    pub deleted: bool,
    pub keywords_json: String,
    pub modseq: Option<u64>,
}

/// Failure of the writer surface: the caller learns that the operation
/// may not be durable instead of the actor panicking and taking the
/// single SQLite writer down with it.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// The writer thread has exited; the op was not queued.
    #[error("the writer thread has stopped")]
    Closed,
    /// The batch behind a flush marker did not commit in time.
    #[error("the writer did not acknowledge the flush in time")]
    Timeout,
    /// The batched transaction could not be committed.
    #[error("the writer failed to commit: {0}")]
    Failed(String),
}

/// Outcome of committing the batch segment that ends at a
/// [`WriteOp::Flush`] marker.
pub type FlushResult = Result<(), String>;

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
    Flush(mpsc::Sender<FlushResult>),
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
        subject: Option<String>,
        from_json: String,
        to_json: String,
        cc_json: String,
        reply_to_json: String,
        date_utc: Option<i64>,
        size_bytes: Option<i64>,
        snippet: Option<String>,
        /// Gmail X-GM-THRID when the server advertises X-GM-EXT-1;
        /// resolution moves writer-side (see [`LocationInline`]).
        gmail_thread_id: Option<String>,
        /// First References entry (else In-Reply-To), angle brackets
        /// stripped — clusters generic threads.
        root_message_id: Option<String>,
        /// Lowercased subject with reply prefixes stripped.
        subject_norm: Option<String>,
        /// One message_locations slot keyed by mailbox NAME, resolved
        /// inside the same transaction (the actor cannot read back
        /// row ids synchronously).
        location: Option<LocationInline>,
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
    /// Recomputes denormalized thread counters (message_count,
    /// unread_count) for every thread of the account. Called after a
    /// batch of message upserts; counters must match the locations.
    #[allow(dead_code)]
    ThreadCountersRecompute { account_id: i64 },
}

/// Handle for enqueueing writes; `Drop` closes the channel first
/// (letting the actor drain, commit, and exit) and then joins, so
/// dropping the last handle flushes and releases the SQLite
/// connection deterministically. Joining before the last `Sender`
/// is dropped would deadlock: the actor would wait forever on a
/// channel that never disconnects.
pub struct WriterActor {
    tx: Option<mpsc::Sender<WriteOp>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WriterActor {
    fn drop(&mut self) {
        // Drop the Sender BEFORE joining: the actor loop only
        // observes Disconnected once the channel is closed, then
        // flushes whatever is queued and exits.
        drop(self.tx.take());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl WriterActor {
    /// Starts the actor thread against the database at `db_path`
    /// (it must already exist and be migrated). Failed operations
    /// are silently skipped; prefer [`WriterActor::spawn_with_error_hook`].
    pub fn spawn(db_path: &Path) -> Self {
        Self::spawn_with_error_hook(db_path, |_| {})
    }

    /// Like [`WriterActor::spawn`], but every skipped operation is
    /// reported through `on_error` (the daemon pipes this into its
    /// tracing log; tests record it). A failed op never kills the
    /// single writer thread.
    pub fn spawn_with_error_hook(
        db_path: &Path,
        on_error: impl Fn(String) + Send + Sync + 'static,
    ) -> Self {
        let conn = open_writer(db_path);
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || writer_loop(conn, rx, on_error));
        Self {
            tx: Some(tx),
            handle: Some(handle),
        }
    }

    /// Queues one operation; returns once the actor has buffered it
    /// (delivery, not durability, at this point).
    pub fn enqueue(&self, op: WriteOp) -> Result<(), WriteError> {
        let tx = self.tx.as_ref().ok_or(WriteError::Closed)?;
        tx.send(op).map_err(|_| WriteError::Closed)
    }

    /// Blocks until every operation enqueued before this call has
    /// been committed (FIFO channel order guarantees the ack lands
    /// behind the ops it covers).
    pub fn flush(&self) -> Result<(), WriteError> {
        let sender = self.tx.as_ref().ok_or(WriteError::Closed)?;
        let (ack, rx) = mpsc::channel();
        sender
            .send(WriteOp::Flush(ack))
            .map_err(|_| WriteError::Closed)?;
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(detail)) => Err(WriteError::Failed(detail)),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(WriteError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(WriteError::Closed),
        }
    }
}

fn open_writer(db_path: &Path) -> Connection {
    let mut conn = db::open_rw(db_path).expect("writer db open");
    migrations::run(&mut conn).expect("writer db migrations");
    conn
}

/// Batching loop for the single writer thread.
fn writer_loop(mut conn: Connection, rx: mpsc::Receiver<WriteOp>, on_error: impl Fn(String)) {
    // Batched transactions: up to BATCH_OPS ops or BATCH_WINDOW,
    // whichever comes first, then one COMMIT. WAL readers see each
    // batch atomically.
    let mut batch: Vec<WriteOp> = Vec::with_capacity(BATCH_OPS);
    loop {
        match rx.recv_timeout(BATCH_WINDOW) {
            Ok(op) => batch.push(op),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                flush_batch(&mut conn, &batch, &on_error);
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
                    flush_batch(&mut conn, &batch, &on_error);
                    return;
                }
            }
        }
        flush_batch(&mut conn, &batch, &on_error);
        batch.clear();
    }
}

/// Applies one batch inside a single transaction per segment: a
/// `Flush` marker commits everything before it, acks, and opens a
/// fresh transaction for the rest of the batch — no operation is
/// ever dropped for arriving after a marker, and every marker gets
/// its own ack. Failed operations are reported through `on_error`
/// and skipped; the writer keeps serving (one bad op must never
/// kill the only SQLite writer).
fn flush_batch(conn: &mut Connection, batch: &[WriteOp], on_error: &dyn Fn(String)) {
    if batch.is_empty() {
        return;
    }
    let mut tx = match conn.transaction() {
        Ok(tx) => tx,
        Err(err) => {
            // The connection is unusable for this batch: report once
            // and fail every marker so no caller waits out the ack
            // deadline. The writer thread keeps running.
            let detail = format!("writer transaction failed: {err}");
            on_error(detail.clone());
            fail_pending_flushes(batch, &detail);
            return;
        }
    };
    for (index, op) in batch.iter().enumerate() {
        match op {
            WriteOp::MetaPut { key, value } => {
                if let Err(err) = tx.execute(
                    "INSERT INTO meta(key, value) VALUES(?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [key, value],
                ) {
                    on_error(format!("meta upsert failed: {err}"));
                }
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
                if let Err(err) = tx.execute(
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
                ) {
                    on_error(format!("account insert failed: {err}"));
                }
            }
            WriteOp::AccountRemove { id } => {
                // Children cascade via FK (mailboxes, messages,
                // locations, bodies, attachments, tags, outbox).
                if let Err(err) = tx.execute("DELETE FROM accounts WHERE id = ?1", [id]) {
                    on_error(format!("account delete failed: {err}"));
                }
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
                if let Err(err) = tx.execute(
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
                ) {
                    on_error(format!("account update failed: {err}"));
                }
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
                if let Err(err) = tx.execute(
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
                ) {
                    on_error(format!("mailbox upsert failed: {err}"));
                }
            }
            WriteOp::MessageUpsert {
                account_id,
                dedupe_key,
                message_id_hdr,
                subject,
                from_json,
                to_json,
                cc_json,
                reply_to_json,
                date_utc,
                size_bytes,
                snippet,
                gmail_thread_id,
                root_message_id,
                subject_norm,
                location,
            } => {
                // The whole upsert is one soft-failed unit: a bad op
                // is reported and skipped, never allowed to kill the
                // only writer thread.
                let attempt = (|| -> Result<(), rusqlite::Error> {
                    // body_state is deliberately absent from the UPDATE:
                    // a header re-sync must never downgrade a fetched body.
                    // thread_id is resolved below, never taken from the wire.
                    tx.execute(
                        "INSERT INTO messages(
                        account_id, dedupe_key, message_id_hdr,
                        subject, from_json, to_json, cc_json, reply_to_json,
                        date_utc, size_bytes, snippet
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                     ON CONFLICT(account_id, dedupe_key) DO UPDATE SET
                        message_id_hdr = excluded.message_id_hdr,
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
                            subject,
                            from_json,
                            to_json,
                            cc_json,
                            reply_to_json,
                            date_utc,
                            size_bytes,
                            snippet
                        ],
                    )?;

                    // Resolve (or create) the thread row and link the
                    // message: a Gmail thrid wins, else the generic
                    // (root_message_id OR subject_norm) cluster matches,
                    // else a fresh thread is inserted.
                    let thread_id: Option<i64> = if let Some(thrid) = gmail_thread_id.as_deref() {
                        let existing: Option<i64> = tx
                            .query_row(
                                "SELECT id FROM threads
                             WHERE account_id = ?1 AND gmail_thread_id = ?2",
                                rusqlite::params![account_id, thrid],
                                |row| row.get(0),
                            )
                            .optional()?;
                        Some(match existing {
                            Some(id) => id,
                            None => {
                                tx.execute(
                                    "INSERT INTO threads(
                                    account_id, gmail_thread_id, root_message_id,
                                    subject_norm, last_date
                                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                                    rusqlite::params![
                                        account_id,
                                        thrid,
                                        root_message_id,
                                        subject_norm,
                                        date_utc.unwrap_or(0)
                                    ],
                                )?;
                                tx.last_insert_rowid()
                            }
                        })
                    } else if root_message_id.is_some() || subject_norm.is_some() {
                        let existing: Option<i64> = tx
                            .query_row(
                                "SELECT id FROM threads
                             WHERE account_id = ?1
                               AND ((?2 IS NOT NULL AND root_message_id = ?2)
                                 OR (?3 IS NOT NULL AND subject_norm = ?3))
                             ORDER BY id
                             LIMIT 1",
                                rusqlite::params![account_id, root_message_id, subject_norm],
                                |row| row.get(0),
                            )
                            .optional()?;
                        Some(match existing {
                            Some(id) => id,
                            None => {
                                tx.execute(
                                    "INSERT INTO threads(
                                    account_id, gmail_thread_id, root_message_id,
                                    subject_norm, last_date
                                 ) VALUES (?1, NULL, ?2, ?3, ?4)",
                                    rusqlite::params![
                                        account_id,
                                        root_message_id,
                                        subject_norm,
                                        date_utc.unwrap_or(0)
                                    ],
                                )?;
                                tx.last_insert_rowid()
                            }
                        })
                    } else {
                        // No threading signals: the message stays
                        // unthreaded until a sibling upsert clusters it.
                        None
                    };
                    if let Some(thread_id) = thread_id {
                        tx.execute(
                            "UPDATE messages SET thread_id = ?1
                         WHERE account_id = ?2 AND dedupe_key = ?3",
                            rusqlite::params![thread_id, account_id, dedupe_key],
                        )?;
                    }

                    // Inline location: resolve both ids inside the same
                    // transaction (mailbox by name, message just upserted).
                    if let Some(loc) = location {
                        let mailbox_id: i64 = tx.query_row(
                            "SELECT id FROM mailboxes
                             WHERE account_id = ?1 AND name = ?2",
                            rusqlite::params![account_id, loc.mailbox_name],
                            |row| row.get(0),
                        )?;
                        let message_id: i64 = tx.query_row(
                            "SELECT id FROM messages
                             WHERE account_id = ?1 AND dedupe_key = ?2",
                            rusqlite::params![account_id, dedupe_key],
                            |row| row.get(0),
                        )?;
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
                                loc.uid as i64,
                                loc.seen,
                                loc.flagged,
                                loc.answered,
                                loc.draft,
                                loc.deleted,
                                loc.keywords_json,
                                loc.modseq.map(|v| v as i64)
                            ],
                        )?;
                    }
                    Ok(())
                })();
                if let Err(err) = attempt {
                    on_error(format!("message upsert failed: {err}"));
                }
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
                if let Err(err) = tx.execute(
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
                ) {
                    on_error(format!("location upsert failed: {err}"))
                }
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
                        if let Err(err) = tx.execute(
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
                        ) { on_error(format!("gmail thread upsert failed: {err}")) }
                    }
                    // No thrid: plain insert — clustering across
                    // references is decided by the sync engine.
                    None => {
                        if let Err(err) = tx.execute(
                            "INSERT INTO threads(
                                account_id, root_message_id, subject_norm, last_date
                             ) VALUES (?1, ?2, ?3, ?4)",
                            rusqlite::params![account_id, root_message_id, subject_norm, last_date],
                        ) { on_error(format!("generic thread insert failed: {err}")) }
                    }
                }
            }
            WriteOp::ThreadCountersRecompute { account_id } => {
                if let Err(err) = tx.execute(
                    "UPDATE threads SET
                        message_count = (SELECT COUNT(*) FROM messages
                                         WHERE thread_id = threads.id),
                        unread_count = (SELECT COUNT(*) FROM messages m
                                        JOIN message_locations l ON l.message_id = m.id
                                        WHERE m.thread_id = threads.id AND l.seen = 0)
                     WHERE account_id = ?1",
                    rusqlite::params![account_id],
                ) {
                    on_error(format!("thread counters recompute failed: {err}"))
                }
            }
            WriteOp::Flush(ack) => {
                // Commit what precedes the marker, then ack: the ack
                // sender only observes the batch boundary it asked
                // for. The rest of the batch continues in a fresh
                // transaction so nothing queued behind the marker is
                // lost. A failed commit is reported and acked as a
                // failure; it never panics the only writer.
                match tx.commit() {
                    Ok(()) => {
                        let _ = ack.send(Ok(()));
                    }
                    Err(err) => {
                        let detail = format!("writer commit failed: {err}");
                        on_error(detail.clone());
                        let _ = ack.send(Err(detail));
                    }
                }
                match conn.transaction() {
                    Ok(fresh) => tx = fresh,
                    Err(err) => {
                        let detail = format!("writer transaction failed: {err}");
                        on_error(detail.clone());
                        fail_pending_flushes(&batch[index + 1..], &detail);
                        return;
                    }
                }
            }
        }
    }
    if let Err(err) = tx.commit() {
        on_error(format!("writer commit failed: {err}"));
    }
}

/// Fails every flush marker in `batch` with `detail`; used when the
/// writer cannot even open the transaction that would commit them, so
/// callers learn immediately instead of waiting out the ack deadline.
fn fail_pending_flushes(batch: &[WriteOp], detail: &str) {
    for op in batch {
        if let WriteOp::Flush(ack) = op {
            let _ = ack.send(Err(detail.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {

    /// Enqueues an operation, failing the test when the writer is gone.
    fn enqueue(writer: &WriterActor, op: WriteOp) {
        writer.enqueue(op).expect("enqueue");
    }
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
            enqueue(
                &writer,
                WriteOp::MetaPut {
                    key: format!("key-{i}"),
                    value: format!("value-{i}"),
                },
            );
        }
        writer.flush().expect("flush");

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
        enqueue(
            &writer,
            WriteOp::MailboxUpsert {
                account_id: 1,
                name: "INBOX".into(),
                kind: "inbox".into(),
                special_use: None,
                uidvalidity: Some(42),
                uidnext: Some(10),
                highest_modseq: Some(7),
            },
        );
        writer.flush().expect("flush");
        enqueue(
            &writer,
            WriteOp::MailboxUpsert {
                account_id: 1,
                name: "INBOX".into(),
                kind: "inbox".into(),
                special_use: Some("\\Sent".into()),
                uidvalidity: Some(42),
                uidnext: Some(11),
                highest_modseq: Some(9),
            },
        );
        writer.flush().expect("flush");

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
        enqueue(
            &writer,
            WriteOp::MailboxUpsert {
                account_id: 1,
                name: "INBOX".into(),
                kind: "inbox".into(),
                special_use: None,
                uidvalidity: None,
                uidnext: None,
                highest_modseq: None,
            },
        );
        enqueue(
            &writer,
            WriteOp::ThreadUpsert {
                account_id: 1,
                gmail_thread_id: Some("gm-1".into()),
                root_message_id: None,
                subject_norm: Some("hello".into()),
                last_date: 100,
            },
        );
        writer.flush().expect("flush");

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
            subject: Some(subject.into()),
            from_json: "[]".into(),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            reply_to_json: "[]".into(),
            date_utc: Some(date_utc),
            size_bytes: Some(1024),
            snippet: Some("body text".into()),
            gmail_thread_id: Some("gm-1".into()),
            root_message_id: None,
            subject_norm: None,
            location: None,
        };
        enqueue(&writer, message("Hello", 500));
        writer.flush().expect("flush");
        enqueue(&writer, message("Hello v2", 600));
        writer.flush().expect("flush");

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
        enqueue(
            &writer,
            WriteOp::MailboxUpsert {
                account_id: 1,
                name: "INBOX".into(),
                kind: "inbox".into(),
                special_use: None,
                uidvalidity: None,
                uidnext: None,
                highest_modseq: None,
            },
        );
        enqueue(
            &writer,
            WriteOp::MessageUpsert {
                account_id: 1,
                dedupe_key: "mid:loc@example.com".into(),
                message_id_hdr: Some("loc@example.com".into()),
                subject: None,
                from_json: "[]".into(),
                to_json: "[]".into(),
                cc_json: "[]".into(),
                reply_to_json: "[]".into(),
                date_utc: None,
                size_bytes: None,
                snippet: None,
                gmail_thread_id: None,
                root_message_id: None,
                subject_norm: None,
                location: None,
            },
        );
        writer.flush().expect("flush");

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
        enqueue(&writer, location(true, false, 10));
        writer.flush().expect("flush");
        enqueue(&writer, location(false, true, 11));
        writer.flush().expect("flush");

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
        enqueue(
            &writer,
            WriteOp::ThreadUpsert {
                account_id: 1,
                gmail_thread_id: Some("gm-9".into()),
                root_message_id: Some("<r1>".into()),
                subject_norm: Some("s".into()),
                last_date: 100,
            },
        );
        writer.flush().expect("flush");
        enqueue(
            &writer,
            WriteOp::ThreadUpsert {
                account_id: 1,
                gmail_thread_id: Some("gm-9".into()),
                root_message_id: None,
                subject_norm: None,
                last_date: 200,
            },
        );
        writer.flush().expect("flush");
        enqueue(
            &writer,
            WriteOp::ThreadUpsert {
                account_id: 1,
                gmail_thread_id: None,
                root_message_id: Some("<r2>".into()),
                subject_norm: Some("t".into()),
                last_date: 50,
            },
        );
        writer.flush().expect("flush");

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

    #[test]
    fn message_upsert_resolves_threads_and_locations_inline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(super::super::DB_FILE_NAME);
        seed_account(&path, 1);

        let writer = WriterActor::spawn(&path);
        enqueue(
            &writer,
            WriteOp::MailboxUpsert {
                account_id: 1,
                name: "INBOX".into(),
                kind: "inbox".into(),
                special_use: None,
                uidvalidity: Some(11),
                uidnext: Some(6),
                highest_modseq: Some(4),
            },
        );
        let message = |dedupe_key: &str, root: &str, seen: bool, uid: u32| WriteOp::MessageUpsert {
            account_id: 1,
            dedupe_key: dedupe_key.into(),
            message_id_hdr: Some(dedupe_key.trim_start_matches("mid:").into()),
            subject: Some("Re: Hello".into()),
            from_json: "[]".into(),
            to_json: "[]".into(),
            cc_json: "[]".into(),
            reply_to_json: "[]".into(),
            date_utc: Some(500),
            size_bytes: Some(128),
            snippet: Some("body".into()),
            gmail_thread_id: None,
            root_message_id: Some(root.into()),
            subject_norm: Some("hello".into()),
            location: Some(LocationInline {
                mailbox_name: "INBOX".into(),
                uid,
                seen,
                flagged: false,
                answered: false,
                draft: false,
                deleted: false,
                keywords_json: "[]".into(),
                modseq: Some(4),
            }),
        };
        enqueue(&writer, message("mid:1@example.com", "<r1>", true, 5));
        enqueue(&writer, message("mid:2@example.com", "<r2>", false, 6));
        enqueue(&writer, WriteOp::ThreadCountersRecompute { account_id: 1 });
        writer.flush().expect("flush");

        let conn = crate::infra::db::open_readonly(&path).expect("re-open");
        let (thread_count, message_count, unread_count): (i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), MAX(message_count), MAX(unread_count) FROM threads
                 WHERE account_id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("thread row");
        assert_eq!(thread_count, 1, "same subject_norm must cluster");
        assert_eq!(message_count, 2);
        assert_eq!(unread_count, 1);

        let thread_id: i64 = conn
            .query_row("SELECT id FROM threads WHERE account_id = 1", [], |row| {
                row.get(0)
            })
            .expect("thread row");
        let assigned: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE account_id = 1 AND thread_id = ?1",
                [thread_id],
                |row| row.get(0),
            )
            .expect("thread assignment");
        assert_eq!(assigned, 2, "both messages join the cluster");

        let (locations, seen_sum): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(seen) FROM message_locations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("locations row");
        assert_eq!(locations, 2);
        assert_eq!(seen_sum, 1);
    }

    /// A migrated scratch connection for white-box `flush_batch`
    /// calls.
    fn scratch_conn(dir: &tempfile::TempDir) -> Connection {
        let path = dir.path().join(super::super::DB_FILE_NAME);
        let mut conn = crate::infra::db::open_rw(&path).expect("open scratch");
        migrations::run(&mut conn).expect("migrate scratch");
        conn
    }

    fn meta_count(conn: &Connection, key: &str) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .expect("meta count")
    }

    /// Operations queued behind a Flush marker inside one drained
    /// batch must still be applied (the ack must not cost the tail).
    #[test]
    fn flush_batch_applies_operations_after_a_flush_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = scratch_conn(&dir);
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        let batch = vec![
            WriteOp::MetaPut {
                key: "before".into(),
                value: "1".into(),
            },
            WriteOp::Flush(ack_tx),
            WriteOp::MetaPut {
                key: "after".into(),
                value: "2".into(),
            },
        ];
        flush_batch(&mut conn, &batch, &|_| {});
        ack_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("ack")
            .expect("commit ok");
        assert_eq!(meta_count(&conn, "before"), 1);
        assert_eq!(
            meta_count(&conn, "after"),
            1,
            "ops queued after a flush marker must still land"
        );
    }

    /// Every Flush marker in one batch gets its own commit and ack.
    #[test]
    fn flush_batch_acks_every_flush_marker_in_one_batch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = scratch_conn(&dir);
        let (first_tx, first_rx) = std::sync::mpsc::channel();
        let (second_tx, second_rx) = std::sync::mpsc::channel();
        let batch = vec![
            WriteOp::MetaPut {
                key: "one".into(),
                value: "1".into(),
            },
            WriteOp::Flush(first_tx),
            WriteOp::MetaPut {
                key: "two".into(),
                value: "2".into(),
            },
            WriteOp::Flush(second_tx),
            WriteOp::MetaPut {
                key: "three".into(),
                value: "3".into(),
            },
        ];
        flush_batch(&mut conn, &batch, &|_| {});
        first_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("first flush acked")
            .expect("commit ok");
        second_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("second flush acked")
            .expect("commit ok");
        assert_eq!(meta_count(&conn, "one"), 1);
        assert_eq!(meta_count(&conn, "two"), 1);
        assert_eq!(meta_count(&conn, "three"), 1);
    }

    /// A failing operation is reported through the error hook and
    /// skipped; the writer keeps serving later operations.
    #[test]
    fn flush_batch_skips_failing_operations_and_keeps_going() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = scratch_conn(&dir);
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        let reported: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        let on_error = |msg: String| {
            reported.lock().expect("report lock").push(msg);
        };
        let batch = vec![
            WriteOp::MailboxUpsert {
                account_id: 999,
                name: "INBOX".into(),
                kind: "mail".into(),
                special_use: None,
                uidvalidity: None,
                uidnext: None,
                highest_modseq: None,
            },
            WriteOp::MetaPut {
                key: "survivor".into(),
                value: "yes".into(),
            },
            WriteOp::Flush(ack_tx),
        ];
        flush_batch(&mut conn, &batch, &on_error);
        ack_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("ack")
            .expect("commit ok");
        assert_eq!(
            meta_count(&conn, "survivor"),
            1,
            "the writer must keep serving after a failed op"
        );
        assert!(
            !reported.lock().expect("report lock").is_empty(),
            "the failed op must be reported through the error hook"
        );
    }

    /// When the writer cannot even open a transaction, every flush
    /// marker in the batch learns about it immediately instead of
    /// timing out, and the failure reaches the error hook.
    #[test]
    fn flush_batch_fails_markers_when_a_transaction_cannot_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = scratch_conn(&dir);
        // Holding an outer transaction makes `conn.transaction()`
        // fail exactly like a wedged connection would.
        conn.execute_batch("BEGIN").expect("outer begin");
        let (ack_tx, ack_rx) = std::sync::mpsc::channel();
        let reported: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        let on_error = |msg: String| {
            reported.lock().expect("report lock").push(msg);
        };
        let batch = vec![
            WriteOp::MetaPut {
                key: "never".into(),
                value: "1".into(),
            },
            WriteOp::Flush(ack_tx),
        ];

        flush_batch(&mut conn, &batch, &on_error);

        let outcome = ack_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("marker must be failed, not ignored");
        assert!(outcome.is_err(), "the marker must report the failure");
        assert!(
            !reported.lock().expect("report lock").is_empty(),
            "the failure must reach the error hook"
        );
        conn.execute_batch("ROLLBACK").expect("outer rollback");
    }

    /// The public actor surface reports failed ops and keeps the
    /// thread alive for subsequent flushes.
    #[test]
    fn writer_actor_survives_a_failing_operation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(super::super::DB_FILE_NAME);
        let mut seed = crate::infra::db::open_rw(&path).expect("open seed");
        migrations::run(&mut seed).expect("migrate seed");
        drop(seed);

        let reported: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = std::sync::Arc::clone(&reported);
        let writer = WriterActor::spawn_with_error_hook(&path, move |msg| {
            recorder.lock().expect("report lock").push(msg);
        });
        enqueue(
            &writer,
            WriteOp::MailboxUpsert {
                account_id: 999,
                name: "INBOX".into(),
                kind: "mail".into(),
                special_use: None,
                uidvalidity: None,
                uidnext: None,
                highest_modseq: None,
            },
        );
        enqueue(
            &writer,
            WriteOp::MetaPut {
                key: "after-failure".into(),
                value: "ok".into(),
            },
        );
        writer.flush().expect("flush");

        assert_eq!(
            meta_count(&conn_of(&path), "after-failure"),
            1,
            "ops after a failed op must still land"
        );
        assert!(
            !reported.lock().expect("report lock").is_empty(),
            "failure must reach the error hook"
        );
    }

    fn conn_of(path: &std::path::Path) -> Connection {
        crate::infra::db::open_readonly(path).expect("re-open")
    }
}
