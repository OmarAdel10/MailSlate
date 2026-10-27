//! Unit tests for the SMTP submission adapter.
//!
//! Every test runs against an in-process fake SMTP server, so the
//! protocol conversation, the envelope, and the failure taxonomy are
//! observable without a network or a real MTA.

use super::*;
use crate::domain::accounts::Security;
use base64::Engine as _;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const OK_MAIL: &str = "250 2.1.0 sender ok";
const OK_RCPT: &str = "250 2.1.5 recipient ok";

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthReply {
    Accept,
    Reject,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DataReply {
    Accept,
    Drop,
}

#[derive(Clone, Copy)]
struct Script {
    auth: AuthReply,
    mail_from: &'static str,
    rcpt: &'static str,
    data: DataReply,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            auth: AuthReply::Accept,
            mail_from: OK_MAIL,
            rcpt: OK_RCPT,
            data: DataReply::Accept,
        }
    }
}

#[derive(Default)]
struct Captured {
    connections: usize,
    auth_lines: Vec<String>,
    mail_from: Vec<String>,
    rcpt_to: Vec<String>,
    message: Vec<u8>,
}

struct FakeSmtp {
    port: u16,
    captured: Arc<Mutex<Captured>>,
}

impl FakeSmtp {
    fn config(&self) -> SmtpConfig {
        SmtpConfig {
            host: "127.0.0.1".to_string(),
            port: self.port,
            security: Security::Plaintext,
        }
    }

    fn captured(&self) -> std::sync::MutexGuard<'_, Captured> {
        self.captured.lock().expect("capture lock")
    }
}

async fn start_fake(script: Script) -> FakeSmtp {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind fake smtp");
    let port = listener.local_addr().expect("local addr").port();
    let captured = Arc::new(Mutex::new(Captured::default()));
    let sink = Arc::clone(&captured);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            {
                let mut captured = sink.lock().expect("capture lock");
                captured.connections += 1;
            }
            tokio::spawn(handle(stream, script, Arc::clone(&sink)));
        }
    });
    FakeSmtp { port, captured }
}

async fn handle(stream: TcpStream, script: Script, captured: Arc<Mutex<Captured>>) {
    let (read_half, mut write) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    if write.write_all(b"220 fake ESMTP ready\r\n").await.is_err() {
        return;
    }

    let mut in_data = false;
    let mut message: Vec<u8> = Vec::new();

    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }

        if in_data {
            if line == b".\r\n" {
                in_data = false;
                if script.data == DataReply::Drop {
                    return;
                }
                {
                    let mut captured = captured.lock().expect("capture lock");
                    captured.message = std::mem::take(&mut message);
                }
                if write.write_all(b"250 2.0.0 queued\r\n").await.is_err() {
                    return;
                }
            } else {
                message.extend_from_slice(&line);
            }
            continue;
        }

        let text = String::from_utf8_lossy(&line);
        let trimmed = text.trim_end_matches(['\r', '\n']);
        let upper = trimmed.to_ascii_uppercase();

        let reply = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            Some("250-fake\r\n250-AUTH PLAIN LOGIN XOAUTH2\r\n250 OK".to_string())
        } else if upper.starts_with("AUTH ") {
            {
                let mut captured = captured.lock().expect("capture lock");
                captured.auth_lines.push(trimmed.to_string());
            }
            // Challenge-based forms need an extra round trip before
            // the verdict.
            if upper == "AUTH PLAIN" {
                if write.write_all(b"334 \r\n").await.is_err() {
                    return;
                }
                let mut response = Vec::new();
                if reader.read_until(b'\n', &mut response).await.unwrap_or(0) == 0 {
                    return;
                }
            } else if upper == "AUTH LOGIN" {
                if write.write_all(b"334 VXNlcm5hbWU6\r\n").await.is_err() {
                    return;
                }
                let mut response = Vec::new();
                if reader.read_until(b'\n', &mut response).await.unwrap_or(0) == 0 {
                    return;
                }
                if write.write_all(b"334 UGFzc3dvcmQ6\r\n").await.is_err() {
                    return;
                }
                let mut response = Vec::new();
                if reader.read_until(b'\n', &mut response).await.unwrap_or(0) == 0 {
                    return;
                }
            }
            Some(match script.auth {
                AuthReply::Accept => "235 2.7.0 accepted".to_string(),
                AuthReply::Reject => "535 5.7.8 bad credentials".to_string(),
            })
        } else if upper.starts_with("MAIL FROM") {
            {
                let mut captured = captured.lock().expect("capture lock");
                captured.mail_from.push(trimmed.to_string());
            }
            Some(script.mail_from.to_string())
        } else if upper.starts_with("RCPT TO") {
            {
                let mut captured = captured.lock().expect("capture lock");
                captured.rcpt_to.push(trimmed.to_string());
            }
            Some(script.rcpt.to_string())
        } else if upper == "DATA" {
            in_data = true;
            Some("354 end with <CRLF>.<CRLF>".to_string())
        } else if upper == "QUIT" {
            let _ = write.write_all(b"221 2.0.0 bye\r\n").await;
            return;
        } else {
            Some("250 2.0.0 ok".to_string())
        };

        if let Some(reply) = reply {
            if write
                .write_all(format!("{reply}\r\n").as_bytes())
                .await
                .is_err()
            {
                return;
            }
        }
    }
}

