//! System API: daemon handshake and liveness for the GUI.

use std::path::Path;

use flutter_rust_bridge::frb;
use mailslate_core::infra::{ipc::Client, paths};
use mailslate_protocol::{BridgeError, PingResult};

/// Liveness of the local daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    /// The daemon answered a ping round trip.
    Running,
    /// The daemon could not be reached.
    Unavailable,
}

/// Everything the status bar needs to know about the daemon.
#[derive(Debug, Clone)]
pub struct DaemonStatus {
    /// Whether the daemon is reachable right now.
    pub state: DaemonState,
    /// The daemon's crate version, when it is reachable.
    pub daemon_version: Option<String>,
    /// The IPC protocol version, when it is reachable.
    pub protocol_version: Option<u32>,
}

/// Versions exchanged during the ping handshake.
#[derive(Debug, Clone)]
pub struct PingInfo {
    /// The IPC protocol version the daemon speaks.
    pub protocol_version: u32,
    /// The daemon's crate version.
    pub daemon_version: String,
}

/// A failure crossing the FFI boundary.
#[derive(Debug, Clone)]
pub struct DaemonError {
    /// Machine-readable kind (`ErrorKind` snake_case name).
    pub kind: String,
    /// Human-readable detail.
    pub message: String,
    /// Whether retrying the same call can succeed.
    pub retryable: bool,
}

impl From<BridgeError> for DaemonError {
    fn from(error: BridgeError) -> Self {
        Self {
            kind: error.kind.as_str().to_string(),
            message: error.message,
            retryable: error.retryable,
        }
    }
}

impl From<PingResult> for PingInfo {
    fn from(ping: PingResult) -> Self {
        Self {
            protocol_version: ping.protocol_version,
            daemon_version: ping.daemon_version,
        }
    }
}

/// Initializes flutter_rust_bridge utilities; called once at startup.
#[frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

/// Pings the daemon without spawning one.
pub async fn ping() -> Result<PingInfo, DaemonError> {
    connect_ping(&paths::socket_path()).await
}

/// Reports daemon liveness and versions; never fails.
pub async fn daemon_status() -> DaemonStatus {
    status_for(&paths::socket_path()).await
}

/// Ensures a daemon is running (spawning it when needed) and pings it.
pub async fn start_daemon() -> Result<PingInfo, DaemonError> {
    let mut client = Client::connect_or_spawn(&paths::data_dir())
        .await
        .map_err(DaemonError::from)?;
    client
        .ping()
        .await
        .map_err(DaemonError::from)
        .map(Into::into)
}

async fn connect_ping(socket: &Path) -> Result<PingInfo, DaemonError> {
    let mut client = Client::connect(socket).await.map_err(DaemonError::from)?;
    client
        .ping()
        .await
        .map_err(DaemonError::from)
        .map(Into::into)
}

async fn status_for(socket: &Path) -> DaemonStatus {
    match connect_ping(socket).await {
        Ok(info) => DaemonStatus {
            state: DaemonState::Running,
            daemon_version: Some(info.daemon_version),
            protocol_version: Some(info.protocol_version),
        },
        Err(_) => DaemonStatus {
            state: DaemonState::Unavailable,
            daemon_version: None,
            protocol_version: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailslate_protocol::async_io::{read_json_frame_async, write_json_frame_async};
    use mailslate_protocol::message::{Request, Response};
    use mailslate_protocol::{PROTOCOL_VERSION, methods};
    use serde_json::json;
    use tokio::net::UnixListener;

    async fn serve(listener: UnixListener) {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                while let Ok(request) = read_json_frame_async::<_, Request>(&mut stream).await {
                    let result = if request.method == methods::PING {
                        json!({
                            "protocol_version": PROTOCOL_VERSION,
                            "daemon_version": "9.9.9",
                        })
                    } else {
                        json!({})
                    };
                    let response = Response::ok(request.id, result);
                    if write_json_frame_async(&mut stream, &response)
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            });
        }
    }

    async fn fake_daemon_socket() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket).expect("bind fake daemon");
        tokio::spawn(serve(listener));
        (dir, socket)
    }

    #[tokio::test]
    async fn ping_reports_daemon_versions() {
        let (_dir, socket) = fake_daemon_socket().await;
        let info = connect_ping(&socket).await.expect("ping");
        assert_eq!(info.protocol_version, PROTOCOL_VERSION);
        assert_eq!(info.daemon_version, "9.9.9");
    }

    #[tokio::test]
    async fn status_reports_running_with_versions() {
        let (_dir, socket) = fake_daemon_socket().await;
        let status = status_for(&socket).await;
        assert_eq!(status.state, DaemonState::Running);
        assert_eq!(status.daemon_version.as_deref(), Some("9.9.9"));
        assert_eq!(status.protocol_version, Some(PROTOCOL_VERSION));
    }

    #[tokio::test]
    async fn status_is_unavailable_without_a_daemon() {
        let dir = tempfile::tempdir().expect("tempdir");
        let status = status_for(&dir.path().join("absent.sock")).await;
        assert_eq!(status.state, DaemonState::Unavailable);
        assert!(status.daemon_version.is_none());
        assert!(status.protocol_version.is_none());
    }

    #[tokio::test]
    async fn daemon_errors_map_to_kind_message_and_retryable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = connect_ping(&dir.path().join("absent.sock"))
            .await
            .expect_err("connect must fail");
        assert_eq!(error.kind, "daemon_unavailable");
        assert!(error.retryable);
        assert!(!error.message.is_empty());
    }
}
