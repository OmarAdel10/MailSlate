//! End-to-end IPC tests: spawn the real daemon and speak the real wire format.
#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use mailslate_protocol::{MAX_FRAME_BYTES, Request, Response, read_json_frame, write_json_frame};

/// A daemon process under test with an isolated profile and runtime dir.
struct Daemon {
    child: Child,
    data: tempfile::TempDir,
    runtime: tempfile::TempDir,
    socket: PathBuf,
}

impl Daemon {
    fn start() -> Self {
        let data = tempfile::tempdir().expect("data dir");
        let runtime = tempfile::tempdir().expect("runtime dir");
        let socket = socket_path(&runtime);
        let mut child = spawn(data.path(), runtime.path());
        wait_ready(&socket, &mut child);
        Self {
            child,
            data,
            runtime,
            socket,
        }
    }

    fn connect(&self) -> UnixStream {
        UnixStream::connect(&self.socket).expect("connect to daemon")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn socket_path(runtime: &tempfile::TempDir) -> PathBuf {
    runtime.path().join("mailslate").join("daemon.sock")
}

fn spawn(data: &Path, runtime: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_mailslate-daemon"))
        .env("MAILSLATE_DATA_DIR", data)
        .env("XDG_RUNTIME_DIR", runtime)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon")
}

fn wait_ready(socket: &Path, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if socket.exists() && UnixStream::connect(socket).is_ok() {
            return;
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("daemon exited before becoming ready: {status:?}");
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not become ready in time"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_exit(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return status;
        }
        assert!(Instant::now() < deadline, "daemon did not exit in time");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn call(stream: &mut UnixStream, id: u64, method: &str) -> Response {
    write_json_frame(stream, &Request::new(id, method)).expect("write request");
    read_json_frame(stream).expect("read response")
}

#[test]
fn ping_round_trips_and_shutdown_stops_the_daemon() {
    let mut daemon = Daemon::start();
    let mut stream = daemon.connect();

    let ping = call(&mut stream, 1, "ping");
    assert!(ping.ok);
    let result = ping.result.expect("ping result");
    assert_eq!(result["protocol_version"], serde_json::json!(1));
    assert_eq!(
        result["daemon_version"],
        serde_json::json!(env!("CARGO_PKG_VERSION"))
    );

    let shutdown = call(&mut stream, 2, "shutdown");
    assert!(shutdown.ok);

    let status = wait_for_exit(&mut daemon.child);
    assert!(status.success(), "daemon must exit cleanly, got {status:?}");
}

#[test]
fn unknown_methods_answer_not_found() {
    let daemon = Daemon::start();
    let mut stream = daemon.connect();

    let response = call(&mut stream, 1, "does-not-exist");
    assert!(!response.ok);
    let error = response.error.expect("error payload");
    assert_eq!(error.kind.as_str(), "not_found");

    call(&mut stream, 2, "shutdown");
}

#[test]
fn second_instance_exits_cleanly_and_leaves_the_first_running() {
    let daemon = Daemon::start();

    let mut second = spawn(daemon.data.path(), daemon.runtime.path());
    let status = wait_for_exit(&mut second);
    assert!(
        status.success(),
        "second instance must exit 0, got {status:?}"
    );

    let mut stream = daemon.connect();
    let ping = call(&mut stream, 1, "ping");
    assert!(ping.ok, "first daemon must still serve");
    call(&mut stream, 2, "shutdown");
}

#[test]
fn stale_socket_is_replaced() {
    let data = tempfile::tempdir().expect("data dir");
    let runtime = tempfile::tempdir().expect("runtime dir");
    let socket = socket_path(&runtime);
    std::fs::create_dir_all(socket.parent().expect("socket parent")).expect("create socket dir");
    let stale = std::os::unix::net::UnixListener::bind(&socket).expect("bind stale socket");
    drop(stale); // the file remains, nothing listens

    let mut child = spawn(data.path(), runtime.path());
    wait_ready(&socket, &mut child);

    let mut stream = UnixStream::connect(&socket).expect("connect");
    let ping = call(&mut stream, 1, "ping");
    assert!(ping.ok);
    call(&mut stream, 2, "shutdown");
    let _ = child.wait();
}

#[test]
fn hostile_frames_do_not_kill_the_daemon() {
    let daemon = Daemon::start();

    // Oversized length header: rejected before any allocation.
    {
        let mut stream = daemon.connect();
        stream
            .write_all(&((MAX_FRAME_BYTES as u32) + 1).to_le_bytes())
            .expect("write hostile header");
        // The daemon drops the connection; a blocking read ends at EOF.
        let mut buf = [0u8; 1];
        let _ = stream.read(&mut buf);
    }

    // Framing-valid but not JSON.
    {
        let payload = b"not json";
        let mut stream = daemon.connect();
        let mut frame = (payload.len() as u32).to_le_bytes().to_vec();
        frame.extend_from_slice(payload);
        stream.write_all(&frame).expect("write hostile payload");
        let mut buf = [0u8; 1];
        let _ = stream.read(&mut buf);
    }

    // Still alive and serving.
    let mut stream = daemon.connect();
    let ping = call(&mut stream, 1, "ping");
    assert!(ping.ok, "daemon must survive hostile frames");
    call(&mut stream, 2, "shutdown");
}