fn password_auth() -> SmtpAuth {
    SmtpAuth::Password {
        username: "alice@example.com".to_string(),
        password: "secret".to_string(),
    }
}

fn xoauth2_auth() -> SmtpAuth {
    SmtpAuth::Xoauth2 {
        email: "alice@example.com".to_string(),
        access_token: "token-123".to_string(),
    }
}

fn outgoing() -> OutgoingEnvelope {
    OutgoingEnvelope {
        from: "alice@example.com".to_string(),
        to: vec!["bob@other.org".to_string()],
        cc: vec!["carol@other.org".to_string()],
        bcc: vec!["hidden@other.org".to_string()],
    }
}

#[test]
fn tls_mode_maps_each_security_level() {
    assert!(matches!(
        tls_mode(Security::Tls, "smtp.example.com"),
        Ok(Tls::Wrapper(_))
    ));
    assert!(matches!(
        tls_mode(Security::StartTls, "smtp.example.com"),
        Ok(Tls::Required(_))
    ));
    assert!(matches!(
        tls_mode(Security::Plaintext, "smtp.example.com"),
        Ok(Tls::None)
    ));
}

#[test]
fn password_auth_offers_plain_then_login() {
    assert_eq!(
        mechanisms(&password_auth()),
        vec![Mechanism::Plain, Mechanism::Login]
    );
}

#[test]
fn xoauth2_auth_offers_only_xoauth2() {
    assert_eq!(mechanisms(&xoauth2_auth()), vec![Mechanism::Xoauth2]);
}

#[test]
fn envelope_carries_to_cc_and_bcc() {
    let envelope = build_envelope(&outgoing()).expect("valid envelope");

    assert_eq!(
        envelope.from().map(ToString::to_string),
        Some("alice@example.com".to_string())
    );
    let recipients: Vec<String> = envelope.to().iter().map(ToString::to_string).collect();
    assert_eq!(
        recipients,
        vec!["bob@other.org", "carol@other.org", "hidden@other.org"]
    );
}

#[tokio::test]
async fn invalid_recipient_fails_before_any_connection() {
    let server = start_fake(Script::default()).await;
    let mut envelope = outgoing();
    envelope.to = vec!["not an address".to_string()];

    let error = send(&server.config(), &password_auth(), &envelope, b"wire\r\n")
        .await
        .expect_err("invalid address must fail");

    assert!(
        matches!(error, SendError::InvalidEnvelope { .. }),
        "{error:?}"
    );
    assert_eq!(server.captured().connections, 0);
}

