//! System API: daemon handshake and liveness for the GUI.

use std::path::Path;

use crate::frb_generated::StreamSink;
use flutter_rust_bridge::frb;
use mailslate_core::infra::events::{spawn_event_stream, EventStreamOptions};
use mailslate_core::infra::{ipc::Client, paths};
use mailslate_protocol::message::Event as WireEvent;
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

/// Daemon events surfaced to the GUI as a Dart stream.
///
/// Each variant mirrors a daemon EventBus event the GUI reacts to
/// (ARCHITECTURE §5.4); anything the bridge does not model arrives as
/// [`AppEvent::Other`] so an unknown event never breaks the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// `account_status`: an account's sync state changed.
    AccountStatus {
        /// Row id of the account.
        account: i64,
        /// Sync state name (`SyncState` snake_case).
        state: String,
    },
    /// `sync_progress`: a mailbox batch finished during a sync pass.
    SyncProgress {
        /// Row id of the account.
        account: i64,
        /// Mailbox the batch belongs to.
        mailbox: String,
        /// Sync phase name (currently `initial`).
        phase: String,
        /// Mailboxes completed so far.
        done: u32,
        /// Total mailboxes in this sync.
        total: u32,
    },
    /// `oauth_url_ready`: the provider consent screen is ready.
    OauthUrlReady {
        /// URL to open in the system browser.
        url: String,
    },
    /// `oauth_completed`: the OAuth handshake finished for this email.
    OauthCompleted {
        /// Email address that just signed in.
        email: String,
    },
    /// `oauth_failed`: the OAuth handshake failed.
    OauthFailed {
        /// Why the handshake failed.
        reason: String,
    },
    /// `oauth_reconnected`: a reconnect flow refreshed an existing
    /// account's tokens and its sync actor was restarted.
    OauthReconnected {
        /// Row id of the reconnected account.
        account: i64,
        /// Email address the provider returned.
        email: String,
    },
    /// Any other event: unknown name or malformed payload.
    Other {
        /// The `event` field from the wire frame.
        name: String,
    },
}

/// Streams daemon events to the GUI for the whole app lifetime.
///
/// Backs the global event bloc (ARCHITECTURE §4/§6): the underlying
/// loop heartbeats to defeat the daemon's idle timeout and reconnects
/// with backoff while the daemon is down; the Dart stream ends only
/// when the listener cancels.
pub async fn app_events(sink: StreamSink<AppEvent>) -> Result<(), DaemonError> {
    let events = spawn_event_stream(paths::socket_path(), EventStreamOptions::default());
    pump_events(events, |event| sink.add(event).is_ok()).await;
    Ok(())
}

/// Maps a wire [`WireEvent`] to an [`AppEvent`].
///
/// Known payloads are parsed field by field; unknown names and
/// malformed payloads collapse to [`AppEvent::Other`] so one bad
/// frame cannot break the stream.
fn app_event_from_wire(event: &WireEvent) -> AppEvent {
    let data = &event.data;
    let other = || AppEvent::Other {
        name: event.event.clone(),
    };
    match event.event.as_str() {
        "account_status" => match (
            data.get("account").and_then(|value| value.as_i64()),
            data.get("state").and_then(|value| value.as_str()),
        ) {
            (Some(account), Some(state)) => AppEvent::AccountStatus {
                account,
                state: state.to_string(),
            },
            _ => other(),
        },
        "sync_progress" => {
            match (
                data.get("account").and_then(|value| value.as_i64()),
                data.get("mailbox").and_then(|value| value.as_str()),
                data.get("phase").and_then(|value| value.as_str()),
                data.get("done").and_then(|value| value.as_u64()),
                data.get("total").and_then(|value| value.as_u64()),
            ) {
                (Some(account), Some(mailbox), Some(phase), Some(done), Some(total)) => {
                    match (u32::try_from(done), u32::try_from(total)) {
                        (Ok(done), Ok(total)) => AppEvent::SyncProgress {
                            account,
                            mailbox: mailbox.to_string(),
                            phase: phase.to_string(),
                            done,
                            total,
                        },
                        _ => other(),
                    }
                }
                _ => other(),
            }
        }
        "oauth_url_ready" => match data.get("url").and_then(|value| value.as_str()) {
            Some(url) => AppEvent::OauthUrlReady {
                url: url.to_string(),
            },
            None => other(),
        },
        "oauth_completed" => match data.get("email").and_then(|value| value.as_str()) {
            Some(email) => AppEvent::OauthCompleted {
                email: email.to_string(),
            },
            None => other(),
        },
        "oauth_failed" => match data.get("reason").and_then(|value| value.as_str()) {
            Some(reason) => AppEvent::OauthFailed {
                reason: reason.to_string(),
            },
            None => other(),
        },
        "oauth_reconnected" => match (
            data.get("account").and_then(|value| value.as_i64()),
            data.get("email").and_then(|value| value.as_str()),
        ) {
            (Some(account), Some(email)) => AppEvent::OauthReconnected {
                account,
                email: email.to_string(),
            },
            _ => other(),
        },
        _ => other(),
    }
}

