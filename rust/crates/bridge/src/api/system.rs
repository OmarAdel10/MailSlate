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

/// Absolute path of the daemon's log directory.
///
/// The paused banner's View log action opens the newest rotation file from
/// this directory (USER_FLOW §9); pure path math, never spawns a daemon.
pub fn daemon_log_dir() -> String {
    paths::logs_dir().to_string_lossy().into_owned()
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
    use mailslate_protocol::{methods, PROTOCOL_VERSION};
    use serde_json::json;
    use tokio::net::UnixListener;

    /// Serializes tests that mutate process-wide env vars
    /// (`MAILSLATE_DATA_DIR`, `MAILSLATE_DAEMON_PATH`,
    /// `XDG_RUNTIME_DIR`); sibling tests in this binary run in
    /// parallel. Async-aware because the guarded tests hold it across
    /// await points.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

    /// The daemon binary built next to this test binary
    /// (`rust/target/{debug,release}/mailslate-daemon`).
    fn daemon_binary_for_tests() -> std::path::PathBuf {
        let exe = std::env::current_exe().expect("test binary path");
        exe.parent()
            .and_then(std::path::Path::parent)
            .expect("target directory")
            .join("mailslate-daemon")
    }

    #[tokio::test]
    async fn start_daemon_spawns_the_binary_and_reports_running() {
        let _guard = ENV_LOCK.lock().await;
        let daemon = daemon_binary_for_tests();
        assert!(
            daemon.is_file(),
            "daemon binary not built at {}; run cargo test --workspace",
            daemon.display()
        );

        let saved_data = std::env::var_os("MAILSLATE_DATA_DIR");
        let saved_xdg = std::env::var_os("XDG_RUNTIME_DIR");
        let data = tempfile::tempdir().expect("data dir");
        std::env::set_var("MAILSLATE_DATA_DIR", data.path());
        std::env::set_var("MAILSLATE_DAEMON_PATH", &daemon);
        std::env::remove_var("XDG_RUNTIME_DIR");

        // No daemon yet: status must be unavailable and ping must fail.
        assert_eq!(daemon_status().await.state, DaemonState::Unavailable);
        assert!(ping().await.is_err());

        // start_daemon spawns the binary and pings it. The product gives a
        // spawned daemon 10 s to become ready; a cold boot on a slow or
        // oversubscribed temp filesystem can run past that, so wait a little
        // longer for the socket before failing - a broken spawn path never
        // becomes reachable, so the longer wait only tolerates slow disks.
        let info = match start_daemon().await {
            Ok(info) => info,
            Err(_) => {
                let socket = data.path().join("daemon.sock");
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
                loop {
                    if mailslate_core::infra::ipc::Client::connect(&socket)
                        .await
                        .is_ok()
                    {
                        break;
                    }
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "spawned daemon never became reachable"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                ping().await.expect("ping the spawned daemon")
            }
        };
        assert_eq!(info.protocol_version, PROTOCOL_VERSION);
        assert!(!info.daemon_version.is_empty());

        let status = daemon_status().await;
        assert_eq!(status.state, DaemonState::Running);
        assert_eq!(status.protocol_version, Some(PROTOCOL_VERSION));

        // Shut the spawned daemon down and restore the environment.
        let socket = data.path().join("daemon.sock");
        let mut client = mailslate_core::infra::ipc::Client::connect(&socket)
            .await
            .expect("connect for shutdown");
        let _ = client.request(methods::SHUTDOWN, json!(null)).await;

        match saved_data {
            Some(value) => std::env::set_var("MAILSLATE_DATA_DIR", value),
            None => std::env::remove_var("MAILSLATE_DATA_DIR"),
        }
        std::env::remove_var("MAILSLATE_DAEMON_PATH");
        match saved_xdg {
            Some(value) => std::env::set_var("XDG_RUNTIME_DIR", value),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
    }

    #[test]
    fn init_app_smoke() {
        init_app();
    }
}
