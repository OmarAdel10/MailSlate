//! Connection probes for the account wizard's Test connection step
//! (USER_FLOW.md §3).
//!
//! The IMAP probe walks the exact connect + login path the sync engine
//! uses, so a green probe guarantees the first sync can authenticate.
//! The SMTP probe is a reachability check only (greeting + `EHLO` +
//! `QUIT`): real submission — including STARTTLS negotiation and SMTP
//! auth — lands with M4 compose-and-send.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::connect::{Conn, connect, login_password, server_name};
use super::tls::tls_connector;
use crate::domain::accounts::Security;
use crate::infra::imap::ImapError;

/// Probe failures, shared by the IMAP and SMTP probes.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// The TCP connection could not be established.
    #[error("connect failed: {0}")]
    Connect(String),
    /// The TLS handshake failed.
    #[error("tls handshake failed: {0}")]
    Tls(String),
    /// The server rejected the credentials (IMAP probe).
    #[error("authentication failed: {0}")]
    Auth(String),
    /// The server greeting was missing or invalid (SMTP probe).
    #[error("bad server greeting: {0}")]
    Greeting(String),
    /// The server did not accept `EHLO` (SMTP probe).
    #[error("EHLO rejected: {0}")]
    Ehlo(String),
    /// Authentication worked but mailbox discovery failed: the account
    /// cannot sync even though the login is valid (PRD F1.4).
    #[error("mailbox list failed: {0}")]
    List(String),
}

impl From<ImapError> for ProbeError {
    fn from(err: ImapError) -> Self {
        match err {
            ImapError::Connect(detail) => ProbeError::Connect(detail),
            ImapError::Tls(detail) => ProbeError::Tls(detail),
            ImapError::Auth(detail) => ProbeError::Auth(detail),
            ImapError::Protocol(detail) => ProbeError::Greeting(detail),
            // A probe never writes to the database; the catch-all keeps
            // the conversion complete if the error set grows.
            other => ProbeError::Connect(other.to_string()),
        }
    }
}

/// How long a single probe waits for the server before giving up.
///
/// The daemon wraps every probe in this budget; a server that swallows
/// bytes must not hold the wizard's Test connection step hostage.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Proves an IMAP account works: connect with the account's transport
/// security, authenticate, and enumerate mailboxes. Runs the same code
/// path the sync engine will use on the first sync, and covers the
/// `login + mailbox list` half of PRD F1.4.
pub async fn probe_imap_login(
    host: &str,
    port: u16,
    security: Security,
    username: &str,
    password: &str,
) -> Result<(), ProbeError> {
    tokio::time::timeout(PROBE_TIMEOUT, async {
        let client = connect(host, port, security).await?;
        let mut session = login_password(client, username, password).await?;
        let mailboxes =
            super::mailbox::list_mailboxes(&mut session)
                .await
                .map_err(|err| match err {
                    ImapError::Protocol(detail) => ProbeError::List(detail),
                    other => other.into(),
                })?;
        // A server that refuses LIST may close the command with `NO`,
        // which async-imap reports as an empty stream. Either way an
        // account with no mailbox is unusable (PRD F1.4).
        if mailboxes.is_empty() {
            return Err(ProbeError::List("the server listed no mailboxes".into()));
        }
        Ok(())
    })
    .await
    .map_err(|_| ProbeError::Connect("timed out".into()))?
}

/// Proves an SMTP endpoint is reachable and speaks SMTP: connect (with
/// implicit TLS when the account asks for it), read the `220` greeting,
/// exchange `EHLO`, and `QUIT`.
///
/// STARTTLS upgrade and SMTP auth are out of scope for the probe; M4
/// owns real submission.
pub async fn probe_smtp(host: &str, port: u16, security: Security) -> Result<(), ProbeError> {
    tokio::time::timeout(PROBE_TIMEOUT, async {
        let tcp = TcpStream::connect((host, port))
            .await
            .map_err(|err| ProbeError::Connect(err.to_string()))?;
        let mut stream = match security {
            Security::Tls => {
                let tls = tls_connector()
                    .connect(server_name(host)?, tcp)
                    .await
                    .map_err(|err| ProbeError::Tls(err.to_string()))?;
                Conn::Tls(Box::new(tls))
            }
            Security::StartTls | Security::Plaintext => Conn::Plain(Box::new(tcp)),
        };
        let mut scratch = Vec::new();
        let greeting = read_reply_line(&mut stream, &mut scratch).await?;
        if !greeting.starts_with("220") {
            return Err(ProbeError::Greeting(format!("got: {greeting}")));
        }
        stream
            .write_all(b"EHLO mailslate.local\r\n")
            .await
            .map_err(|err| ProbeError::Connect(err.to_string()))?;
        // Multi-line replies continue with `250-` until the final `250 `.
        loop {
            let line = read_reply_line(&mut stream, &mut scratch).await?;
            if !line.starts_with("250") {
                return Err(ProbeError::Ehlo(format!("got: {line}")));
            }
            if !line.starts_with("250-") {
                break;
            }
        }
        let _ = stream.write_all(b"QUIT\r\n").await;
        Ok(())
    })
    .await
    .map_err(|_| ProbeError::Connect("timed out".into()))?
}

