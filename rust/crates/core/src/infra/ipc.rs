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
use mailslate_protocol::message::{Event, Request, Response};
use mailslate_protocol::{BridgeError, ErrorKind, PingResult, methods};
use serde_json::{Value, json};

use crate::infra::paths;

/// How long `connect_or_spawn` waits for a freshly spawned daemon.
const SPAWN_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Delay between connection attempts after a spawn.
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(50);

/// How long [`Client::request`] waits for the daemon's reply before giving
/// up; keeps the GUI from hanging on a wedged daemon.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A request/response channel to the daemon.
///
/// The client is **single-shot after failure**: any mid-stream error
/// (timeout, frame failure, id mismatch) poisons it, because the
/// stream may hold a late or malformed reply. The next call fails
/// fast; callers open a fresh connection instead of reusing. Clean
/// `ok=false` replies are complete exchanges and do not poison.
#[derive(Debug)]
pub struct Client {
    stream: tokio::net::UnixStream,
    next_id: u64,
    request_timeout: Duration,
    poisoned: bool,
    /// Daemon events interleaved during request/reply frames, parked for
    /// the consumer until an event API surfaces them (phase 7 wires the
    /// GUI stream).
    pending_events: std::collections::VecDeque<Event>,
}

impl Client {
    /// Connects to the daemon listening at `socket`, without spawning one.
    pub async fn connect(socket: &Path) -> Result<Self, BridgeError> {
        let stream = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(|err| {
                daemon_unavailable(format!("could not connect to {}: {err}", socket.display()))
            })?;
        check_peer_uid(&stream)?;
        Ok(Self {
            stream,
            next_id: 0,
            request_timeout: REQUEST_TIMEOUT,
            poisoned: false,
            pending_events: std::collections::VecDeque::new(),
        })
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
    ///
    /// Fails with [`ErrorKind::DaemonUnavailable`] when no reply arrives
    /// within the request timeout. Any such mid-stream failure poisons
    /// the client; the next call fails fast and the caller must open a
    /// fresh connection.
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, BridgeError> {
        if self.poisoned {
            return Err(daemon_unavailable(
                "client is poisoned: a previous request failed mid-stream, \
                 open a fresh connection"
                    .to_string(),
            ));
        }
        self.next_id += 1;
        let request = Request {
            id: self.next_id,
            method: method.to_string(),
            params,
        };
        let timeout = self.request_timeout;
        let response: Response = match tokio::time::timeout(timeout, async {
            write_json_frame_async(&mut self.stream, &request)
                .await
                .map_err(frame_error)?;
            // Events may interleave with replies: skip-and-park every
            // frame until a response envelope resolves this request.
            loop {
                let raw: Value = read_json_frame_async(&mut self.stream)
                    .await
                    .map_err(frame_error)?;
                if let Ok(event) = serde_json::from_value::<Event>(raw.clone()) {
                    self.pending_events.push_back(event);
                    continue;
                }
                let response: Response = serde_json::from_value(raw).map_err(json_error)?;
                break Ok::<Response, BridgeError>(response);
            }
        })
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                self.poisoned = true;
                return Err(err);
            }
            Err(_) => {
                self.poisoned = true;
                return Err(daemon_unavailable(format!(
                    "daemon did not answer within {timeout:?}"
                )));
            }
        };
        if response.id != request.id {
            self.poisoned = true;
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

    /// Creates a generic IMAP/SMTP account; replies with `account_id`.
    pub async fn add_imap_account(
        &mut self,
        conn: Value,
        creds: Value,
    ) -> Result<Value, BridgeError> {
        self.request(
            methods::ADD_IMAP_ACCOUNT,
            json!({ "conn": conn, "creds": creds }),
        )
        .await
    }

    /// Lists every configured account.
    pub async fn get_accounts(&mut self) -> Result<Value, BridgeError> {
        self.request(methods::GET_ACCOUNTS, json!({})).await
    }

    /// Removes an account and scrubs its keyring secrets.
    pub async fn remove_account(&mut self, account_id: i64) -> Result<Value, BridgeError> {
        self.request(methods::REMOVE_ACCOUNT, json!({ "account_id": account_id }))
            .await
    }

    /// Patches mutable account fields (name, color, sync window).
    /// Starts an OAuth authorization-code flow; the reply carries the
    /// provider's authorize URL for the GUI to open in a browser.
    pub async fn start_oauth(&mut self, provider: &str) -> Result<Value, BridgeError> {
        self.request(methods::START_OAUTH, json!({ "provider": provider }))
            .await
    }

    /// Completes a started OAuth flow: names the account and files it.
    pub async fn complete_oauth_account(
        &mut self,
        display_name: &str,
        email: &str,
        color: Option<&str>,
    ) -> Result<Value, BridgeError> {
        self.request(
            methods::COMPLETE_OAUTH_ACCOUNT,
            json!({ "display_name": display_name, "email": email, "color": color }),
        )
        .await
    }

    pub async fn update_account(
        &mut self,
        account_id: i64,
        patch: Value,
    ) -> Result<Value, BridgeError> {
        self.request(
            methods::UPDATE_ACCOUNT,
            json!({ "account_id": account_id, "patch": patch }),
        )
        .await
    }

    /// Asks the daemon to re-run the sync for one account.
    pub async fn sync_now(&mut self, account_id: i64) -> Result<Value, BridgeError> {
        self.request(methods::SYNC_NOW, json!({ "account_id": account_id }))
            .await
    }

    /// Lists every account with its current sync state.
    pub async fn get_status(&mut self) -> Result<Value, BridgeError> {
        self.request(methods::GET_STATUS, json!({})).await
    }

    /// Overrides the per-request timeout; tests use short values.
    #[cfg(test)]
    pub(crate) fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
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
        .find(|candidate| is_runnable_file(candidate))
        .unwrap_or_else(|| PathBuf::from(NAME))
}

/// A candidate we can actually execute: a regular file with at least
/// one execute bit set (unix). Without this filter a stale
/// non-executable artifact could shadow a working binary, and the
/// bare-name `PATH` fallback would resolve to anything.
#[cfg(unix)]
fn is_runnable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = path.metadata() else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

/// Non-unix platforms have no portable execute bit; a plain file
/// check is the best available filter there.
#[cfg(not(unix))]
fn is_runnable_file(path: &Path) -> bool {
    path.is_file()
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

/// Maps a serde deserialization failure on a decoded frame to the
/// "daemon sent an invalid frame" contract used by [`frame_error`].
fn json_error(err: serde_json::Error) -> BridgeError {
    BridgeError::new(
        ErrorKind::Internal,
        format!("daemon sent an invalid frame: {err}"),
    )
}

fn daemon_unavailable(message: String) -> BridgeError {
    BridgeError::new(ErrorKind::DaemonUnavailable, message)
}

/// Refuses to talk to a daemon owned by another user: a foreign-uid
/// socket could be planted in a writable directory to hijack this
/// session. Mirrors the server side of the same check: both ends of
/// a connection must see a peer running as the effective user.
fn check_peer_uid(stream: &tokio::net::UnixStream) -> Result<(), BridgeError> {
    let cred = stream
        .peer_cred()
        .map_err(|err| daemon_unavailable(format!("could not read peer credentials: {err}")))?;
    let owner = unsafe { libc::geteuid() };
    if cred.uid() != owner {
        return Err(daemon_unavailable(format!(
            "daemon is owned by another user (uid {} != {owner}); refusing to connect",
            cred.uid()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Serializes tests that mutate process-wide env vars
    /// (`MAILSLATE_DAEMON_PATH`, `XDG_RUNTIME_DIR`); sibling tests in
    /// this binary run in parallel. Async-aware because the guarded
    /// tests hold it across await points.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
                "echo-id" => Response::ok(request.id, json!({"seen": request.id})),
                "stall" => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    return;
                }
                "garbage" => {
                    use tokio::io::AsyncWriteExt;
                    let payload = b"not json";
                    stream
                        .write_all(&(payload.len() as u32).to_le_bytes())
                        .await
                        .expect("garbage length");
                    stream.write_all(payload).await.expect("garbage payload");
                    return;
                }
                "huge" => {
                    use tokio::io::AsyncWriteExt;
                    let header = ((mailslate_protocol::MAX_FRAME_BYTES as u32) + 1).to_le_bytes();
                    stream.write_all(&header).await.expect("huge header");
                    return;
                }
                "drop" => return,
                "event-then-ping" => {
                    let event =
                        Event::new("sync_progress", serde_json::json!({"done": 3, "total": 10}));
                    write_json_frame_async(&mut stream, &event)
                        .await
                        .expect("fake server writes the event frame");
                    Response::ok(request.id, json!({"seen": true}))
                }
                methods::GET_ACCOUNTS => Response::ok(request.id, json!({ "accounts": [] })),
                methods::ADD_IMAP_ACCOUNT => Response::ok(request.id, json!({ "account_id": 7 })),
                methods::START_OAUTH => Response::ok(
                    request.id,
                    json!({ "url": "https://accounts.example.test/auth" }),
                ),
                methods::COMPLETE_OAUTH_ACCOUNT => {
                    Response::ok(request.id, json!({ "account_id": 11 }))
                }
                methods::REMOVE_ACCOUNT | methods::UPDATE_ACCOUNT => {
                    Response::ok(request.id, json!({ "ok": true }))
                }
                methods::SYNC_NOW => Response::ok(request.id, json!({ "ok": true })),
                methods::GET_STATUS => Response::ok(
                    request.id,
                    json!({ "accounts": [{ "id": 3, "state": "idle" }] }),
                ),
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

    /// An unsolicited event frame that arrives while the client awaits
    /// its response is skipped, not fatal: the reply on the wire still
    /// resolves the pending request.
    #[tokio::test]
    async fn event_frame_before_response_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();
        let reply = client
            .request("event-then-ping", Value::Null)
            .await
            .unwrap();
        assert_eq!(reply["seen"], true);
    }

    /// Thin typed wrappers round-trip the account method names and
    /// parameter shapes over the wire.
    #[tokio::test]
    async fn oauth_method_wrappers_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();

        let reply = client.start_oauth("google").await.unwrap();
        assert_eq!(reply["url"], json!("https://accounts.example.test/auth"));

        let reply = client
            .complete_oauth_account("Gmail", "me@gmail.com", Some("blue"))
            .await
            .unwrap();
        assert_eq!(reply["account_id"], json!(11));
    }

    /// sync_now / get_status wrappers round-trip the wire shapes.
    #[tokio::test]
    async fn sync_method_wrappers_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();

        let reply = client.sync_now(3).await.unwrap();
        assert_eq!(reply["ok"], json!(true));

        let status = client.get_status().await.unwrap();
        assert_eq!(status["accounts"][0]["id"], json!(3));
        assert_eq!(status["accounts"][0]["state"], json!("idle"));
    }

    #[tokio::test]
    async fn account_method_wrappers_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();

        let created = client
            .add_imap_account(json!({ "imap_host": "h" }), json!({ "password": "p" }))
            .await
            .unwrap();
        assert_eq!(created["account_id"], json!(7));

        let listing = client.get_accounts().await.unwrap();
        assert_eq!(listing["accounts"], json!([]));

        client
            .update_account(7, json!({ "color": "blue" }))
            .await
            .unwrap();
        client.remove_account(7).await.unwrap();
    }

    /// The client only speaks to a daemon owned by this user: the peer
    /// credential check runs at connect and must accept the fake server
    /// (a same-uid process).
    #[tokio::test]
    async fn peer_credentials_are_verified_on_connect() {
        let dir = tempfile::tempdir().unwrap();
        let socket_path = dir.path().join("peercred.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
        tokio::spawn(serve(listener));

        let stream = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
        let checked = check_peer_uid(&stream);
        assert!(
            checked.is_ok(),
            "same-uid peer must be accepted: {checked:?}"
        );
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

    #[tokio::test]
    async fn stalled_daemon_request_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket)
            .await
            .unwrap()
            .with_request_timeout(Duration::from_millis(50));
        let err = client.request("stall", Value::Null).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::DaemonUnavailable);
        assert!(err.message.contains("did not answer"));
    }

