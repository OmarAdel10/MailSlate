//! Daemon event stream: a dedicated connection forwarding every daemon
//! event to the GUI for the app's lifetime (ARCHITECTURE.md §5.4).
//!
//! The GUI's event connection is separate from its request connections.
//! The daemon drops idle connections after 60 s (`READ_TIMEOUT` in the
//! server), so this loop heartbeats with `ping` requests and reconnects
//! with exponential backoff whenever the daemon restarts. The loop exits
//! only when the consumer drops its receiver.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mailslate_protocol::async_io::{read_json_frame_async, write_json_frame_async};
use mailslate_protocol::message::{Event, Request, Response};
use mailslate_protocol::{BridgeError, ErrorKind, methods};
use serde_json::Value;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use crate::infra::ipc::check_peer_uid;

/// Tuning knobs for the event stream loop; tests inject short values.
///
/// Defaults keep the daemon's 60 s idle timeout at bay (heartbeat 30 s)
/// and settle reconnect attempts at a 15 s ceiling.
#[derive(Debug, Clone)]
pub struct EventStreamOptions {
    /// `ping` cadence; must stay well under the daemon's 60 s read timeout.
    pub heartbeat: Duration,
    /// Delay before the first reconnect attempt and after each successful
    /// connection (the exponential backoff floor).
    pub retry_initial: Duration,
    /// Ceiling the reconnect backoff doubles up to.
    pub retry_max: Duration,
}

impl Default for EventStreamOptions {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_secs(30),
            retry_initial: Duration::from_millis(500),
            retry_max: Duration::from_secs(15),
        }
    }
}

/// Spawns the daemon event loop for `socket` and returns the receiver
/// every forwarded event arrives on.
///
/// The loop runs for the receiver's lifetime: it reconnects forever
/// (the GUI outlives daemon restarts) and stops once the receiver is
/// dropped.
pub fn spawn_event_stream(socket: PathBuf, options: EventStreamOptions) -> mpsc::Receiver<Event> {
    let (sender, receiver) = mpsc::channel(1024);
    tokio::spawn(run_event_stream(socket, options, sender));
    receiver
}

/// The event loop body, separated from [`spawn_event_stream`] so tests
/// can await it directly.
///
/// Returns once the consumer drops its receiver; connection failures
/// are retried, never fatal.
pub async fn run_event_stream(
    socket: PathBuf,
    options: EventStreamOptions,
    sender: mpsc::Sender<Event>,
) {
    let mut retry_delay = options.retry_initial;
    loop {
        if let Ok(stream) = connect_event_connection(&socket).await {
            retry_delay = options.retry_initial;
            if event_connection_loop(stream, &options, &sender).await {
                return;
            }
        }
        if sender.is_closed() {
            return;
        }
        tokio::time::sleep(retry_delay).await;
        retry_delay = retry_delay.saturating_mul(2).min(options.retry_max);
    }
}

/// Connects to `socket` and verifies the peer uid, mirroring
/// [`crate::infra::ipc::Client::connect`].
async fn connect_event_connection(socket: &Path) -> Result<UnixStream, BridgeError> {
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|err| {
            BridgeError::new(
                ErrorKind::DaemonUnavailable,
                format!("could not connect to {}: {err}", socket.display()),
            )
        })?;
    check_peer_uid(&stream)?;
    Ok(stream)
}

