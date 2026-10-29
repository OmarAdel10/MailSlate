//! Unit tests for the SMTP submission adapter.
//!
//! Every test runs against an in-process fake SMTP server, so the
//! protocol conversation, the envelope, and the failure taxonomy are
//! observable without a network or a real MTA.
//!
//! Accepted deferrals (recorded here, not testable cheaply):
//! - the `connect timeout` pre-data stage (mod.rs `pre_data_stage`):
//!   needs a black-hole socket held for [`SEND_TIMEOUT`];
//! - the pooled connection's idle close at [`POOL_IDLE_TIMEOUT`]:
//!   lettre-owned pool behavior at a 30 s idle window;
//! - `pre_data_stage`'s `has_rustls_source` arm against a real
//!   lettre error: lettre's `smtp::Error` is not constructible
//!   outside the lettre crate, so [`error_contains_rustls`] is
//!   pinned directly and the TLS stage resolution rides the
//!   implicit-TLS test instead.

use super::*;
use crate::domain::accounts::Security;
use base64::Engine as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const OK_MAIL: &str = "250 2.1.0 sender ok";
const OK_RCPT: &str = "250 2.1.5 recipient ok";

/// Longer than [`DETAIL_LIMIT`], so the adapter must truncate it.
const LONG_REPLY: &str = "550 5.7.1 this reply is intentionally much longer than the two hundred character detail budget so the adapter has to truncate it before the text can reach the outbox ui or the logs and it keeps going to make sure the limit is actually crossed by a comfortable margin xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthReply {
    Accept,
    Reject,
    /// A custom `4xx` refusal reply (e.g. `454 4.7.0 try later`).
    Transient(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DataReply {
    Accept,
    Reply(&'static str),
}

#[derive(Clone)]
struct Script {
    auth: AuthReply,
    /// Shared counter: the first N AUTH attempts across ALL connections
    /// answer with [`Script::auth`]'s verdict; every later attempt
    /// accepts, so a retry after a transient refusal succeeds.
    auth_transients_before_accept: Arc<AtomicUsize>,
    auth_mechanisms: &'static str,
    mail_from: &'static str,
    rcpt: &'static str,
    rcpt_fail_on: Option<usize>,
    data: DataReply,
    data_drop_on: Option<usize>,
    /// Never answer the DATA terminator: the client read pends until
    /// the adapter's total deadline fires.
    data_stall: bool,
    noop: &'static str,
    close_after_noop: Option<usize>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            auth: AuthReply::Accept,
            auth_transients_before_accept: Arc::new(AtomicUsize::new(0)),
            auth_mechanisms: "PLAIN LOGIN XOAUTH2",
            mail_from: OK_MAIL,
            rcpt: OK_RCPT,
            rcpt_fail_on: None,
            data: DataReply::Accept,
            data_drop_on: None,
            data_stall: false,
            noop: "250 2.0.0 ok",
            close_after_noop: None,
        }
    }
}

