//! Mailbox discovery and message listing over a live IMAP session.
//!
//! The sync engine (phase 6) consumes these narrow functions and never
//! sees raw protocol types. Until then the cluster is intentionally
//! unused in-crate, so dead-code is allowed module-wide.

#![allow(dead_code)]

use super::ImapError;
use super::connect::Conn;
use async_imap::Session;
use async_imap::types::{Fetch, Flag, NameAttribute};

/// One mailbox from `LIST`: name, hierarchy delimiter, and the RFC 6154
/// special-use attribute when the server advertises one (e.g. `\Sent`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MailboxSummary {
    pub name: String,
    pub delimiter: Option<String>,
    pub special_use: Option<String>,
}

/// Snapshot of a selected mailbox: message count plus the sync cursors
/// the incremental engine tracks (UIDVALIDITY change = full resync).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SelectedMailbox {
    pub exists: u32,
    pub uid_validity: Option<u32>,
    pub uid_next: Option<u32>,
    pub highest_modseq: Option<u64>,
}

/// One envelope row from a chunked `UID FETCH`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EnvelopeRow {
    pub uid: Option<u32>,
    pub flags: Vec<String>,
    pub size: Option<u32>,
    /// RFC 3339 string (chrono `FixedOffset` rendered by us; chrono is
    /// async-imap's dependency, never core's).
    pub internal_date: Option<String>,
    /// Raw header bytes (`BODY.PEEK[HEADER.FIELDS (...)]`), parsed into
    /// structured fields by the sync engine.
    pub header: Option<Vec<u8>>,
}

/// Fixed fetch items for envelope listing (plan §9). Gmail extensions
/// (X-GM-THRID) ride the quirks module, not this shared query.
pub(crate) const ENVELOPE_QUERY: &str = "(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER.FIELDS (DATE FROM TO CC SUBJECT REPLY-TO MESSAGE-ID REFERENCES IN-REPLY-TO)])";

/// Lists every mailbox the server exposes for the current user.
pub(crate) async fn list_mailboxes(
    session: &mut Session<Conn>,
) -> Result<Vec<MailboxSummary>, ImapError> {
    let stream = session
        .list(None, Some("*"))
        .await
        .map_err(|err| ImapError::Protocol(err.to_string()))?;
    let mut mailboxes = Vec::new();
    let mut stream = std::pin::pin!(stream);
    while let Some(item) = futures_util::StreamExt::next(&mut stream).await {
        let name = item.map_err(|err| ImapError::Protocol(err.to_string()))?;
        mailboxes.push(MailboxSummary {
            name: name.name().to_string(),
            delimiter: name.delimiter().map(str::to_string),
            special_use: special_use(name.attributes()),
        });
    }
    Ok(mailboxes)
}

