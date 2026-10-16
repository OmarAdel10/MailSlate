//! IPC server lifecycle: single-instance guard, socket, accept loop.
//!
//! The wire protocol lives in `mailslate-protocol`; this module owns the
//! Unix domain socket (ARCHITECTURE.md §5). Windows named-pipe support lands
//! with the Windows build.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mailslate_protocol::FrameError;
use mailslate_protocol::async_io::{read_json_frame_async, write_json_frame_async};
use mailslate_protocol::message::{Request, Response};

use crate::dispatch::{self, Dispatched};
use crate::logging::LoggingError;

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
}

/// Serves the IPC protocol on `socket_path` until shutdown.
///
/// Exits with [`ServerError::AlreadyRunning`] when a live daemon already
/// answers on the socket; a stale socket file is removed and rebound.
#[cfg(unix)]
pub async fn run(socket_path: PathBuf, pid_file: PathBuf) -> Result<(), ServerError> {
    prepare_socket(&socket_path)?;

    let listener = tokio::net::UnixListener::bind(&socket_path)?;
    set_socket_permissions(&socket_path)?;
    std::fs::write(&pid_file, std::process::id().to_string())?;

    tracing::info!(
        socket = %socket_path.display(),
        version = env!("CARGO_PKG_VERSION"),
        "mailslate-daemon listening"
    );

    let shutdown = Arc::new(tokio::sync::Notify::new());
    let result = accept_loop(listener, shutdown).await;

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
) -> Result<(), ServerError> {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
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
                let (stream, _) = accepted?;
                let shutdown = Arc::clone(&shutdown);
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(stream, shutdown).await {
                        tracing::debug!(error = %err, "connection closed with error");
                    }
                });
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn handle_connection(
    mut stream: tokio::net::UnixStream,
    shutdown: Arc<tokio::sync::Notify>,
) -> Result<(), ConnError> {
    check_peer_uid(&stream)?;
    loop {
        let request: Request = match read_json_frame_async(&mut stream).await {
            Ok(request) => request,
            Err(FrameError::Io(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(()); // client disconnected
            }
            Err(err) => return Err(ConnError::Frame(err)),
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
