//! IPC server lifecycle: single-instance guard, socket, accept loop.
//!
//! The wire protocol lives in `mailslate-protocol`; this module owns the
//! Unix domain socket (ARCHITECTURE.md §5). Windows named-pipe support lands
//! with the Windows build.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mailslate_protocol::FrameError;
use mailslate_protocol::async_io::{read_json_frame_async, write_json_frame_async};
use mailslate_protocol::message::{Request, Response};

use crate::dispatch::{self, Dispatched};
use crate::logging::LoggingError;

/// Pause after a failed `accept()` before retrying.
///
/// Accept errors on a bound listener are transient (e.g. file-descriptor
/// exhaustion); the server logs them and keeps serving.
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(50);

/// Maximum simultaneously served connections.
///
/// Beyond this the daemon assumes a misbehaving (same-uid) peer is holding
/// slots and drops new connections at accept time. One GUI client plus
/// short-lived probes never approach the cap.
const MAX_CONNECTIONS: usize = 8;

/// Per-connection idle budget.
///
/// A connection that sends nothing within this window is dropped: an
/// idle (or hostile) same-uid peer cannot hold a task and a frame
/// allocation forever. The GUI opens a fresh connection per call, so
/// legitimate traffic never approaches the window.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Fatal daemon errors: startup failures and server lifecycle problems.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// Filesystem or socket I/O failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// SQLite failed to open or configure the database.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The schema migration failed.
    #[error("database migration failed: {0}")]
    Migration(#[from] mailslate_core::infra::db::migrations::MigrationError),
    /// Logging could not be initialized.
    #[error(transparent)]
    Logging(#[from] LoggingError),
    /// Another daemon instance already owns the socket.
    #[error("another daemon instance is already running")]
    AlreadyRunning,
    /// The IPC server is not implemented for this platform yet.
    #[cfg(not(unix))]
    #[error("this platform is not supported yet")]
    UnsupportedPlatform,
}

/// Errors scoped to one connection; never fatal to the server.
#[derive(Debug, thiserror::Error)]
enum ConnError {
    /// Socket I/O failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The frame codec rejected the peer's bytes.
    #[error("frame error: {0}")]
    Frame(#[from] FrameError),
    /// The peer runs under a different user id.
    #[error("peer uid {0} does not own this daemon")]
    PeerRejected(u32),
    /// The peer sent nothing within the connection's read budget.
    #[error("connection idle for {0:?}; dropped")]
    Idle(Duration),
}

/// Serves the IPC protocol on `socket_path` until shutdown.
///
/// Exits with [`ServerError::AlreadyRunning`] when a live daemon already
/// answers on the socket; a stale socket file is removed and rebound.
#[cfg(unix)]
pub async fn run(socket_path: PathBuf, pid_file: PathBuf) -> Result<(), ServerError> {
    // Register SIGTERM before the socket exists: an external observer treats
    // the socket as readiness, so a signal arriving right after startup must
    // already be handled (graceful exit + artifact cleanup) instead of
    // killing the process with the default disposition.
    let sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    prepare_socket(&socket_path)?;

    let listener = bind_with_race_check(&socket_path)?;
    set_socket_permissions(&socket_path)?;
    std::fs::write(&pid_file, std::process::id().to_string())?;

    tracing::info!(
        socket = %socket_path.display(),
        version = env!("CARGO_PKG_VERSION"),
        "mailslate-daemon listening"
    );

    let shutdown = Arc::new(tokio::sync::Notify::new());
    let result = accept_loop(listener, shutdown, sigterm).await;

    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&pid_file);
    result
}

/// Not implemented outside Unix yet (Windows named pipes land with the
/// Windows build).
#[cfg(not(unix))]
pub async fn run(_socket_path: PathBuf, _pid_file: PathBuf) -> Result<(), ServerError> {
    Err(ServerError::UnsupportedPlatform)
}

/// Ensures the socket path is ours to bind: a live daemon is reported, a
/// stale file is unlinked.
#[cfg(unix)]
fn prepare_socket(socket_path: &Path) -> Result<(), ServerError> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(_) => Err(ServerError::AlreadyRunning),
        Err(_) => {
            let _ = std::fs::remove_file(socket_path);
            Ok(())
        }
    }
}

/// Binds the listener, turning a lost bind race into a friendly error.
///
/// [`prepare_socket`]'s probe and the bind are two steps: a concurrent
/// daemon can claim the path in between. Re-probing on bind failure lets
/// the race loser report [`ServerError::AlreadyRunning`]; a dead path
/// keeps its original io error.
#[cfg(unix)]
fn bind_with_race_check(socket_path: &Path) -> Result<tokio::net::UnixListener, ServerError> {
    match tokio::net::UnixListener::bind(socket_path) {
        Ok(listener) => Ok(listener),
        Err(error) => match std::os::unix::net::UnixStream::connect(socket_path) {
            Ok(_) => Err(ServerError::AlreadyRunning),
            Err(_) => Err(ServerError::Io(error)),
        },
    }
}