/// Reads one `\r\n`-terminated reply line from the stream.
///
/// `buf` carries leftover bytes across calls: one socket read can hold
/// several reply lines, and dropping them would stall the next read.
async fn read_reply_line(stream: &mut Conn, buf: &mut Vec<u8>) -> Result<String, ProbeError> {
    let mut chunk = [0u8; 256];
    loop {
        if let Some(pos) = buf.windows(2).position(|window| window == b"\r\n") {
            let line = String::from_utf8_lossy(&buf[..pos]).trim().to_string();
            buf.drain(..pos + 2);
            return Ok(line);
        }
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|err| ProbeError::Connect(err.to_string()))?;
        if read == 0 {
            return Err(ProbeError::Connect(
                "connection closed before a full line".into(),
            ));
        }
        buf.extend_from_slice(&chunk[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::imap::testing::spawn_fake;

    /// Accepts one SMTP conversation: greeting, EHLO, QUIT.
    async fn spawn_fake_smtp(greeting: &'static str) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake smtp bind");
        let port = listener.local_addr().expect("local addr").port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("smtp accept");
            use tokio::io::{AsyncBufReadExt, BufReader};
            let (reader, mut writer) = tokio::io::split(stream);
            writer
                .write_all(format!("{greeting}\r\n").as_bytes())
                .await
                .expect("smtp greeting");
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.starts_with("EHLO") {
                    writer.write_all(b"250-fake.local\r\n250 SIZE 42\r\n").await
                } else {
                    writer.write_all(b"221 bye\r\n").await
                }
                .expect("smtp reply");
            }
        });
        port
    }

    #[tokio::test]
    async fn imap_probe_accepts_working_credentials() {
        let port = spawn_fake(|tag, command| {
            if command.starts_with("LOGIN") {
                vec![format!("{tag} OK logged in")]
            } else if command.starts_with("LIST") {
                vec![
                    "* LIST (\\HasNoChildren) \"/\" INBOX".to_string(),
                    format!("{tag} OK list done"),
                ]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        probe_imap_login("127.0.0.1", port, Security::Plaintext, "user", "pass")
            .await
            .expect("probe must succeed");
    }

    #[tokio::test]
    async fn imap_probe_lists_mailboxes_after_login() {
        let listed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&listed);
        let port = spawn_fake(move |tag, command| {
            if command.starts_with("LOGIN") {
                vec![format!("{tag} OK logged in")]
            } else if command.starts_with("LIST") {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                vec![
                    "* LIST (\\HasNoChildren) \"/\" INBOX".to_string(),
                    format!("{tag} OK list done"),
                ]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        probe_imap_login("127.0.0.1", port, Security::Plaintext, "user", "pass")
            .await
            .expect("probe must succeed");
        assert!(
            listed.load(std::sync::atomic::Ordering::SeqCst),
            "PRD F1.4 requires the probe to enumerate mailboxes"
        );
    }

    #[tokio::test]
    async fn imap_probe_reports_a_failing_mailbox_list() {
        let port = spawn_fake(|tag, command| {
            if command.starts_with("LOGIN") {
                vec![format!("{tag} OK logged in")]
            } else if command.starts_with("LIST") {
                vec![format!("{tag} NO [CANNOT] list denied")]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        let err = probe_imap_login("127.0.0.1", port, Security::Plaintext, "user", "pass")
            .await
            .expect_err("a server that cannot LIST must fail the probe");
        assert!(matches!(err, ProbeError::List(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn imap_probe_reports_rejected_credentials() {
        let port = spawn_fake(|tag, command| {
            if command.starts_with("LOGIN") {
                vec![format!("{tag} NO bad credentials")]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        let err = probe_imap_login("127.0.0.1", port, Security::Plaintext, "user", "wrong")
            .await
            .expect_err("bad credentials must fail");
        assert!(matches!(err, ProbeError::Auth(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn imap_probe_reports_unreachable_hosts() {
        let err = probe_imap_login("127.0.0.1", 1, Security::Plaintext, "user", "pass")
            .await
            .expect_err("port 1 must refuse");
        assert!(matches!(err, ProbeError::Connect(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn smtp_probe_accepts_a_speaking_server() {
        let port = spawn_fake_smtp("220 fake.local ESMTP").await;

        probe_smtp("127.0.0.1", port, Security::Plaintext)
            .await
            .expect("probe must succeed");
    }

    #[tokio::test]
    async fn smtp_probe_rejects_a_bad_greeting() {
        let port = spawn_fake_smtp("554 go away").await;

        let err = probe_smtp("127.0.0.1", port, Security::Plaintext)
            .await
            .expect_err("bad greeting must fail");
        assert!(matches!(err, ProbeError::Greeting(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn smtp_probe_reports_unreachable_hosts() {
        let err = probe_smtp("127.0.0.1", 1, Security::Plaintext)
            .await
            .expect_err("port 1 must refuse");
        assert!(matches!(err, ProbeError::Connect(_)), "got {err:?}");
    }
}