/// Forwards mapped [`AppEvent`]s until `emit` returns `false` (the
/// Dart side cancelled) or the core stream ends.
async fn pump_events<F: FnMut(AppEvent) -> bool>(
    mut events: tokio::sync::mpsc::Receiver<WireEvent>,
    mut emit: F,
) {
    while let Some(event) = events.recv().await {
        if !emit(app_event_from_wire(&event)) {
            break;
        }
    }
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
pub(crate) mod tests {
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
    /// await points. Shared with the accounts API tests: one lock, one
    /// environment.
    pub(crate) static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

    #[test]
    fn daemon_log_dir_reports_the_data_dir_logs_path() {
        let _guard = ENV_LOCK.blocking_lock();

        let dir = tempfile::tempdir().expect("tempdir for the data dir");
        let saved = std::env::var("MAILSLATE_DATA_DIR").ok();
        std::env::set_var("MAILSLATE_DATA_DIR", dir.path());

        let reported = daemon_log_dir();

        match saved {
            Some(value) => std::env::set_var("MAILSLATE_DATA_DIR", value),
            None => std::env::remove_var("MAILSLATE_DATA_DIR"),
        }

        let expected = dir.path().join("logs");
        assert_eq!(Path::new(&reported), expected);
    }

    #[test]
    fn maps_account_status_to_its_variant() {
        let wire = WireEvent::new("account_status", json!({"account": 7, "state": "syncing"}));
        assert_eq!(
            app_event_from_wire(&wire),
            AppEvent::AccountStatus {
                account: 7,
                state: "syncing".to_string(),
            }
        );
    }

    #[test]
    fn maps_sync_progress_to_its_variant() {
        let wire = WireEvent::new(
            "sync_progress",
            json!({"account": 3, "mailbox": "INBOX", "phase": "initial", "done": 2, "total": 5}),
        );
        assert_eq!(
            app_event_from_wire(&wire),
            AppEvent::SyncProgress {
                account: 3,
                mailbox: "INBOX".to_string(),
                phase: "initial".to_string(),
                done: 2,
                total: 5,
            }
        );
    }

    #[test]
    fn maps_oauth_url_ready_to_its_variant() {
        let wire = WireEvent::new(
            "oauth_url_ready",
            json!({"url": "https://example.com/auth"}),
        );
        assert_eq!(
            app_event_from_wire(&wire),
            AppEvent::OauthUrlReady {
                url: "https://example.com/auth".to_string(),
            }
        );
    }

    #[test]
    fn maps_oauth_completed_to_its_variant() {
        let wire = WireEvent::new("oauth_completed", json!({"email": "user@example.com"}));
        assert_eq!(
            app_event_from_wire(&wire),
            AppEvent::OauthCompleted {
                email: "user@example.com".to_string(),
            }
        );
    }

    #[test]
    fn maps_oauth_failed_to_its_variant() {
        let wire = WireEvent::new("oauth_failed", json!({"reason": "denied"}));
        assert_eq!(
            app_event_from_wire(&wire),
            AppEvent::OauthFailed {
                reason: "denied".to_string(),
            }
        );
    }

    #[test]
    fn maps_oauth_reconnected_to_its_variant() {
        let wire = WireEvent::new(
            "oauth_reconnected",
            json!({"account": 7, "email": "user@example.com"}),
        );
        assert_eq!(
            app_event_from_wire(&wire),
            AppEvent::OauthReconnected {
                account: 7,
                email: "user@example.com".to_string(),
            }
        );
    }

    #[test]
    fn malformed_oauth_reconnected_falls_back_to_other() {
        let wire = WireEvent::new("oauth_reconnected", json!({"email": "user@example.com"}));
        assert_eq!(
            app_event_from_wire(&wire),
            AppEvent::Other {
                name: "oauth_reconnected".to_string(),
            }
        );
    }

    #[test]
    fn unknown_event_names_map_to_other() {
        let wire = WireEvent::new("mail_new", json!({"thread": 12}));
        assert_eq!(
            app_event_from_wire(&wire),
            AppEvent::Other {
                name: "mail_new".to_string(),
            }
        );
    }

    #[test]
    fn malformed_known_payloads_map_to_other() {
        // Known name, payload missing a required field.
        let missing = WireEvent::new("account_status", json!({"account": 7}));
        assert_eq!(
            app_event_from_wire(&missing),
            AppEvent::Other {
                name: "account_status".to_string(),
            }
        );

        // Known name, wrong field types: `done` is a string and
        // `account` is a float.
        let wrong = WireEvent::new(
            "sync_progress",
            json!({"account": 1.5, "mailbox": "INBOX", "phase": "initial", "done": "2", "total": 5}),
        );
        assert_eq!(
            app_event_from_wire(&wrong),
            AppEvent::Other {
                name: "sync_progress".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn pump_forwards_events_in_order() {
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        for state in ["syncing", "idle", "error"] {
            sender
                .send(WireEvent::new(
                    "account_status",
                    json!({"account": 1, "state": state}),
                ))
                .await
                .expect("send");
        }
        drop(sender);

        let mut received = Vec::new();
        pump_events(receiver, |event| {
            received.push(event);
            true
        })
        .await;

        assert_eq!(
            received,
            vec![
                AppEvent::AccountStatus {
                    account: 1,
                    state: "syncing".to_string(),
                },
                AppEvent::AccountStatus {
                    account: 1,
                    state: "idle".to_string(),
                },
                AppEvent::AccountStatus {
                    account: 1,
                    state: "error".to_string(),
                },
            ]
        );
    }

    #[tokio::test]
    async fn pump_stops_when_emit_returns_false() {
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        for state in ["syncing", "idle", "error"] {
            sender
                .send(WireEvent::new(
                    "account_status",
                    json!({"account": 1, "state": state}),
                ))
                .await
                .expect("send");
        }
        drop(sender);

        let mut count = 0;
        pump_events(receiver, |_| {
            count += 1;
            false
        })
        .await;

        assert_eq!(count, 1);
    }
}
