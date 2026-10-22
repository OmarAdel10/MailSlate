//! Read-side connection pool (ARCHITECTURE.md §7.1): bounded read-only
//! connections for bridge reads; never touches the write path.
//!
//! rusqlite connections are sync; callers across `.await` hand the guard
//! off via `spawn_blocking` instead of holding it.

use std::ops::Deref;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::Connection;

/// Number of read connections in the pool (ARCHITECTURE §7.1).
pub const READ_POOL_SIZE: usize = 4;

/// Checkout failure reasons.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    /// All `READ_POOL_SIZE` connections are checked out.
    #[error("read pool exhausted ({READ_POOL_SIZE} connections)")]
    Exhausted,
    /// The bounded wait expired before a connection was returned.
    #[error("timed out waiting for a pooled read connection")]
    Timeout,
}

/// A bounded pool of read-only connections; checkouts return via RAII.
#[derive(Debug)]
pub struct ReadPool {
    idle: Mutex<Vec<Connection>>,
}

impl ReadPool {
    /// Opens `READ_POOL_SIZE` read-only connections over `path`.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let mut idle = Vec::with_capacity(READ_POOL_SIZE);
        for _ in 0..READ_POOL_SIZE {
            idle.push(crate::infra::db::open_readonly(path)?);
        }
        Ok(Self {
            idle: Mutex::new(idle),
        })
    }

    /// Acquires a connection without waiting; use [`Self::acquire_async`]
    /// when the pool may be exhausted.
    pub fn acquire(&self) -> Result<PooledRead<'_>, PoolError> {
        let mut idle = self.idle.lock().expect("pool lock");
        let conn = idle.pop().ok_or(PoolError::Exhausted)?;
        Ok(PooledRead {
            pool: self,
            conn: Some(conn),
        })
    }

    /// Acquires a connection, polling until a checkout is returned.
    pub async fn acquire_async(&self, timeout: Duration) -> Result<PooledRead<'_>, PoolError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.acquire() {
                Ok(conn) => return Ok(conn),
                Err(PoolError::Exhausted) => {
                    if Instant::now() >= deadline {
                        return Err(PoolError::Timeout);
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(other) => return Err(other),
            }
        }
    }
}

/// A checked-out read connection; dropping returns it to the pool.
pub struct PooledRead<'a> {
    pool: &'a ReadPool,
    conn: Option<Connection>,
}

impl Deref for PooledRead<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("connection is checked out")
    }
}

impl Drop for PooledRead<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            self.pool.idle.lock().expect("pool lock").push(conn);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn seeded_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(super::super::DB_FILE_NAME);
        let mut conn = crate::infra::db::open_rw(&path).expect("open");
        crate::infra::db::migrations::run(&mut conn).expect("migrate");
        (dir, path)
    }

    /// A fully checked-out pool reports Exhausted, not lock contention.
    #[test]
    fn pool_limits_concurrent_readers_to_four() {
        let (_dir, path) = seeded_db();
        let pool = ReadPool::open(&path).expect("open pool");

        let one = pool.acquire().expect("checkout 1");
        let two = pool.acquire().expect("checkout 2");
        let three = pool.acquire().expect("checkout 3");
        let four = pool.acquire().expect("checkout 4");
        let fifth = match pool.acquire() {
            Err(PoolError::Exhausted) => PoolError::Exhausted,
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("a fifth checkout must reject Exhausted"),
        };
        assert!(matches!(fifth, PoolError::Exhausted));
        drop((one, two, three, four));

        // After releasing all, a fresh checkout succeeds immediately.
        let five = pool.acquire().expect("checkout after release");
        drop(five);
    }

    /// The bounded wait lands on Timeout instead of hanging forever.
    #[tokio::test]
    async fn acquire_async_times_out_when_pool_is_drained() {
        let (_dir, path) = seeded_db();
        let pool = ReadPool::open(&path).expect("open pool");

        let holder_1 = pool.acquire().expect("checkout holder 1");
        let holder_2 = pool.acquire().expect("checkout holder 2");
        let holder_3 = pool.acquire().expect("checkout holder 3");
        let holder_4 = pool.acquire().expect("checkout holder 4");
        let started = Instant::now();
        let error = match pool.acquire_async(Duration::from_millis(50)).await {
            Err(error) => error,
            Ok(_) => panic!("drained pool must time out"),
        };
        drop((holder_1, holder_2, holder_3, holder_4));
        assert!(matches!(error, PoolError::Timeout));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "timeout must be bounded, took {:?}",
            started.elapsed()
        );
    }

    /// Every pooled connection is read-only: writes are rejected.
    #[test]
    fn pooled_connections_reject_writes() {
        let (_dir, path) = seeded_db();
        let pool = ReadPool::open(&path).expect("open pool");

        {
            let conn = pool.acquire().expect("checkout");
            let result = conn.execute_batch("CREATE TABLE probe_w (id INTEGER PRIMARY KEY);");
            assert!(result.is_err(), "pooled connection must be read-only");
        }
    }
}
