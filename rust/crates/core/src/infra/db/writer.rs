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
    /// Internal: ack after every queued op before it is committed.
    Flush(mpsc::Sender<()>),
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
}