#[cfg(unix)]
fn set_socket_permissions(socket_path: &Path) -> Result<(), ServerError> {
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
async fn accept_loop(
    listener: tokio::net::UnixListener,
    shutdown: Arc<tokio::sync::Notify>,
    mut sigterm: tokio::signal::unix::Signal,
) -> Result<(), ServerError> {
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    loop {
        tokio::select! {
            _ = shutdown.notified() => {
                tracing::info!("shutdown requested over IPC");
                break;
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("interrupted; shutting down");
                break;
            }
            _ = sigterm.recv() => {
                tracing::info!("terminated; shutting down");
                break;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        use std::sync::atomic::Ordering;
                        if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                            tracing::warn!("connection limit reached; rejecting peer");
                            continue; // dropping the stream closes the connection
                        }
                        active.fetch_add(1, Ordering::Relaxed);
                        let shutdown = Arc::clone(&shutdown);
                        let active = Arc::clone(&active);
                        tokio::spawn(async move {
                            if let Err(err) =
                                handle_connection(stream, shutdown, READ_TIMEOUT).await
                            {
                                tracing::debug!(error = %err, "connection closed with error");
                            }
                            active.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "accept failed; retrying");
                        tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn handle_connection(
    mut stream: tokio::net::UnixStream,
    shutdown: Arc<tokio::sync::Notify>,
    read_timeout: Duration,
) -> Result<(), ConnError> {
    check_peer_uid(&stream)?;
    loop {
        let request: Request =
            match tokio::time::timeout(read_timeout, read_json_frame_async(&mut stream)).await {
                Ok(Ok(request)) => request,
                Ok(Err(FrameError::Io(err))) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(()); // client disconnected
                }
                Ok(Err(err)) => return Err(ConnError::Frame(err)),
                Err(_) => return Err(ConnError::Idle(read_timeout)),
            };
        let (response, stop) = match dispatch::dispatch(&request.method) {
            Ok(Dispatched::Reply(value)) => (Response::ok(request.id, value), false),
            Ok(Dispatched::Stop(value)) => (Response::ok(request.id, value), true),
            Err(err) => (Response::err(request.id, err), false),
        };
        write_json_frame_async(&mut stream, &response).await?;
        if stop {
            shutdown.notify_one();
            return Ok(());
        }
    }
}

/// Rejects peers running under another uid (Linux; other platforms keep the
/// socket-permission defense until their credential APIs land).
#[cfg(all(unix, target_os = "linux"))]
fn check_peer_uid(stream: &tokio::net::UnixStream) -> Result<(), ConnError> {
    let cred = stream.peer_cred()?;
    let owner = unsafe { libc::geteuid() };
    if cred.uid() != owner {
        return Err(ConnError::PeerRejected(cred.uid()));
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn check_peer_uid(_stream: &tokio::net::UnixStream) -> Result<(), ConnError> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// An idle peer must be dropped within its read budget — without the
    /// timeout the server side would block on the first read forever.
    #[tokio::test]
    async fn idle_connection_is_dropped_within_read_budget() {
        let (mut client, server) = tokio::net::UnixStream::pair().expect("socket pair");
        let shutdown = Arc::new(tokio::sync::Notify::new());

        let handled = tokio::time::timeout(
            Duration::from_secs(5),
            handle_connection(server, shutdown, Duration::from_millis(100)),
        )
        .await
        .expect("idle connection must be dropped promptly, not hang");

        assert!(handled.is_err(), "idle connection must be dropped");

        // The client observes the drop as EOF.
        let mut buf = [0u8; 1];
        let read = tokio::time::timeout(Duration::from_secs(5), client.read(&mut buf))
            .await
            .expect("client read must not hang")
            .expect("client read");
        assert_eq!(read, 0, "client must see EOF after the drop");
    }

    /// A bind race lost to a concurrent daemon surfaces the friendly
    /// AlreadyRunning error instead of a raw io error.
    #[test]
    fn bind_race_reports_already_running_when_socket_is_live() {
        let dir = tempfile::tempdir().expect("temp dir");
        let socket_path = dir.path().join("race.sock");
        let winner = std::os::unix::net::UnixListener::bind(&socket_path).expect("winner binds");

        let error =
            bind_with_race_check(&socket_path).expect_err("live socket must report AlreadyRunning");
        assert!(
            matches!(error, ServerError::AlreadyRunning),
            "got {error:?}"
        );
        drop(winner);
    }

    /// A bind failure with no live daemon behind it keeps the original
    /// io error (e.g. a plain file squatting on the socket path).
    #[test]
    fn bind_failure_without_live_daemon_returns_the_io_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let socket_path = dir.path().join("squat.sock");
        std::fs::write(&socket_path, b"not a socket").expect("squat file");

        let error = bind_with_race_check(&socket_path).expect_err("dead path must fail");
        assert!(matches!(error, ServerError::Io(_)), "got {error:?}");
    }
}
