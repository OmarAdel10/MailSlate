//! IPC client: talks to the daemon over the socket it serves.
//!
//! The GUI uses one [`Client`] per logical connection; method calls are
//! serialized on the stream and request ids increase monotonically
//! (ARCHITECTURE §5). `connect_or_spawn` starts the daemon on demand so the
//! GUI works even when nothing autostarted it at login.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mailslate_protocol::FrameError;
use mailslate_protocol::async_io::{read_json_frame_async, write_json_frame_async};
use mailslate_protocol::message::{Request, Response};
use mailslate_protocol::{BridgeError, ErrorKind, PingResult, methods};
use serde_json::Value;

use crate::infra::paths;

/// How long `connect_or_spawn` waits for a freshly spawned daemon.
const SPAWN_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Delay between connection attempts after a spawn.
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(50);

/// A request/response channel to the daemon.
#[derive(Debug)]
pub struct Client {
    stream: tokio::net::UnixStream,
    next_id: u64,
}

impl Client {
    /// Connects to the daemon listening at `socket`, without spawning one.
    pub async fn connect(socket: &Path) -> Result<Self, BridgeError> {
        let stream = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(|err| {
                daemon_unavailable(format!("could not connect to {}: {err}", socket.display()))
            })?;
        Ok(Self { stream, next_id: 0 })
    }

    /// Connects to the daemon for `data_dir`, spawning it when needed and
    /// waiting until it accepts connections.
    ///
    /// The spawned child is intentionally not waited on; a running daemon
    /// outlives the caller and exits on its own via IPC or a signal.
    pub async fn connect_or_spawn(data_dir: &Path) -> Result<Self, BridgeError> {
        let socket = paths::socket_path_for(data_dir);
        if let Ok(client) = Self::connect(&socket).await {
            return Ok(client);
        }
        spawn_daemon(data_dir)?;
        connect_with_retry(&socket).await
    }

    /// Sends one request and awaits its matching response.
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, BridgeError> {
        self.next_id += 1;
        let request = Request {
            id: self.next_id,
            method: method.to_string(),
            params,
        };
        write_json_frame_async(&mut self.stream, &request)
            .await
            .map_err(frame_error)?;
        let response: Response = read_json_frame_async(&mut self.stream)
            .await
            .map_err(frame_error)?;
        if response.id != request.id {
            return Err(BridgeError::new(
                ErrorKind::Internal,
                format!(
                    "response id {} does not match request id {}",
                    response.id, request.id
                ),
            ));
        }
        response.into_result()
    }

    /// The `ping` handshake: both sides' versions.
    pub async fn ping(&mut self) -> Result<PingResult, BridgeError> {
        let value = self.request(methods::PING, Value::Null).await?;
        serde_json::from_value(value).map_err(|err| {
            BridgeError::new(ErrorKind::Internal, format!("invalid ping payload: {err}"))
        })
    }
}

async fn connect_with_retry(socket: &Path) -> Result<Client, BridgeError> {
    let deadline = tokio::time::Instant::now() + SPAWN_CONNECT_TIMEOUT;
    loop {
        match Client::connect(socket).await {
            Ok(client) => return Ok(client),
            Err(err) if tokio::time::Instant::now() >= deadline => {
                return Err(daemon_unavailable(format!(
                    "daemon did not become ready within {SPAWN_CONNECT_TIMEOUT:?}: {err}"
                )));
            }
            Err(_) => tokio::time::sleep(CONNECT_RETRY_DELAY).await,
        }
    }
}

/// Spawns the daemon binary detached, with `data_dir` as its profile.
fn spawn_daemon(data_dir: &Path) -> Result<(), BridgeError> {
    std::process::Command::new(daemon_binary())
        .env(paths::DATA_DIR_ENV, data_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|err| daemon_unavailable(format!("could not start the daemon: {err}")))
}

/// Locates the daemon binary: `MAILSLATE_DAEMON_PATH`, an executable
/// sibling, the workspace target directory, then `PATH`.
fn daemon_binary() -> PathBuf {
    const NAME: &str = "mailslate-daemon";
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = std::env::var_os("MAILSLATE_DAEMON_PATH") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(NAME));
    }
    // Development layout: `rust/target/{debug,release}` relative to this crate.
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    candidates.push(target.join("debug").join(NAME));
    candidates.push(target.join("release").join(NAME));
    candidates.push(Path::new("rust/target/debug").join(NAME));
    candidates.push(Path::new("rust/target/release").join(NAME));
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(NAME))
}

fn frame_error(err: FrameError) -> BridgeError {
    match err {
        FrameError::Io(io) => daemon_unavailable(format!("connection to the daemon failed: {io}")),
        other => BridgeError::new(
            ErrorKind::Internal,
            format!("daemon sent an invalid frame: {other}"),
        ),
    }
}

fn daemon_unavailable(message: String) -> BridgeError {
    BridgeError::new(ErrorKind::DaemonUnavailable, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A fake daemon: answers `ping`, errors on `boom`, and replies with a
    /// wrong id for `bad-id`.
    async fn serve(listener: tokio::net::UnixListener) {
        let (mut stream, _) = listener.accept().await.expect("accept");
        loop {
            let request: Request = match read_json_frame_async(&mut stream).await {
                Ok(request) => request,
                Err(_) => return,
            };
            let response = match request.method.as_str() {
                methods::PING => Response::ok(
                    request.id,
                    json!({"protocol_version": 1, "daemon_version": "9.9.9"}),
                ),
                "boom" => Response::err(
                    request.id,
                    BridgeError::new(ErrorKind::Storage, "disk busy"),
                ),
                "bad-id" => Response::ok(request.id + 1, Value::Null),
                _ => Response::ok(request.id, Value::Null),
            };
            write_json_frame_async(&mut stream, &response)
                .await
                .expect("write response");
        }
    }

    fn start_fake_server(dir: &tempfile::TempDir) -> PathBuf {
        let socket = dir.path().join("daemon.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        tokio::spawn(serve(listener));
        socket
    }

    #[tokio::test]
    async fn ping_round_trips_through_the_wire_format() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();
        let ping = client.ping().await.unwrap();
        assert_eq!(ping.protocol_version, 1);
        assert_eq!(ping.daemon_version, "9.9.9");
    }

    #[tokio::test]
    async fn daemon_errors_surface_as_bridge_errors() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();
        let err = client.request("boom", Value::Null).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Storage);
        assert_eq!(err.message, "disk busy");
        assert!(!err.retryable);
    }

    #[tokio::test]
    async fn mismatched_response_ids_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();
        let err = client.request("bad-id", Value::Null).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Internal);
        assert!(err.message.contains("does not match"));
    }

    #[tokio::test]
    async fn connect_failure_is_daemon_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let err = Client::connect(&dir.path().join("absent.sock"))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::DaemonUnavailable);
    }
}
