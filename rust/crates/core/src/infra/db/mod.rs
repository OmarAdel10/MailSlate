//! SQLite storage layer: connection profile, migrations, FTS5 probe
//! (ARCHITECTURE.md §7).
//!
//! The daemon is the only writer (single-writer actor, §7.1); the GUI and the
//! bridge open read-only snapshots.

pub mod migrations;

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

/// File name of the MailSlate database inside the data directory.
pub const DB_FILE_NAME: &str = "mailslate.db";

/// Opens the database read-write with the standard connection profile.
pub fn open_rw(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    Ok(conn)
}

/// Opens the database read-only (bridge-side reads, diagnostics).
pub fn open_readonly(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(Duration::from_millis(5_000))?;
    Ok(conn)
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    // journal_mode returns the resulting mode instead of an empty result set.
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    debug_assert_eq!(mode.to_ascii_lowercase(), "wal");
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(Duration::from_millis(5_000))?;
    Ok(())
}

/// Probes the linked SQLite build for FTS5 support on a scratch database.
///
/// The daemon calls this at boot and refuses to start when it fails
/// (ARCHITECTURE §7.3).
pub fn assert_fts5() -> rusqlite::Result<()> {
    let scratch = Connection::open_in_memory()?;
    scratch.execute_batch("CREATE VIRTUAL TABLE fts5_probe USING fts5(content);")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DB_FILE_NAME);
        (dir, path)
    }

    #[test]
    fn open_rw_applies_the_connection_profile() {
        let (_dir, path) = temp_db();
        let conn = open_rw(&path).expect("open");

        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal_mode");
        assert_eq!(journal.to_ascii_lowercase(), "wal");

        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("synchronous");
        assert_eq!(synchronous, 1, "NORMAL is 1");

        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("foreign_keys");
        assert_eq!(foreign_keys, 1);

        let busy_timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("busy_timeout");
        assert_eq!(busy_timeout, 5_000);
    }

    #[test]
    fn open_readonly_reads_an_existing_database() {
        let (_dir, path) = temp_db();
        {
            let conn = open_rw(&path).expect("open rw");
            conn.execute_batch("CREATE TABLE probe (id INTEGER PRIMARY KEY);")
                .expect("create");
        }

        let conn = open_readonly(&path).expect("open ro");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM probe", [], |row| row.get(0))
            .expect("count");
        assert_eq!(count, 0);

        let write = conn.execute_batch("INSERT INTO probe DEFAULT VALUES;");
        assert!(write.is_err(), "read-only connection must reject writes");
    }

    #[test]
    fn fts5_is_available() {
        assert_fts5().expect("bundled SQLite must ship FTS5");
    }
}