    #[tokio::test]
    async fn invalid_frame_from_daemon_is_internal() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();
        let err = client.request("garbage", Value::Null).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Internal);
        assert!(err.message.contains("invalid frame"));
    }

    #[tokio::test]
    async fn oversized_frame_header_from_daemon_is_internal() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();
        let err = client.request("huge", Value::Null).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Internal);
        assert!(err.message.contains("invalid frame"));
    }

    #[tokio::test]
    async fn invalid_ping_payload_is_internal() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("daemon.sock");
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let request: Request = read_json_frame_async(&mut stream).await.expect("request");
            let response = Response::ok(request.id, json!("not-an-object"));
            write_json_frame_async(&mut stream, &response)
                .await
                .expect("write");
        });
        let mut client = Client::connect(&socket).await.unwrap();
        let err = client.ping().await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Internal);
        assert!(err.message.contains("invalid ping payload"));
    }

    #[tokio::test]
    async fn dropped_connection_maps_to_daemon_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();
        let err = client.request("drop", Value::Null).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::DaemonUnavailable);
        assert!(err.message.contains("connection to the daemon failed"));
    }

    #[tokio::test]
    async fn request_ids_increase_monotonically() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();
        let first = client.request("echo-id", Value::Null).await.unwrap();
        let second = client.request("echo-id", Value::Null).await.unwrap();
        assert_eq!(first["seen"], json!(1));
        assert_eq!(second["seen"], json!(2));
    }

    /// The daemon binary built next to this test binary
    /// (`rust/target/{debug,release}/mailslate-daemon`).
    fn daemon_binary_for_tests() -> PathBuf {
        let exe = std::env::current_exe().expect("test binary path");
        exe.parent()
            .and_then(Path::parent)
            .map(|dir| dir.join("mailslate-daemon"))
            .expect("target directory")
    }

    /// Covers the `MAILSLATE_DAEMON_PATH` override and the spawn-on-demand
    /// path. Kept as one test because both mutate process-wide env vars.
    #[tokio::test]
    async fn spawn_path_uses_env_overrides() {
        let _guard = ENV_LOCK.lock().await;
        let tools = tempfile::tempdir().unwrap();
        let fake = tools.path().join("fake-daemon");
        std::fs::write(&fake, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        unsafe { std::env::set_var("MAILSLATE_DAEMON_PATH", &fake) };
        assert_eq!(daemon_binary(), fake);

        // Keep sockets inside test temp dirs even on desktops that set
        // XDG_RUNTIME_DIR (it redirects socket_path_for elsewhere).
        let saved_xdg = std::env::var_os("XDG_RUNTIME_DIR");
        unsafe { std::env::remove_var("XDG_RUNTIME_DIR") };

        // Fast path: a daemon that already answers must win over spawning.
        // MAILSLATE_DAEMON_PATH still points at the empty fake file, so a
        // spawn attempt would fail this test.
        let live = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&live);
        assert_eq!(socket, paths::socket_path_for(live.path()));
        let mut client = Client::connect_or_spawn(live.path()).await.unwrap();
        assert_eq!(client.ping().await.unwrap().daemon_version, "9.9.9");

        let daemon = daemon_binary_for_tests();
        assert!(
            daemon.is_file(),
            "daemon binary not built at {}; run cargo test --workspace",
            daemon.display()
        );
        unsafe { std::env::set_var("MAILSLATE_DAEMON_PATH", &daemon) };
        let dir = tempfile::tempdir().unwrap();
        // The product gives a spawned daemon 10 s to become ready. On an
        // oversubscribed temp filesystem a cold boot can occasionally run
        // past that budget, so wait a little longer here: a broken spawn
        // path never becomes reachable, so this only tolerates slow disks.
        let mut client = match Client::connect_or_spawn(dir.path()).await {
            Ok(client) => client,
            Err(first_err) => {
                let socket = dir.path().join("daemon.sock");
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                loop {
                    match Client::connect(&socket).await {
                        Ok(client) => break client,
                        Err(_) => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "spawned daemon never became reachable: {first_err}"
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        };
        assert!(dir.path().join("daemon.sock").exists());
        let ping = client.ping().await.unwrap();
        assert_eq!(ping.protocol_version, mailslate_protocol::PROTOCOL_VERSION);
        assert!(!ping.daemon_version.is_empty());
        let _ = client.request(methods::SHUTDOWN, Value::Null).await;
        unsafe {
            std::env::remove_var("MAILSLATE_DAEMON_PATH");
            match saved_xdg {
                Some(value) => std::env::set_var("XDG_RUNTIME_DIR", value),
                None => std::env::remove_var("XDG_RUNTIME_DIR"),
            }
        }
    }

    #[test]
    fn daemon_binary_skips_non_executable_env_candidate() {
        let _guard = ENV_LOCK.blocking_lock();
        let tools = tempfile::tempdir().unwrap();
        let fake = tools.path().join("not-executable");
        std::fs::write(&fake, b"").unwrap();
        unsafe { std::env::set_var("MAILSLATE_DAEMON_PATH", &fake) };
        let resolved = daemon_binary();
        unsafe { std::env::remove_var("MAILSLATE_DAEMON_PATH") };
        assert_ne!(
            resolved, fake,
            "non-executable candidate must not shadow a runnable binary"
        );
    }

    #[tokio::test]
    async fn client_is_poisoned_after_a_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket)
            .await
            .unwrap()
            .with_request_timeout(Duration::from_millis(100));
        let err = client.request("stall", Value::Null).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::DaemonUnavailable);
        assert!(err.message.contains("did not answer"));
        // The stream is desynchronized after the timeout; reuse must
        // fail fast instead of consuming the late reply.
        let started = std::time::Instant::now();
        let err = client
            .request(methods::PING, Value::Null)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::DaemonUnavailable);
        assert!(
            err.message.contains("poisoned"),
            "expected a poisoned-client error, got: {err}"
        );
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "poisoned client must fail fast, took {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn client_is_poisoned_after_an_id_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();
        let err = client.request("bad-id", Value::Null).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Internal);
        assert!(err.message.contains("does not match"));
        let err = client
            .request(methods::PING, Value::Null)
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::DaemonUnavailable);
        assert!(
            err.message.contains("poisoned"),
            "expected a poisoned-client error, got: {err}"
        );
    }

    #[tokio::test]
    async fn client_survives_clean_daemon_errors() {
        let dir = tempfile::tempdir().unwrap();
        let socket = start_fake_server(&dir);
        let mut client = Client::connect(&socket).await.unwrap();
        let err = client.request("boom", Value::Null).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Storage);
        // An ok=false reply is a complete, in-sync exchange: the same
        // connection must stay usable.
        client.ping().await.unwrap();
    }
}