#[tokio::test]
async fn send_delivers_wire_bytes_and_envelope() {
    let server = start_fake(Script::default()).await;
    let wire =
        b"From: Alice <alice@example.com>\r\nTo: bob@other.org\r\nSubject: Hi\r\n\r\nHello\r\n";

    send(&server.config(), &password_auth(), &outgoing(), wire)
        .await
        .expect("send succeeds");

    let captured = server.captured();
    assert!(
        captured.connections >= 2,
        "probe plus submission, got {}",
        captured.connections
    );
    assert!(
        captured
            .auth_lines
            .iter()
            .any(|line| line.starts_with("AUTH PLAIN")),
        "password auth must use PLAIN: {:?}",
        captured.auth_lines
    );
    assert_eq!(captured.message, wire);
    assert!(
        captured
            .mail_from
            .iter()
            .any(|line| line.contains("alice@example.com"))
    );
    for recipient in ["bob@other.org", "carol@other.org", "hidden@other.org"] {
        assert!(
            captured.rcpt_to.iter().any(|line| line.contains(recipient)),
            "missing RCPT for {recipient}: {:?}",
            captured.rcpt_to
        );
    }
}

#[tokio::test]
async fn send_dot_stuffs_leading_dots() {
    let server = start_fake(Script::default()).await;
    let wire = b"Subject: dots\r\n\r\n.hidden\r\n..two\r\n";

    send(&server.config(), &password_auth(), &outgoing(), wire)
        .await
        .expect("send succeeds");

    let captured = server.captured();
    let text = String::from_utf8_lossy(&captured.message);
    assert!(text.contains("\r\n..hidden\r\n"), "{text}");
    assert!(text.contains("\r\n...two\r\n"), "{text}");
}

#[tokio::test]
async fn xoauth2_send_authenticates_and_delivers() {
    let server = start_fake(Script::default()).await;

    send(&server.config(), &xoauth2_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect("xoauth2 send succeeds");

    let captured = server.captured();
    let line = captured
        .auth_lines
        .iter()
        .find(|line| line.starts_with("AUTH XOAUTH2 "))
        .expect("XOAUTH2 mechanism used");
    let payload = line["AUTH XOAUTH2 ".len()..].trim();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .expect("base64 payload");
    assert_eq!(
        String::from_utf8(decoded).expect("utf-8 payload"),
        "user=alice@example.com\u{1}auth=Bearer token-123\u{1}\u{1}"
    );
    assert_eq!(captured.message, b"wire\r\n");
}

#[tokio::test]
async fn auth_rejection_is_pre_data() {
    let server = start_fake(Script {
        auth: AuthReply::Reject,
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("auth must fail");

    assert!(error.retry_safe(), "{error:?}");
    assert!(!error.needs_sent_check(), "{error:?}");
    match &error {
        SendError::PreData { stage, .. } => assert_eq!(*stage, "auth"),
        other => panic!("expected PreData, got {other:?}"),
    }
    assert!(
        server.captured().mail_from.is_empty(),
        "no mail transaction after an auth failure"
    );
}

#[tokio::test]
async fn mail_from_451_is_transient_and_retry_safe() {
    let server = start_fake(Script {
        mail_from: "451 4.3.0 try later",
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("4xx must fail");

    assert!(error.retry_safe(), "{error:?}");
    assert!(!error.needs_sent_check(), "{error:?}");
    match &error {
        SendError::Transient { code, .. } => assert_eq!(*code, 451),
        other => panic!("expected Transient, got {other:?}"),
    }
}

#[tokio::test]
async fn rcpt_550_is_rejected() {
    let server = start_fake(Script {
        rcpt: "550 5.1.1 no such user",
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("5xx must fail");

    assert!(!error.retry_safe(), "{error:?}");
    assert!(!error.needs_sent_check(), "{error:?}");
    match &error {
        SendError::Rejected { code, .. } => assert_eq!(*code, 550),
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[tokio::test]
async fn connection_drop_during_data_is_ambiguous() {
    let server = start_fake(Script {
        data: DataReply::Drop,
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("dropped DATA must fail");

    assert!(error.needs_sent_check(), "{error:?}");
    assert!(!error.retry_safe(), "{error:?}");
    match &error {
        SendError::Ambiguous { .. } => {}
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

#[tokio::test]
async fn connect_refused_is_pre_data() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind temp listener");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);

    let config = SmtpConfig {
        host: "127.0.0.1".to_string(),
        port,
        security: Security::Plaintext,
    };

    let error = send(&config, &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("connect must fail");

    assert!(error.retry_safe(), "{error:?}");
    match &error {
        SendError::PreData { stage, .. } => assert_eq!(*stage, "connect"),
        other => panic!("expected PreData, got {other:?}"),
    }
}