/// Serves one connection until the consumer goes away.
///
/// Returns `true` when the receiver is dropped (the whole loop ends)
/// and `false` when the connection dies (the caller reconnects).
async fn event_connection_loop(
    mut stream: UnixStream,
    options: &EventStreamOptions,
    sender: &mpsc::Sender<Event>,
) -> bool {
    let mut heartbeat = tokio::time::interval(options.heartbeat);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let _ = heartbeat.tick().await; // consume the immediate first tick
    let mut next_id = 0u64;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                if sender.is_closed() {
                    return true;
                }
                next_id += 1;
                let request = Request {
                    id: next_id,
                    method: methods::PING.to_string(),
                    params: Value::Null,
                };
                if write_json_frame_async(&mut stream, &request).await.is_err() {
                    return false;
                }
            }
            frame = read_json_frame_async::<_, Value>(&mut stream) => {
                let raw = match frame {
                    Ok(raw) => raw,
                    Err(_) => return false,
                };
                if let Ok(event) = serde_json::from_value::<Event>(raw.clone()) {
                    if sender.send(event).await.is_err() {
                        return true;
                    }
                } else if serde_json::from_value::<Response>(raw).is_err() {
                    // Neither an event nor a response: the stream is
                    // desynchronized; a fresh connection is the only
                    // recovery.
                    return false;
                }
                // Ping replies are ignored: heartbeats are fire-and-forget.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailslate_protocol::{PROTOCOL_VERSION, message::Request};
    use serde_json::json;
    use tokio::net::UnixListener;

    /// Accepts one connection and serves it with [`serve_stream`].
    async fn serve_single_connection(
        listener: UnixListener,
        idle: Duration,
        events: Vec<(Duration, Event)>,
    ) {
        let (stream, _) = listener.accept().await.expect("accept");
        serve_stream(stream, idle, events).await;
    }

    /// Per-connection handler: publishes `events` at their offsets,
    /// answers pings, and drops the connection after `idle` without a
    /// request (mirrors the daemon's READ_TIMEOUT).
    async fn serve_stream(
        mut stream: tokio::net::UnixStream,
        idle: Duration,
        events: Vec<(Duration, Event)>,
    ) {
        let start = tokio::time::Instant::now();
        let mut idle_deadline = start + idle;
        let mut pending: Vec<(tokio::time::Instant, Event)> = events
            .into_iter()
            .map(|(delay, event)| (start + delay, event))
            .collect();
        loop {
            let next_event = pending
                .iter()
                .map(|(at, _)| *at)
                .min()
                .unwrap_or(start + Duration::from_secs(600));
            tokio::select! {
                _ = tokio::time::sleep_until(idle_deadline) => {
                    use tokio::io::AsyncWriteExt;
                    let _ = stream.shutdown().await;
                    return;
                }
                _ = tokio::time::sleep_until(next_event) => {
                    let (index, _) = pending
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, (at, _))| *at)
                        .expect("event due");
                    let (_, event) = pending.swap_remove(index);
                    write_json_frame_async(&mut stream, &event)
                        .await
                        .expect("server writes the event frame");
                }
                frame = read_json_frame_async::<_, Request>(&mut stream) => {
                    let request = frame.expect("server reads a request");
                    let response = Response::ok(
                        request.id,
                        json!({"protocol_version": PROTOCOL_VERSION, "daemon_version": "9.9.9"}),
                    );
                    write_json_frame_async(&mut stream, &response)
                        .await
                        .expect("server answers the request");
                    idle_deadline = tokio::time::Instant::now() + idle;
                }
            }
        }
    }

    /// Publishes `events`, then closes the connection abruptly — a
    /// daemon restart or crash, indifferent to heartbeats.
    async fn serve_then_close(mut stream: tokio::net::UnixStream, events: Vec<Event>) {
        for event in events {
            write_json_frame_async(&mut stream, &event)
                .await
                .expect("server writes the event frame");
        }
        use tokio::io::AsyncWriteExt;
        let _ = stream.shutdown().await;
    }

    fn temp_socket(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        dir.path().join(name)
    }

    /// Events published while the consumer listens arrive on the
    /// receiver, payload intact.
    #[tokio::test]
    async fn events_are_forwarded_to_the_receiver() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = temp_socket(&dir, "events.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        tokio::spawn(serve_single_connection(
            listener,
            Duration::from_secs(60),
            vec![(
                Duration::from_millis(20),
                Event::new("account_status", json!({"account": 7, "state": "idle"})),
            )],
        ));

        let mut receiver = spawn_event_stream(
            socket.clone(),
            EventStreamOptions {
                heartbeat: Duration::from_millis(50),
                ..EventStreamOptions::default()
            },
        );
        let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("event arrives within the timeout")
            .expect("stream alive");
        assert_eq!(
            event,
            Event::new("account_status", json!({"account": 7, "state": "idle"}))
        );
    }

    /// The daemon drops idle connections; heartbeat pings must keep the
    /// connection alive so a later event still arrives on the same
    /// connection (the listener never accepts a second one).
    #[tokio::test]
    async fn heartbeat_pings_keep_the_connection_alive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = temp_socket(&dir, "heartbeat.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        tokio::spawn(serve_single_connection(
            listener,
            // Drop the connection after 120 ms without a request, and
            // only publish the event at 300 ms — surviving proves the
            // 30 ms heartbeats landed.
            Duration::from_millis(120),
            vec![(
                Duration::from_millis(300),
                Event::new("sync_progress", json!({"done": 3, "total": 10})),
            )],
        ));

        let mut receiver = spawn_event_stream(
            socket.clone(),
            EventStreamOptions {
                heartbeat: Duration::from_millis(30),
                ..EventStreamOptions::default()
            },
        );
        let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("event arrives within the timeout")
            .expect("stream alive");
        assert_eq!(
            event,
            Event::new("sync_progress", json!({"done": 3, "total": 10}))
        );
    }

    /// A daemon that is not there yet must not kill the stream: the
    /// loop retries until the socket starts accepting.
    #[tokio::test]
    async fn reconnects_until_the_daemon_appears() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = temp_socket(&dir, "late.sock");

        let late = socket.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            let listener = UnixListener::bind(&late).expect("bind");
            serve_single_connection(
                listener,
                Duration::from_secs(60),
                vec![(
                    Duration::from_millis(10),
                    Event::new(
                        "oauth_url_ready",
                        json!({"url": "https://example.test/auth"}),
                    ),
                )],
            )
            .await;
        });

        let mut receiver = spawn_event_stream(
            socket.clone(),
            EventStreamOptions {
                heartbeat: Duration::from_millis(50),
                retry_initial: Duration::from_millis(10),
                ..EventStreamOptions::default()
            },
        );
        let event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("event arrives within the timeout")
            .expect("stream alive");
        assert_eq!(
            event,
            Event::new(
                "oauth_url_ready",
                json!({"url": "https://example.test/auth"})
            )
        );
    }

    /// The daemon closing a connection (restart, crash) must not end
    /// the stream: the loop reconnects and keeps forwarding.
    #[tokio::test]
    async fn reconnects_after_the_daemon_drops_the_connection() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = temp_socket(&dir, "reconnect.sock");
        let listener = UnixListener::bind(&socket).expect("bind");

        let mut receiver = spawn_event_stream(
            socket.clone(),
            EventStreamOptions {
                heartbeat: Duration::from_millis(40),
                retry_initial: Duration::from_millis(10),
                ..EventStreamOptions::default()
            },
        );
        // Connection 1: one event, then an abrupt close (a daemon
        // restart). Connection 2: one event, stays open. Both accepts
        // are bounded so a dead loop fails the test instead of hanging.
        let first = tokio::time::timeout(Duration::from_secs(2), listener.accept())
            .await
            .expect("first accept within the timeout")
            .expect("accept")
            .0;
        tokio::spawn(serve_then_close(
            first,
            vec![Event::new(
                "account_status",
                json!({"account": 1, "state": "syncing"}),
            )],
        ));
        let second = tokio::time::timeout(Duration::from_secs(2), listener.accept())
            .await
            .expect("second accept within the timeout")
            .expect("accept")
            .0;
        tokio::spawn(serve_stream(
            second,
            Duration::from_secs(60),
            vec![(
                Duration::from_millis(10),
                Event::new("account_status", json!({"account": 1, "state": "idle"})),
            )],
        ));
        let first_event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("first event arrives")
            .expect("stream alive");
        assert_eq!(
            first_event,
            Event::new("account_status", json!({"account": 1, "state": "syncing"}))
        );
        let second_event = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("second event arrives after the reconnect")
            .expect("stream alive");
        assert_eq!(
            second_event,
            Event::new("account_status", json!({"account": 1, "state": "idle"}))
        );
    }

    /// Dropping the receiver must end the loop; nothing may spin
    /// forever against an absent daemon.
    #[tokio::test]
    async fn loop_exits_when_the_receiver_is_dropped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = temp_socket(&dir, "absent.sock");
        let (sender, receiver) = mpsc::channel(1024);
        drop(receiver);

        tokio::time::timeout(
            Duration::from_secs(1),
            run_event_stream(
                socket.clone(),
                EventStreamOptions {
                    heartbeat: Duration::from_millis(10),
                    retry_initial: Duration::from_millis(5),
                    ..EventStreamOptions::default()
                },
                sender,
            ),
        )
        .await
        .expect("loop exits once the receiver is dropped");
    }
}
