//! Test-only wire fakes shared by the IMAP adapter slices.
//!
//! Lives behind `#[cfg(test)]`; production code never links this module.

use super::connect::{Conn, connect, login_password};
use crate::domain::accounts::Security;
use async_imap::Session;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// A minimal IMAP wire fake: greeting, then one canned answer per
/// command line, chosen by `responder(tag, command)`. SASL continuation
/// answers arrive without a tag and attach to the last tagged command.
pub(crate) async fn spawn_fake(
    responder: impl Fn(&str, &str) -> Vec<String> + Send + 'static,
) -> u16 {
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
                // SASL continuation answers arrive without a tag; they
                // belong to the last tagged command.
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

/// A logged-in plaintext session against the wire fake: convenience for
/// slices that only care about post-authentication commands.
pub(crate) async fn fake_session(port: u16) -> Session<Conn> {
    let client = connect("127.0.0.1", port, Security::Plaintext)
        .await
        .expect("connect");
    login_password(client, "user", "pass").await.expect("login")
}
