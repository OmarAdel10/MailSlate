//! Per-server behavioral flags the sync engine consults.
//!
//! Detected from the CAPABILITY list the connected session advertises;
//! no hard-coded provider assumptions beyond what the wire claims.

#![allow(dead_code)]

use super::ImapError;
use super::connect::Conn;
use async_imap::Session;

/// Behavioral flags the sync engine consults before using an extension.
pub(crate) struct Quirks {
    /// Gmail threading: `X-GM-THRID` is authoritative (`X-GM-EXT-1`).
    pub gmail_thrid: bool,
    /// MODSEQ tracking (`CONDSTORE`) for flag-only deltas.
    pub condstore: bool,
    /// Full quick-resync (`QRESYNC`, implies CONDSTORE).
    pub qresync: bool,
    /// Push-style wait (`IDLE`); false = polling fallback.
    pub idle: bool,
}

/// Reads the session's advertised capabilities and derives the flags.
pub(crate) async fn detect(session: &mut Session<Conn>) -> Result<Quirks, ImapError> {
    let caps = session
        .capabilities()
        .await
        .map_err(|err| ImapError::Protocol(err.to_string()))?;
    Ok(Quirks {
        gmail_thrid: caps.has_str("X-GM-EXT-1"),
        condstore: caps.has_str("CONDSTORE"),
        qresync: caps.has_str("QRESYNC"),
        idle: caps.has_str("IDLE"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::imap::testing::fake_session;

    #[tokio::test]
    async fn detect_reads_extension_capabilities() {
        let port = crate::infra::imap::testing::spawn_fake(|tag, command| {
            if command.starts_with("CAPABILITY") {
                vec![
                    "* CAPABILITY IMAP4rev1 IDLE CONDSTORE QRESYNC X-GM-EXT-1".to_string(),
                    format!("{tag} OK capability done"),
                ]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;
        let mut session = fake_session(port).await;

        let quirks = detect(&mut session).await.expect("detect");

        assert!(quirks.gmail_thrid);
        assert!(quirks.condstore);
        assert!(quirks.qresync);
        assert!(quirks.idle);
    }

    #[tokio::test]
    async fn plain_servers_report_no_extension_quirks() {
        let port = crate::infra::imap::testing::spawn_fake(|tag, command| {
            if command.starts_with("CAPABILITY") {
                vec![
                    "* CAPABILITY IMAP4rev1".to_string(),
                    format!("{tag} OK capability done"),
                ]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;
        let mut session = fake_session(port).await;

        let quirks = detect(&mut session).await.expect("detect");

        assert!(!quirks.gmail_thrid);
        assert!(!quirks.condstore);
        assert!(!quirks.qresync);
        assert!(!quirks.idle);
    }
}
