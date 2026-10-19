//! Connecting and authenticating IMAP sessions.
//!
//! Security comes from [`crate::domain::accounts::Security`]: implicit
//! TLS, STARTTLS upgrade, or plaintext (tests and exotic servers only).
//!
//! The sync engine (phase 6) is the first in-crate consumer of these
//! entry points; until it lands the cluster is intentionally unused,
//! so dead-code is allowed module-wide.
#![allow(dead_code)]

use std::pin::Pin;
use std::task::{Context, Poll};

use async_imap::{Client, Session};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;

use super::auth::xoauth2_string;
use super::tls::tls_connector;
use crate::domain::accounts::Security;
use crate::infra::imap::ImapError;

/// A client stream over either plaintext TCP or TLS. async-imap is
/// generic over the stream; this enum is the one concrete type the
/// adapter hands it, so callers never see the transport.
#[derive(Debug)]
pub(crate) enum Conn {
    Plain(Box<TcpStream>),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for Conn {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Conn::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
            Conn::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut *self {
            Conn::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
            Conn::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Conn::Plain(stream) => Pin::new(stream).poll_flush(cx),
            Conn::Tls(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match &mut *self {
            Conn::Plain(stream) => Pin::new(stream).poll_shutdown(cx),
            Conn::Tls(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

async fn tcp(host: &str, port: u16) -> Result<TcpStream, ImapError> {
    TcpStream::connect((host, port))
        .await
        .map_err(|err| ImapError::Connect(err.to_string()))
}

fn server_name(host: &str) -> Result<ServerName<'static>, ImapError> {
    ServerName::try_from(host.to_string())
        .map_err(|err| ImapError::Tls(format!("invalid server name {host}: {err}")))
}

/// Reads and discards the server greeting. `Client::new` does not
/// consume it, and the crate docs make greeting consumption the
/// caller's job: a pending greeting poisons the SASL handshake
/// (`do_auth_handshake` routes the untagged greeting into
/// `check_done_ok_from`, which then swallows the `+` continuation and
/// waits for a tagged completion that never comes).
async fn read_greeting(client: &mut Client<Conn>) -> Result<(), ImapError> {
    match client.read_response().await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(ImapError::Connect(
            "connection closed before greeting".into(),
        )),
        Err(err) => Err(ImapError::Protocol(err.to_string())),
    }
}

/// Connects to `host:port` with the requested transport security and
/// returns an unauthenticated client. Callers then authenticate with
/// [`login_password`] or [`login_xoauth2`].
pub(crate) async fn connect(
    host: &str,
    port: u16,
    security: Security,
) -> Result<Client<Conn>, ImapError> {
    match security {
        Security::Tls => {
            let tcp = tcp(host, port).await?;
            let stream = tls_connector()
                .connect(server_name(host)?, tcp)
                .await
                .map_err(|err| ImapError::Tls(err.to_string()))?;
            let mut client = Client::new(Conn::Tls(Box::new(stream)));
            read_greeting(&mut client).await?;
            Ok(client)
        }
        Security::StartTls => {
            let tcp = tcp(host, port).await?;
            let mut client = Client::new(Conn::Plain(Box::new(tcp)));
            read_greeting(&mut client).await?;
            client
                .run_command_and_check_ok("STARTTLS", None)
                .await
                .map_err(|err| ImapError::Protocol(err.to_string()))?;
            let raw = match client.into_inner() {
                Conn::Plain(raw) => *raw,
                Conn::Tls(_) => unreachable!("plain connection cannot be tls before handshake"),
            };
            let stream = tls_connector()
                .connect(server_name(host)?, raw)
                .await
                .map_err(|err| ImapError::Tls(err.to_string()))?;
            Ok(Client::new(Conn::Tls(Box::new(stream))))
        }
        Security::Plaintext => {
            let tcp = tcp(host, port).await?;
            let mut client = Client::new(Conn::Plain(Box::new(tcp)));
            read_greeting(&mut client).await?;
            Ok(client)
        }
    }
}

/// Authenticates with user and password (LOGIN).
pub(crate) async fn login_password(
    client: Client<Conn>,
    username: &str,
    password: &str,
) -> Result<Session<Conn>, ImapError> {
    match client.login(username, password).await {
        Ok(session) => Ok(session),
        Err((err, _)) => Err(ImapError::Auth(err.to_string())),
    }
}

struct Xoauth2Auth {
    user: String,
    token: String,
}

impl async_imap::Authenticator for Xoauth2Auth {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> String {
        xoauth2_string(&self.user, &self.token)
    }
}

/// Authenticates with the XOAUTH2 SASL mechanism (OAuth2 accounts).
pub(crate) async fn login_xoauth2(
    client: Client<Conn>,
    username: &str,
    token: &str,
) -> Result<Session<Conn>, ImapError> {
    let auth = Xoauth2Auth {
        user: username.into(),
        token: token.into(),
    };
    match client.authenticate("XOAUTH2", auth).await {
        Ok(session) => Ok(session),
        Err((err, _)) => Err(ImapError::Auth(err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::Security;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// A minimal IMAP wire fake: greeting, then one canned answer per
    /// command line, chosen by `responder(tag, command)`.
    async fn spawn_fake(responder: impl Fn(&str, &str) -> Vec<String> + Send + 'static) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake imap bind");
        let port = listener.local_addr().expect("local addr").port();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let (reader, mut writer) = tokio::io::split(stream);
            writer
                .write_all(b"* OK fake imap ready\r\n")
                .await
                .expect("greeting");
            let mut last_tag = String::new();
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let (tag, command) = match line.split_once(' ') {
                    Some((tag, command)) => {
                        last_tag = tag.to_string();
                        (tag, command)
                    }
                    // SASL continuation answers arrive without a tag;
                    // they belong to the last tagged command.
                    None => (last_tag.as_str(), line.as_str()),
                };
                for reply in responder(tag, command) {
                    writer
                        .write_all(format!("{reply}\r\n").as_bytes())
                        .await
                        .expect("fake reply");
                }
            }
        });
        port
    }

    #[tokio::test]
    async fn connects_and_logs_in_with_a_password() {
        let port = spawn_fake(|tag, command| {
            if command.starts_with("LOGIN") {
                vec![format!("{tag} OK logged in")]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        let client = connect("127.0.0.1", port, Security::Plaintext)
            .await
            .expect("connect");
        let _session = login_password(client, "user", "pass").await.expect("login");
    }

    #[tokio::test]
    async fn connects_and_authenticates_with_xoauth2() {
        let port = spawn_fake(|tag, command| {
            if command.starts_with("AUTHENTICATE") {
                vec!["+ ".to_string()]
            } else {
                vec![format!("{tag} OK authenticated")]
            }
        })
        .await;

        let client = connect("127.0.0.1", port, Security::Plaintext)
            .await
            .expect("connect");
        let _session = login_xoauth2(client, "me@gmail.com", "tok-1")
            .await
            .expect("xoauth2");
    }

    #[tokio::test]
    async fn login_failure_surfaces_as_an_auth_error() {
        let port = spawn_fake(|tag, command| {
            if command.starts_with("LOGIN") {
                vec![format!("{tag} NO bad credentials")]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;

        let client = connect("127.0.0.1", port, Security::Plaintext)
            .await
            .expect("connect");
        let error = login_password(client, "user", "wrong")
            .await
            .expect_err("bad credentials must fail");
        assert!(
            matches!(error, crate::infra::imap::ImapError::Auth(_)),
            "got {error:?}"
        );
    }
}
