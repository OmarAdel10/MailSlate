//! Forward-only migration runner (ARCHITECTURE.md §7.4).
//!
//! Migration SQL is embedded at compile time and tracked by `PRAGMA
//! user_version`. Each migration runs in one transaction; the daemon refuses
//! to start when one fails. The GUI never migrates — it version-gates through
//! the bridge instead.

use rusqlite::Connection;

/// A single forward migration: the schema version it upgrades to.
struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "init",
    sql: include_str!("migrations/0001_init.sql"),
}];

/// Errors while running migrations.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// Reading the current schema version failed.
    #[error("failed to read schema version")]
    Version(#[source] rusqlite::Error),
    /// Applying one migration failed; the transaction rolled back.
    #[error("migration {version:04} ({name}) failed")]
    Apply {
        /// Schema version the migration upgrades to.
        version: u32,
        /// Migration name for diagnostics.
        name: &'static str,
        /// Underlying SQLite error.
        #[source]
        source: rusqlite::Error,
    },
    /// The database was migrated by a newer MailSlate build.
    #[error(
        "database schema v{found} is newer than this build supports (v{supported}); update MailSlate"
    )]
    TooNew {
        /// Schema version found in the database.
        found: u32,
        /// Highest schema version this build knows.
        supported: u32,
    },
}

/// Applies every pending migration; returns the resulting schema version.
pub fn run(conn: &mut Connection) -> Result<u32, MigrationError> {
    let current = schema_version(conn).map_err(MigrationError::Version)?;
    let latest = MIGRATIONS.iter().map(|m| m.version).max().unwrap_or(0);
    if current > latest {
        return Err(MigrationError::TooNew {
            found: current,
            supported: latest,
        });
    }
    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        apply(conn, migration).map_err(|source| MigrationError::Apply {
            version: migration.version,
            name: migration.name,
            source,
        })?;
    }
    Ok(latest)
}

/// Current schema version (`PRAGMA user_version`).
pub fn schema_version(conn: &Connection) -> rusqlite::Result<u32> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn apply(conn: &mut Connection, migration: &Migration) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(migration.sql)?;
    tx.pragma_update(None, "user_version", migration.version)?;
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::{DB_FILE_NAME, open_rw};

    const EXPECTED_TABLES: &[&str] = &[
        "accounts",
        "mailboxes",
        "threads",
        "messages",
        "message_locations",
        "message_bodies",
        "attachments",
        "tags",
        "message_tags",
        "outbox",
        "meta",
        "messages_fts_content",
        "messages_fts",
    ];

    fn migrated() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = open_rw(&dir.path().join(DB_FILE_NAME)).expect("open");
        let version = run(&mut conn).expect("migrate");
        assert_eq!(version, 1);
        (dir, conn)
    }

    #[test]
    fn creates_schema_v1_and_is_idempotent() {
        let (_dir, mut conn) = migrated();
        assert_eq!(schema_version(&conn).unwrap(), 1);

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("prepare")
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("collect");
        for table in EXPECTED_TABLES {
            assert!(tables.iter().any(|t| t == table), "missing table {table}");
        }

        // Re-running is a no-op.
        assert_eq!(run(&mut conn).expect("re-run"), 1);
    }

    #[test]
    fn fts5_index_round_trips() {
        let (_dir, conn) = migrated();
        conn.execute(
            "INSERT INTO messages_fts_content \
             (message_id, subject, from_text, to_text, body) \
             VALUES (1, 'Quarterly report', 'ana', 'bo', 'the budgets look fine')",
            [],
        )
        .expect("content row");
        conn.execute(
            "INSERT INTO messages_fts (rowid, subject, from_text, to_text, body) \
             VALUES (1, 'Quarterly report', 'ana', 'bo', 'the budgets look fine')",
            [],
        )
        .expect("index row");

        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'budgets'",
                [],
                |row| row.get(0),
            )
            .expect("match");
        assert_eq!(hits, 1);
    }

    #[test]
    fn refuses_databases_from_newer_builds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = open_rw(&dir.path().join(DB_FILE_NAME)).expect("open");
        conn.pragma_update(None, "user_version", 999u32)
            .expect("set version");
        let err = run(&mut conn).expect_err("must refuse");
        assert!(matches!(
            err,
            MigrationError::TooNew {
                found: 999,
                supported: 1
            }
        ));
    }

    #[test]
    fn failure_rolls_back_the_entire_migration() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut conn = open_rw(&dir.path().join(DB_FILE_NAME)).expect("open");
        // Poison the schema: the first CREATE TABLE of 0001 collides.
        conn.execute_batch("CREATE TABLE accounts (bogus TEXT);")
            .expect("poison");

        let err = run(&mut conn).expect_err("must fail");
        assert!(matches!(err, MigrationError::Apply { version: 1, .. }));
        assert_eq!(schema_version(&conn).unwrap(), 0, "version untouched");

        let created: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'mailboxes'",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(created, 0, "no partial schema left behind");
    }
}