/// Maps dedicated RFC 6154 special-use variants to their wire strings.
/// `NameAttribute` is `#[non_exhaustive]`, so the catch-all arm is
/// required for forward compatibility.
fn special_use(attributes: &[NameAttribute<'_>]) -> Option<String> {
    attributes.iter().find_map(|attr| match attr {
        NameAttribute::All => Some("\\All".to_string()),
        NameAttribute::Archive => Some("\\Archive".to_string()),
        NameAttribute::Drafts => Some("\\Drafts".to_string()),
        NameAttribute::Flagged => Some("\\Flagged".to_string()),
        NameAttribute::Junk => Some("\\Junk".to_string()),
        NameAttribute::Sent => Some("\\Sent".to_string()),
        NameAttribute::Trash => Some("\\Trash".to_string()),
        _ => None,
    })
}

/// Renders a system flag to its wire form; keywords and custom flags
/// keep their server-provided text.
fn flag_string(flag: Flag<'_>) -> String {
    match flag {
        Flag::Seen => "\\Seen".to_string(),
        Flag::Answered => "\\Answered".to_string(),
        Flag::Flagged => "\\Flagged".to_string(),
        Flag::Deleted => "\\Deleted".to_string(),
        Flag::Draft => "\\Draft".to_string(),
        Flag::Recent => "\\Recent".to_string(),
        Flag::MayCreate => "\\*".to_string(),
        Flag::Custom(text) => text.into_owned(),
    }
}

fn envelope_row(fetch: Fetch) -> EnvelopeRow {
    EnvelopeRow {
        uid: fetch.uid,
        flags: fetch.flags().map(flag_string).collect(),
        size: fetch.size,
        internal_date: fetch.internal_date().map(|date| date.to_rfc3339()),
        header: fetch.header().map(<[u8]>::to_vec),
    }
}

/// Selects a mailbox read-write and reports its sync cursors.
pub(crate) async fn select_mailbox(
    session: &mut Session<Conn>,
    name: &str,
) -> Result<SelectedMailbox, ImapError> {
    let mailbox = session
        .select(name)
        .await
        .map_err(|err| ImapError::Protocol(err.to_string()))?;
    Ok(SelectedMailbox {
        exists: mailbox.exists,
        uid_validity: mailbox.uid_validity,
        uid_next: mailbox.uid_next,
        highest_modseq: mailbox.highest_modseq,
    })
}

/// Fetches one chunk of envelopes by UID set (the sync engine chunks at
/// 500 UIDs per request).
pub(crate) async fn fetch_envelope_chunk(
    session: &mut Session<Conn>,
    uid_set: &str,
) -> Result<Vec<EnvelopeRow>, ImapError> {
    let stream = session
        .uid_fetch(uid_set, ENVELOPE_QUERY)
        .await
        .map_err(|err| ImapError::Protocol(err.to_string()))?;
    let mut rows = Vec::new();
    let mut stream = std::pin::pin!(stream);
    while let Some(item) = futures_util::StreamExt::next(&mut stream).await {
        let fetch = item.map_err(|err| ImapError::Protocol(err.to_string()))?;
        rows.push(envelope_row(fetch));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::imap::testing::{fake_session, spawn_fake};

    #[tokio::test]
    async fn list_returns_mailboxes_with_special_use() {
        let port = spawn_fake(|tag, command| {
            if command.starts_with("LIST") {
                vec![
                    r#"* LIST (\HasNoChildren \Sent) "/" "Sent""#.to_string(),
                    r#"* LIST (\HasNoChildren) "/" INBOX"#.to_string(),
                    format!("{tag} OK list done"),
                ]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;
        let mut session = fake_session(port).await;

        let mailboxes = list_mailboxes(&mut session).await.expect("list");

        assert_eq!(mailboxes.len(), 2, "got {mailboxes:?}");
        assert_eq!(mailboxes[0].name, "Sent");
        assert_eq!(mailboxes[0].delimiter.as_deref(), Some("/"));
        assert_eq!(mailboxes[0].special_use.as_deref(), Some("\\Sent"));
        assert_eq!(mailboxes[1].name, "INBOX");
        assert_eq!(mailboxes[1].special_use, None);
    }

    #[tokio::test]
    async fn select_reports_exists_and_uidvalidity() {
        let port = spawn_fake(|tag, command| {
            if command.starts_with("SELECT") {
                vec![
                    "* 3 EXISTS".to_string(),
                    "* OK [UIDVALIDITY 1234] UIDs valid".to_string(),
                    "* OK [UIDNEXT 5] Predicted next uid".to_string(),
                    "* OK [HIGHESTMODSEQ 77] Highest".to_string(),
                    format!("{tag} OK select done"),
                ]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;
        let mut session = fake_session(port).await;

        let selected = select_mailbox(&mut session, "INBOX").await.expect("select");

        assert_eq!(selected.exists, 3);
        assert_eq!(selected.uid_validity, Some(1234));
        assert_eq!(selected.uid_next, Some(5));
        assert_eq!(selected.highest_modseq, Some(77));
    }

    #[tokio::test]
    async fn fetch_returns_uid_flags_size_and_headers() {
        let port = spawn_fake(|tag, command| {
            if command.starts_with("UID FETCH") {
                vec![
                    concat!(
                        r#"* 1 FETCH (UID 1 FLAGS (\Seen) RFC822.SIZE 1024 "#,
                        r#"INTERNALDATE "17-Jul-1996 02:44:25 -0700" "#,
                        r#"BODY[HEADER.FIELDS (DATE)] {9}"#,
                        "\r\nDate: x\r\n)",
                    )
                    .to_string(),
                    format!("{tag} OK fetch done"),
                ]
            } else {
                vec![format!("{tag} OK")]
            }
        })
        .await;
        let mut session = fake_session(port).await;

        let rows = fetch_envelope_chunk(&mut session, "1")
            .await
            .expect("fetch");

        assert_eq!(rows.len(), 1, "got {rows:?}");
        let row = &rows[0];
        assert_eq!(row.uid, Some(1));
        assert!(
            row.flags.iter().any(|f| f == "\\Seen"),
            "got {:?}",
            row.flags
        );
        assert_eq!(row.size, Some(1024));
        assert!(
            row.header
                .as_deref()
                .is_some_and(|h| h.windows(7).any(|w| w == b"Date: x")),
            "got {:?}",
            row.header
        );
    }
}