#[derive(Default, Debug)]
struct Captured {
    connections: usize,
    data_attempts: usize,
    auth_lines: Vec<String>,
    auth_responses: Vec<String>,
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
            tokio::spawn(handle(stream, script.clone(), Arc::clone(&sink)));
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
    let mut rcpt_index = 0usize;
    let mut noop_index = 0usize;

    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }

        if in_data {
            if line == b".\r\n" {
                in_data = false;
                let attempt = {
                    let mut captured = captured.lock().expect("capture lock");
                    captured.data_attempts += 1;
                    captured.data_attempts
                };
                if script.data_drop_on == Some(attempt) {
                    return;
                }
                if script.data_stall {
                    // Never answer: the client read pends until the
                    // adapter's total deadline fires.
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    return;
                }
                {
                    let mut captured = captured.lock().expect("capture lock");
                    captured.message = std::mem::take(&mut message);
                }
                let reply = match script.data {
                    DataReply::Accept => "250 2.0.0 queued".to_string(),
                    DataReply::Reply(reply) => reply.to_string(),
                };
                if write
                    .write_all(format!("{reply}\r\n").as_bytes())
                    .await
                    .is_err()
                {
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
            Some(format!(
                "250-fake\r\n250-AUTH {}\r\n250 OK",
                script.auth_mechanisms
            ))
        } else if upper.starts_with("AUTH ") {
            {
                let mut captured = captured.lock().expect("capture lock");
                captured.auth_lines.push(trimmed.to_string());
            }
            // Challenge-based forms need an extra round trip before
            // the verdict; capture what the client answered.
            let challenges: usize = if upper == "AUTH PLAIN" {
                1
            } else if upper == "AUTH LOGIN" {
                2
            } else {
                0
            };
            for challenge in 0..challenges {
                let prompt = if challenge == 0 && challenges == 1 {
                    "334 \r\n"
                } else if challenge == 0 {
                    "334 VXNlcm5hbWU6\r\n"
                } else {
                    "334 UGFzc3dvcmQ6\r\n"
                };
                if write.write_all(prompt.as_bytes()).await.is_err() {
                    return;
                }
                let mut response = Vec::new();
                if reader.read_until(b'\n', &mut response).await.unwrap_or(0) == 0 {
                    return;
                }
                let mut captured = captured.lock().expect("capture lock");
                captured
                    .auth_responses
                    .push(String::from_utf8_lossy(&response).trim().to_string());
            }
            let attempt = script
                .auth_transients_before_accept
                .fetch_add(1, Ordering::Relaxed);
            let verdict = if attempt == 0 {
                script.auth
            } else {
                AuthReply::Accept
            };
            Some(match verdict {
                AuthReply::Accept => "235 2.7.0 accepted".to_string(),
                AuthReply::Reject => "535 5.7.8 bad credentials".to_string(),
                AuthReply::Transient(reply) => reply.to_string(),
            })
        } else if upper.starts_with("MAIL FROM") {
            {
                let mut captured = captured.lock().expect("capture lock");
                captured.mail_from.push(trimmed.to_string());
            }
            Some(script.mail_from.to_string())
        } else if upper.starts_with("RCPT TO") {
            rcpt_index += 1;
            {
                let mut captured = captured.lock().expect("capture lock");
                captured.rcpt_to.push(trimmed.to_string());
            }
            if script.rcpt_fail_on == Some(rcpt_index) {
                Some("550 5.1.1 no such user".to_string())
            } else {
                Some(script.rcpt.to_string())
            }
        } else if upper == "DATA" {
            in_data = true;
            Some("354 end with <CRLF>.<CRLF>".to_string())
        } else if upper == "NOOP" {
            noop_index += 1;
            let reply = script.noop.to_string();
            if script.close_after_noop == Some(noop_index) {
                let _ = write.write_all(format!("{reply}\r\n").as_bytes()).await;
                return;
            }
            Some(reply)
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

fn decode_base64(value: &str) -> String {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .expect("base64 payload");
    String::from_utf8(bytes).expect("utf-8 payload")
}

/// Lets spawned pool-recycle tasks park their connections. A pooled
/// connection is recycled by a spawned task on drop, so a test that
/// asserts reuse must yield before the next send.
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

#[test]
fn tls_mode_maps_each_security_level() {
    match tls_mode(Security::Tls, "smtp.example.com").expect("wrapper") {
        Tls::Wrapper(parameters) => assert_eq!(parameters.domain(), "smtp.example.com"),
        _ => panic!("expected Tls::Wrapper"),
    }
    match tls_mode(Security::StartTls, "smtp.example.com").expect("starttls") {
        Tls::Required(parameters) => assert_eq!(parameters.domain(), "smtp.example.com"),
        _ => panic!("expected Tls::Required"),
    }
    assert!(matches!(
        tls_mode(Security::Plaintext, "127.0.0.1"),
        Ok(Tls::None)
    ));
    assert!(matches!(
        tls_mode(Security::Plaintext, "localhost"),
        Ok(Tls::None)
    ));
    assert!(matches!(
        tls_mode(Security::Plaintext, "192.168.1.10"),
        Ok(Tls::None)
    ));
    // Link-local and unique-local IPv6 are trusted with the same
    // verdict as their IPv4 counterparts.
    assert!(matches!(
        tls_mode(Security::Plaintext, "fe80::1"),
        Ok(Tls::None)
    ));
    assert!(matches!(
        tls_mode(Security::Plaintext, "::1"),
        Ok(Tls::None)
    ));
    assert!(matches!(
        tls_mode(Security::Plaintext, "fd00::1"),
        Ok(Tls::None)
    ));
    assert!(matches!(
        tls_mode(Security::Plaintext, "169.254.1.1"),
        Ok(Tls::None)
    ));
    // An unparseable host is not a local address: plaintext is
    // refused even when the port happens to belong to a test relay.
    assert!(matches!(
        tls_mode(Security::Plaintext, "smtp.example.invalid"),
        Err(SendError::InsecureTransport { .. })
    ));
    assert!(matches!(
        tls_mode(Security::Plaintext, "smtp.example.com"),
        Err(SendError::InsecureTransport { .. })
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
async fn envelope_rejects_invalid_sender_cc_bcc_and_empty_recipients() {
    let server = start_fake(Script::default()).await;
    let config = server.config();
    let auth = password_auth();

    let mut invalid_from = outgoing();
    invalid_from.from = "not an address".to_string();
    assert!(matches!(
        send(&config, &auth, &invalid_from, b"wire\r\n").await,
        Err(SendError::InvalidEnvelope { .. })
    ));

    let mut invalid_cc = outgoing();
    invalid_cc.cc = vec!["not an address".to_string()];
    assert!(matches!(
        send(&config, &auth, &invalid_cc, b"wire\r\n").await,
        Err(SendError::InvalidEnvelope { .. })
    ));

    let mut invalid_bcc = outgoing();
    invalid_bcc.bcc = vec!["not an address".to_string()];
    assert!(matches!(
        send(&config, &auth, &invalid_bcc, b"wire\r\n").await,
        Err(SendError::InvalidEnvelope { .. })
    ));

    let mut empty = outgoing();
    empty.to.clear();
    empty.cc.clear();
    empty.bcc.clear();
    assert!(matches!(
        send(&config, &auth, &empty, b"wire\r\n").await,
        Err(SendError::InvalidEnvelope { .. })
    ));

    assert_eq!(
        server.captured().connections,
        0,
        "envelope errors must not open a connection"
    );
}

#[test]
fn invalid_envelope_is_not_retry_safe() {
    let error = SendError::InvalidEnvelope {
        detail: "bad".to_string(),
    };

    assert!(!error.retry_safe());
    assert!(!error.needs_sent_check());
}

#[test]
fn send_error_display_strings() {
    assert_eq!(
        SendError::InvalidEnvelope {
            detail: "bad".to_string()
        }
        .to_string(),
        "invalid envelope: bad"
    );
    assert_eq!(
        SendError::InsecureTransport {
            detail: "smtp.example.com".to_string()
        }
        .to_string(),
        "refusing plaintext SMTP to a non-local host: smtp.example.com"
    );
    assert_eq!(
        SendError::PreData {
            stage: "tls",
            detail: "boom".to_string()
        }
        .to_string(),
        "send failed before DATA (tls): boom"
    );
    assert_eq!(
        SendError::Transient {
            code: 451,
            detail: "later".to_string()
        }
        .to_string(),
        "server transiently refused (451): later"
    );
    assert_eq!(
        SendError::Rejected {
            code: 550,
            detail: "no".to_string()
        }
        .to_string(),
        "server permanently refused (550): no"
    );
    assert_eq!(
        SendError::Ambiguous {
            detail: "unknown".to_string()
        }
        .to_string(),
        "ambiguous send outcome: unknown"
    );
}

#[tokio::test]
async fn plaintext_to_a_remote_host_is_refused_before_connecting() {
    let server = start_fake(Script::default()).await;
    let config = SmtpConfig {
        host: "smtp.example.com".to_string(),
        port: server.port,
        security: Security::Plaintext,
    };

    let error = send(&config, &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("plaintext must be refused");

    assert!(
        matches!(&error, SendError::InsecureTransport { .. }),
        "{error:?}"
    );
    assert!(!error.retry_safe());
    assert!(!error.needs_sent_check());
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
        captured.connections <= 2,
        "probe and submission use at most two connections, got {}",
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
async fn password_plain_payload_carries_username_and_password() {
    let server = start_fake(Script::default()).await;

    send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect("send succeeds");

    let captured = server.captured();
    let line = captured
        .auth_lines
        .iter()
        .find(|line| line.starts_with("AUTH PLAIN "))
        .expect("PLAIN mechanism used");
    let payload = line["AUTH PLAIN ".len()..].trim();
    assert_eq!(
        decode_base64(payload),
        "\0alice@example.com\0secret",
        "PLAIN must carry the configured username and password in order"
    );
}

#[tokio::test]
async fn login_fallback_when_plain_is_not_advertised() {
    let server = start_fake(Script {
        auth_mechanisms: "LOGIN",
        ..Script::default()
    })
    .await;

    send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect("LOGIN send succeeds");

    let captured = server.captured();
    assert!(
        captured.auth_lines.iter().any(|line| line == "AUTH LOGIN"),
        "LOGIN must be used when PLAIN is not offered: {:?}",
        captured.auth_lines
    );
    assert!(
        !captured.auth_responses.is_empty() && captured.auth_responses.len() % 2 == 0,
        "LOGIN sends a username/password pair per connection: {:?}",
        captured.auth_responses
    );
    for pair in captured.auth_responses.chunks(2) {
        assert_eq!(decode_base64(&pair[0]), "alice@example.com");
        assert_eq!(decode_base64(&pair[1]), "secret");
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
async fn wire_without_trailing_crlf_delivers_the_same_bytes() {
    let server = start_fake(Script::default()).await;

    send(
        &server.config(),
        &password_auth(),
        &outgoing(),
        b"Subject: x\r\n\r\nbody",
    )
    .await
    .expect("send succeeds");

    // lettre's terminator restores the missing CRLF, so the server
    // still receives one complete message.
    assert_eq!(server.captured().message, b"Subject: x\r\n\r\nbody\r\n");
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
    assert_eq!(
        decode_base64(payload),
        "user=alice@example.com\u{1}auth=Bearer token-123\u{1}\u{1}"
    );
    assert_eq!(captured.message, b"wire\r\n");
}

#[tokio::test]
async fn auth_rejection_is_permanent() {
    let server = start_fake(Script {
        auth: AuthReply::Reject,
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("auth must fail");

    assert!(!error.retry_safe(), "{error:?}");
    assert!(!error.needs_sent_check(), "{error:?}");
    match &error {
        SendError::Rejected { code, .. } => assert_eq!(*code, 535),
        other => panic!("expected Rejected, got {other:?}"),
    }
    assert!(
        server.captured().mail_from.is_empty(),
        "no mail transaction after an auth failure"
    );
}

#[tokio::test]
async fn noop_rejected_by_probe_is_pre_data_greeting() {
    // The probe is lettre's `test_connection()`, which answers a
    // boolean: it collapses every NOOP refusal (4xx or 5xx) into
    // `Ok(false)` with no visible status, so the refusal surfaces as
    // the retry-safe `greeting` stage. That is dup-safe by
    // construction — the message was never offered.
    let server = start_fake(Script {
        noop: "421 4.3.2 shutting down",
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("NOOP refusal must fail the probe");

    assert!(error.retry_safe(), "{error:?}");
    match &error {
        SendError::PreData { stage, .. } => assert_eq!(*stage, "greeting"),
        other => panic!("expected PreData, got {other:?}"),
    }
    assert!(server.captured().mail_from.is_empty());
}

#[tokio::test]
async fn starttls_refusal_never_falls_back_to_cleartext() {
    let server = start_fake(Script::default()).await;
    let config = SmtpConfig {
        host: "127.0.0.1".to_string(),
        port: server.port,
        security: Security::StartTls,
    };

    let error = send(&config, &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("STARTTLS is required, the fake does not offer it");

    assert!(error.retry_safe(), "{error:?}");
    assert!(!error.needs_sent_check(), "{error:?}");
    assert!(matches!(&error, SendError::PreData { .. }), "{error:?}");
    let captured = server.captured();
    assert!(
        captured.mail_from.is_empty() && captured.message.is_empty(),
        "no transaction may run in clear text: {captured:?}"
    );
}

#[tokio::test]
async fn implicit_tls_against_plaintext_server_is_pre_data_tls() {
    let server = start_fake(Script::default()).await;
    let config = SmtpConfig {
        host: "127.0.0.1".to_string(),
        port: server.port,
        security: Security::Tls,
    };

    let error = send(&config, &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("the plaintext fake cannot complete a TLS handshake");

    match &error {
        SendError::PreData { stage, .. } => assert_eq!(*stage, "tls", "{error:?}"),
        other => panic!("expected PreData, got {other:?}"),
    }
    assert!(server.captured().mail_from.is_empty());
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
async fn partial_rcpt_rejection_aborts_the_transaction() {
    let server = start_fake(Script {
        rcpt_fail_on: Some(3),
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("the third recipient is refused");

    match &error {
        SendError::Rejected { code, .. } => assert_eq!(*code, 550),
        other => panic!("expected Rejected, got {other:?}"),
    }
    let captured = server.captured();
    assert_eq!(captured.rcpt_to.len(), 3);
    assert!(captured.message.is_empty(), "DATA must not start");
}

#[tokio::test]
async fn data_554_is_rejected_after_the_message_was_offered() {
    let server = start_fake(Script {
        data: DataReply::Reply("554 5.6.0 content rejected"),
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("post-DATA 5xx must fail");

    assert!(!error.retry_safe(), "{error:?}");
    assert!(!error.needs_sent_check(), "{error:?}");
    match &error {
        SendError::Rejected { code, .. } => assert_eq!(*code, 554),
        other => panic!("expected Rejected, got {other:?}"),
    }
    assert!(
        !server.captured().message.is_empty(),
        "the message was offered before the refusal"
    );
}

#[tokio::test]
async fn data_451_is_transient() {
    let server = start_fake(Script {
        data: DataReply::Reply("451 4.3.0 try later"),
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("post-DATA 4xx must fail");

    assert!(error.retry_safe(), "{error:?}");
    match &error {
        SendError::Transient { code, .. } => assert_eq!(*code, 451),
        other => panic!("expected Transient, got {other:?}"),
    }
}

#[tokio::test]
async fn server_detail_is_truncated_to_the_first_line() {
    let server = start_fake(Script {
        rcpt: LONG_REPLY,
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("long refusal must fail");

    match &error {
        SendError::Rejected { detail, .. } => {
            assert!(
                detail.ends_with('…'),
                "detail must be visibly truncated: {detail}"
            );
            assert_eq!(detail.chars().count(), DETAIL_LIMIT + 1);
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[tokio::test]
async fn connection_drop_during_data_is_ambiguous() {
    let server = start_fake(Script {
        data_drop_on: Some(1),
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
async fn connection_drop_on_reused_connection_is_ambiguous() {
    let server = start_fake(Script {
        data_drop_on: Some(2),
        ..Script::default()
    })
    .await;
    let sender = SmtpSender::new(&server.config(), &password_auth()).expect("build sender");

    sender.send(&outgoing(), b"one\r\n").await.expect("first");
    settle().await;

    let error = sender
        .send(&outgoing(), b"two\r\n")
        .await
        .expect_err("the reused connection dies during DATA");

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

#[tokio::test]
async fn padded_addresses_are_trimmed() {
    let server = start_fake(Script::default()).await;
    let mut envelope = outgoing();
    envelope.to = vec![" bob@other.org ".to_string()];
    envelope.cc.clear();
    envelope.bcc.clear();

    send(&server.config(), &password_auth(), &envelope, b"wire\r\n")
        .await
        .expect("padded address is valid");

    assert_eq!(
        server.captured().rcpt_to,
        vec!["RCPT TO:<bob@other.org>".to_string()]
    );
}

#[tokio::test]
async fn bcc_only_envelope_delivers() {
    let server = start_fake(Script::default()).await;
    let envelope = OutgoingEnvelope {
        from: "alice@example.com".to_string(),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: vec!["hidden@other.org".to_string()],
    };

    send(&server.config(), &password_auth(), &envelope, b"wire\r\n")
        .await
        .expect("undisclosed recipients are valid");

    let captured = server.captured();
    assert_eq!(
        captured.rcpt_to,
        vec!["RCPT TO:<hidden@other.org>".to_string()]
    );
    assert_eq!(captured.message, b"wire\r\n");
}

#[tokio::test]
async fn pooled_sender_reuses_connections_for_a_burst() {
    let server = start_fake(Script::default()).await;
    let sender = SmtpSender::new(&server.config(), &password_auth()).expect("build sender");

    for message in [&b"one\r\n"[..], b"two\r\n", b"three\r\n"] {
        sender.send(&outgoing(), message).await.expect("send");
        settle().await;
    }

    let captured = server.captured();
    assert!(
        captured.connections <= 2,
        "three messages must reuse the pooled connections, opened {}",
        captured.connections
    );
    assert!(
        captured.auth_lines.len() <= 2,
        "a burst authenticates at most twice: {:?}",
        captured.auth_lines
    );
    assert_eq!(captured.message, b"three\r\n");
}

#[tokio::test]
async fn noop_5xx_refusal_is_pre_data_greeting() {
    // Even a permanent 5xx NOOP refusal is collapsed by lettre's
    // boolean probe into `Ok(false)`: no status reaches the error
    // chain, so the failure lands on the retry-safe greeting stage.
    // Dup-safe by construction — nothing was offered.
    let server = start_fake(Script {
        noop: "550 5.7.0 policy refused",
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("NOOP refusal must fail the probe");

    assert!(error.retry_safe(), "{error:?}");
    assert!(!error.needs_sent_check(), "{error:?}");
    match &error {
        SendError::PreData { stage, .. } => assert_eq!(*stage, "greeting"),
        other => panic!("expected PreData, got {other:?}"),
    }
    let captured = server.captured();
    assert!(captured.mail_from.is_empty());
    assert!(captured.message.is_empty());
}

#[tokio::test]
async fn transient_auth_refusal_is_pre_data_and_retry_safe() {
    let server = start_fake(Script {
        auth: AuthReply::Transient("454 4.7.0 try later"),
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("4xx auth must fail");

    assert!(error.retry_safe(), "{error:?}");
    assert!(!error.needs_sent_check(), "{error:?}");
    match &error {
        SendError::PreData { stage, .. } => assert_eq!(*stage, "pre-data"),
        other => panic!("expected PreData, got {other:?}"),
    }
    assert!(
        server.captured().mail_from.is_empty(),
        "no mail transaction after a transient auth failure"
    );
}

#[tokio::test]
async fn send_recovers_after_a_transient_auth_refusal() {
    // First send: probe OK, AUTH transient (454) -> PreData retry-safe.
    // Second send: fresh transport (no pool reuse across calls), probe OK,
    // AUTH accept -> success. The shared atomic counter ensures the
    // first AUTH across ALL connections is transient, the second is accept.
    let counter = Arc::new(AtomicUsize::new(0));
    let server = start_fake(Script {
        auth: AuthReply::Transient("454 4.7.0 try later"),
        auth_transients_before_accept: counter,
        ..Script::default()
    })
    .await;

    let first = send(&server.config(), &password_auth(), &outgoing(), b"one\r\n")
        .await
        .expect_err("first attempt is refused");
    assert!(first.retry_safe(), "{first:?}");
    match &first {
        SendError::PreData { stage, .. } => assert_eq!(*stage, "pre-data"),
        other => panic!("expected PreData, got {other:?}"),
    }
    assert!(
        server.captured().mail_from.is_empty(),
        "no mail transaction after transient auth failure"
    );

    let second = send(&server.config(), &password_auth(), &outgoing(), b"two\r\n")
        .await
        .expect("the retried send succeeds");
    assert_eq!(second, ());

    let captured = server.captured();
    assert_eq!(captured.message, b"two\r\n");
    assert!(
        captured
            .mail_from
            .iter()
            .any(|line| line.contains("alice@example.com")),
        "the retry must run a mail transaction: {:?}",
        captured.mail_from
    );
}

#[test]
fn error_contains_rustls_walks_the_source_chain() {
    #[derive(Debug)]
    struct WrapsRustlsInSource(rustls::Error);

    impl std::fmt::Display for WrapsRustlsInSource {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("wrapping rustls")
        }
    }

    impl std::error::Error for WrapsRustlsInSource {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    let rustls_error = rustls::Error::General("handshake failure".to_string());

    // A plain io error carries no rustls: false.
    let plain = std::io::Error::other("plain io failure");
    assert!(
        !error_contains_rustls(&plain),
        "plain io error must not be classified as rustls"
    );

    // Direct hit: true.
    let direct = rustls::Error::General("direct".to_string());
    assert!(error_contains_rustls(&direct));

    // An io error wrapping rustls: reached through `get_ref`.
    let wrapped = std::io::Error::other(rustls_error.clone());
    assert!(error_contains_rustls(&wrapped));

    // A deep chain: io wrapping io wrapping rustls, and a source()
    // chain ending in rustls.
    let deep = std::io::Error::other(wrapped);
    assert!(error_contains_rustls(&deep));
    assert!(error_contains_rustls(&WrapsRustlsInSource(rustls_error)));
}

#[tokio::test]
async fn non_ascii_addresses_surface_as_ambiguous_not_invalid_envelope() {
    // lettre does not pre-validate non-ASCII addresses; it attempts
    // to send and the client returns an internal error with no
    // server status. The adapter classifies this as Ambiguous, which
    // requires a Sent check but is not retry-safe — wrong outbox
    // policy. This is a known divergence from the ideal
    // InvalidEnvelope path; tracked for the compose slice.
    let server = start_fake(Script::default()).await;
    let config = server.config();
    let auth = password_auth();

    let mut bad_local_part = outgoing();
    bad_local_part.to = vec!["j\u{f6}rge@other.org".to_string()];
    bad_local_part.cc.clear();
    bad_local_part.bcc.clear();
    let error = send(&config, &auth, &bad_local_part, b"wire\r\n")
        .await
        .expect_err("non-ASCII local part must fail");
    assert!(
        matches!(error, SendError::Ambiguous { .. }),
        "non-ASCII currently yields Ambiguous (no server status): {error:?}"
    );
    assert!(error.needs_sent_check(), "{error:?}");
    assert!(!error.retry_safe(), "{error:?}");

    let mut bad_domain = outgoing();
    bad_domain.from = "alice@ex\u{e4}mple.test".to_string();
    let error = send(&config, &auth, &bad_domain, b"wire\r\n")
        .await
        .expect_err("non-ASCII domain must fail");
    assert!(
        matches!(error, SendError::Ambiguous { .. }),
        "non-ASCII currently yields Ambiguous (no server status): {error:?}"
    );
    assert!(error.needs_sent_check(), "{error:?}");
    assert!(!error.retry_safe(), "{error:?}");

    // Connections are opened because envelope validation passes;
    // the failure occurs at the lettre client level during send.
    // This is the documented divergence from ideal InvalidEnvelope.
    assert!(
        server.captured().connections > 0,
        "connections opened before client-level failure: {}",
        server.captured().connections
    );
}

#[tokio::test]
async fn multi_line_refusal_detail_is_joined_then_truncated() {
    // lettre joins all server reply lines with newlines, then the
    // adapter takes the first line of that joined string (which is
    // the first server line followed by embedded newlines from the
    // joined remainder). The effective "first line" visible to us
    // contains the start of the second server line.
    let server = start_fake(Script {
        rcpt: "550-5.1.1 first line\r\n550 5.1.1 second line",
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("multi-line refusal must fail");

    match &error {
        SendError::Rejected { detail, .. } => {
            // The joined response starts with "550-5.1.1 first line" then
            // the embedded "\r\n" from lettre's join, then "550 5.1.1 second line".
            // Our `error_detail` takes the first *character line* of that.
            assert!(
                detail.contains("first line"),
                "detail must contain the first server line: {detail}"
            );
            assert!(
                detail.contains("second line"),
                "joined response makes the second server line visible in the first line: {detail}"
            );
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[tokio::test]
async fn detail_of_exactly_the_limit_is_not_marked_truncated() {
    // lettre's Display prefixes the reply with `permanent error
    // (550): ` (23 chars); padding the reply so the whole first line
    // is exactly [`DETAIL_LIMIT`] chars must not gain an ellipsis.
    let first_line = Box::leak(
        format!(
            "550 5.7.1 {}",
            "x".repeat(DETAIL_LIMIT - "permanent error (550): ".len() - "550 5.7.1 ".len() + 4)
        )
        .into_boxed_str(),
    );
    let server = start_fake(Script {
        rcpt: first_line,
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("boundary refusal must fail");

    match &error {
        SendError::Rejected { detail, .. } => {
            assert_eq!(detail.chars().count(), DETAIL_LIMIT, "{detail}");
            assert!(
                !detail.ends_with('…'),
                "a line at the limit is not truncated: {detail}"
            );
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[tokio::test]
async fn mail_from_550_is_rejected() {
    let server = start_fake(Script {
        mail_from: "550 5.1.0 envelope blocked",
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &password_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("MAIL FROM 5xx must fail");

    assert!(!error.retry_safe(), "{error:?}");
    assert!(!error.needs_sent_check(), "{error:?}");
    match &error {
        SendError::Rejected { code, .. } => assert_eq!(*code, 550),
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[tokio::test]
async fn xoauth2_rejection_is_permanent() {
    let server = start_fake(Script {
        auth: AuthReply::Reject,
        ..Script::default()
    })
    .await;

    let error = send(&server.config(), &xoauth2_auth(), &outgoing(), b"wire\r\n")
        .await
        .expect_err("XOAUTH2 rejection must fail");

    assert!(!error.retry_safe(), "{error:?}");
    match &error {
        SendError::Rejected { code, .. } => assert_eq!(*code, 535),
        other => panic!("expected Rejected, got {other:?}"),
    }
    assert!(server.captured().mail_from.is_empty());
}

#[tokio::test]
async fn server_drop_after_noop_probe_recovers_on_a_fresh_connection() {
    // The probe connection dies right after answering its NOOP. The
    // pool's liveness check discards that dead parked connection on
    // the next acquire, so the submission still runs on a fresh,
    // working connection.
    let server = start_fake(Script {
        close_after_noop: Some(1),
        ..Script::default()
    })
    .await;
    let sender = SmtpSender::new(&server.config(), &password_auth()).expect("build sender");

    sender
        .send(&outgoing(), b"wire\r\n")
        .await
        .expect("the send must recover after the dropped probe connection");

    settle().await;
    let captured = server.captured();
    assert_eq!(captured.message, b"wire\r\n");
    assert!(
        captured.connections <= 2,
        "the dead probe connection must be replaced by at most one \
         fresh connection, opened {}",
        captured.connections
    );
}

#[tokio::test]
async fn total_send_deadline_elapses_as_ambiguous() {
    // The server accepts the probe and the DATA conversation but
    // never answers the terminator: the per-command budget is far
    // away, so only the total deadline can break the hang.
    let server = start_fake(Script {
        data_stall: true,
        ..Script::default()
    })
    .await;
    let mut sender = SmtpSender::new(&server.config(), &password_auth()).expect("build sender");
    sender.total_timeout = Duration::from_millis(400);

    let started = std::time::Instant::now();
    let error = sender
        .send(&outgoing(), b"wire\r\n")
        .await
        .expect_err("a stalled server must hit the total deadline");

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the total deadline, not the per-command budget, must fire"
    );
    assert!(error.needs_sent_check(), "{error:?}");
    assert!(!error.retry_safe(), "{error:?}");
    match &error {
        SendError::Ambiguous { detail } => assert_eq!(detail, "send deadline exceeded"),
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}
