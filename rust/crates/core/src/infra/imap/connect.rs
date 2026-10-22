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

pub(crate) fn server_name(host: &str) -> Result<ServerName<'static>, ImapError> {
    ServerName::try_from(host.to_string())
        .map_err(|err| ImapError::Tls(format!("invalid server name {host}: {err}")))
}
/// Reads and discards the server greeting. `Client::new` does not
/// consume it, and the crate docs make greeting consumption the
/// caller's job: a pending greeting poisons the SASL handshake
/// (`do_auth_handshake` routes the untagged greeting into
/// `check_done_ok_from`, which then swallows the `+` continuation and
/// waits for a tagged completion that never comes).
pub(crate) async fn read_greeting(client: &mut Client<Conn>) -> Result<(), ImapError> {
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
/// Classifies a login or authenticate failure: server rejections
/// (NO / BAD) are auth failures — the actor parks in reauth; every
/// other failure (connection reset, EOF, parse) is transient and the
/// supervision loop retries it with backoff.
fn login_error(err: async_imap::error::Error) -> ImapError {
    match &err {
        async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => {
            ImapError::Auth(err.to_string())
        }
        _ => ImapError::Connect(err.to_string()),
    }
}

pub(crate) async fn login_password(
    client: Client<Conn>,
    username: &str,
    password: &str,
) -> Result<Session<Conn>, ImapError> {
    match client.login(username, password).await {
        Ok(session) => Ok(session),
        Err((err, _)) => Err(login_error(err)),
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
        Err((err, _)) => Err(login_error(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::Security;
    use crate::infra::imap::testing::spawn_fake;

    #[tokio::test]
    async fn plaintext_connect_succeeds() {
        let port = spawn_fake(|_tag, _command| {
            vec![format!("{_tag} OK")]
        })
        .await;

        let client = connect("127.0.0.1", port, Security::Plaintext)
            .await
            .expect("plaintext connect over plain TCP works");
        let _session = login_password(client, "user", "pass").await.expect("login");
    }

    #[tokio::test]
    async fn plaintext_xoauth2_connect_succeeds() {
        let port = spawn_fake(|_tag, _command| {
            vec![format!("{_tag} OK")]
        })
        .await;

        let client = connect("127.0.0.1", port, Security::Plaintext)
            .await
            .expect("plaintext connect over plain TCP works");
        let _session = login_xoauth2(client, "me@gmail.com", "tok-1")
            .await
            .expect("xoauth2");
    }

    /// Hostnames that are not valid TLS server names fail early with
    /// a TLS classification, not a connect error.
    #[test]
    fn server_name_rejects_invalid_hostnames() {
        let err = server_name("not a host!").expect_err("spaces are not a valid name");
        assert!(matches!(err, ImapError::Tls(_)), "got {err:?}");
        assert!(
            err.to_string().contains("invalid server name"),
            "got {err:?}"
        );
    }
}
