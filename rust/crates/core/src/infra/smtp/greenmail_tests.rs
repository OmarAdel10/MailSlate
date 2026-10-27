//! Real-server SMTP integration against a local GreenMail JVM.
//!
//! Feature-gated exactly like the IMAP harness (`imap/greenmail_tests.rs`):
//! never enabled in CI, no Docker. Run locally with
//!
//! ```text
//! cargo test -p mailslate-core --features greenmail --lib greenmail_tests
//! ```
//!
//! GreenMail's `test.smtp` setup binds SMTP on 3025 and SMTPS on 3465 —
//! disjoint from the IMAP harness's 3143/3993, so both can run in one
//! suite. The submission here is plaintext password auth: GreenMail
//! serves a self-signed certificate that our webpki-rooted TLS client
//! rightly rejects, so implicit/STARTTLS paths stay unit-tested and the
//! wire-level TLS posture is exercised in production only.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::{OutgoingEnvelope, SmtpAuth, SmtpConfig, send};
use crate::domain::accounts::Security;

const SMTP_PORT: u16 = 3025;
const USER: &str = "imapuser";
const PASSWORD: &str = "imappass";

/// Kills the GreenMail JVM when the test ends, pass or fail.
struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn greenmail_jar() -> PathBuf {
    std::env::var_os("GREENMAIL_JAR").map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../.tools/greenmail-standalone-2.1.13.jar")
        },
        PathBuf::from,
    )
}

/// Polls the SMTP port until the JVM accepts connections.
fn wait_ready() {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        match TcpStream::connect(("127.0.0.1", SMTP_PORT)) {
            Ok(_) => return,
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("SMTP readiness probe failed: {error}"),
        }
    }
    panic!("GreenMail SMTP did not become ready within 30s");
}

fn start_greenmail() -> ServerGuard {
    let child = Command::new("java")
        .arg("-Dgreenmail.setup.test.smtp")
        .arg(format!("-Dgreenmail.users={USER}:{PASSWORD}"))
        .arg("-Dgreenmail.startup.timeout=5000")
        .arg("-jar")
        .arg(greenmail_jar())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("java must be on PATH to run the GreenMail integration test");
    let guard = ServerGuard(child);
    wait_ready();
    guard
}

/// A minimal well-formed RFC 5322 message; the builder is ST1's job and
/// already has its own round-trip suite.
fn wire_message() -> Vec<u8> {
    concat!(
        "From: imapuser@localhost\r\n",
        "To: imapuser@localhost\r\n",
        "Subject: GreenMail SMTP integration\r\n",
        "Date: Mon, 19 Oct 2026 00:00:00 +0000\r\n",
        "Message-ID: <greenmail-smtp-test@mailslate.invalid>\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: 7bit\r\n",
        "\r\n",
        "hello from the smtp adapter\r\n",
    )
    .as_bytes()
    .to_vec()
}

#[test]
fn password_submission_is_accepted_by_a_real_server() {
    let _server = start_greenmail();

    let config = SmtpConfig {
        host: "127.0.0.1".to_string(),
        port: SMTP_PORT,
        security: Security::Plaintext,
    };
    let auth = SmtpAuth::Password {
        username: USER.to_string(),
        password: PASSWORD.to_string(),
    };
    let envelope = OutgoingEnvelope {
        from: "imapuser@localhost".to_string(),
        to: vec!["imapuser@localhost".to_string()],
        cc: Vec::new(),
        bcc: Vec::new(),
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime
        .block_on(send(&config, &auth, &envelope, &wire_message()))
        .expect("GreenMail must accept the submission");
}
