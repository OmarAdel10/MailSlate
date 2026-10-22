//! GreenMail integration: the IMAP adapter drives a real server.
//!
//! Compiled only under `cargo test --features greenmail` (see the
//! module gate in `mod.rs`) because it needs the pinned standalone jar
//! (`.tools/greenmail-standalone-2.1.13.jar`, fetched by
//! `tool/fetch_greenmail.sh`) and a local JVM. CI never enables the
//! feature; run it locally with:
//!
//! ```text
//! cargo test -p mailslate-core --features greenmail --lib greenmail_tests
//! ```
//!
//! One JVM per test binary run keeps the suite fast and avoids port
//! churn; the child is killed even on panic via a Drop guard.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::{connect, mailbox};
use crate::domain::accounts::Security;

const IMAP_PORT: u16 = 3143;
const USER: &str = "imapuser";
const PASSWORD: &str = "imappass";

/// Kills the JVM even if an assertion panics.
struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn jar_path() -> PathBuf {
    if let Ok(override_path) = std::env::var("GREENMAIL_JAR") {
        return PathBuf::from(override_path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.tools/greenmail-standalone-2.1.13.jar")
}

fn spawn_server() -> ServerGuard {
    let jar = jar_path();
    assert!(
        jar.is_file(),
        "GreenMail jar missing at {}; run tool/fetch_greenmail.sh",
        jar.display()
    );
    let child = Command::new("java")
        .args([
            "-Dgreenmail.setup.test.imap",
            "-Dgreenmail.users=imapuser:imappass",
            "-Dgreenmail.startup.timeout=5000",
        ])
        .arg("-jar")
        .arg(&jar)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn greenmail jvm");
    ServerGuard(child)
}

fn wait_ready() {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match TcpStream::connect(("127.0.0.1", IMAP_PORT)) {
            Ok(_) => return,
            Err(err) if err.kind() == ErrorKind::ConnectionRefused => {
                assert!(
                    Instant::now() < deadline,
                    "greenmail did not become ready in time"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => panic!("unexpected probe error: {err}"),
        }
    }
}

const PROBE_MESSAGE: &[u8] = b"From: sender@example.com\r\n\
To: imapuser@localhost\r\n\
Subject: greenmail probe\r\n\
Date: Mon, 19 Oct 2026 00:00:00 +0000\r\n\
Message-ID: <probe-1@example.com>\r\n\
\r\n\
probe body\r\n";

#[tokio::test(flavor = "multi_thread")]
async fn adapter_drives_a_real_greenmail_server() {
    let _server = spawn_server();
    wait_ready();

    let client = connect::connect("127.0.0.1", IMAP_PORT, Security::StartTls)
        .await
        .expect("connect to greenmail");
    let mut session = connect::login_password(client, USER, PASSWORD)
        .await
        .expect("login to greenmail");

    let listed = mailbox::list_mailboxes(&mut session)
        .await
        .expect("list mailboxes");
    assert!(
        listed.iter().any(|entry| entry.name == "INBOX"),
        "INBOX must be listed, got {:?}",
        listed
    );

    let empty = mailbox::select_mailbox(&mut session, "INBOX")
        .await
        .expect("select INBOX");
    assert_eq!(empty.exists, 0, "fresh greenmail INBOX must be empty");

    session
        .append("INBOX", None, None, PROBE_MESSAGE)
        .await
        .expect("append probe message");

    let seeded = mailbox::select_mailbox(&mut session, "INBOX")
        .await
        .expect("reselect INBOX");
    assert_eq!(seeded.exists, 1, "appended message must be visible");

    let rows = mailbox::fetch_envelope_chunk(&mut session, "1")
        .await
        .expect("fetch probe envelope");
    assert_eq!(rows.len(), 1, "one probe message must fetch one row");
    assert_eq!(rows[0].uid, Some(1));
    let header = rows[0]
        .header
        .as_deref()
        .expect("probe row must carry a header");
    assert!(
        header
            .windows(b"Subject: greenmail probe".len())
            .any(|window| window == b"Subject: greenmail probe"),
        "header must carry the probe subject, got {:?}",
        String::from_utf8_lossy(header)
    );

    let _ = session.logout().await;
}
