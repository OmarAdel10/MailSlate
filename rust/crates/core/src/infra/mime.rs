//! Header parsing for the sync engine: turns raw IMAP header blocks
//! into the structured fields the messages/threads tables need
//! (ARCHITECTURE §3.1 infra/mime.rs). The IMAP adapter stays
//! wire-focused; this module is the only place that knows mail-parser.
//!
//! The sync engine (later phase-6 slices) is the first consumer; until
//! it lands these items are intentionally unused, so dead-code is
//! allowed module-wide.

#![allow(dead_code)]

use mail_parser::{Address, MessageParser};
use serde_json::{Value, json};

/// Structured header fields for one fetched message. `date_utc` is
/// the Date header's unix epoch, or 0 when the header is missing —
/// [`resolve_date_utc`] layers the IMAP INTERNALDATE and a `now`
/// fallback on top.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MessageRecord {
    pub(crate) message_id_hdr: Option<String>,
    pub(crate) subject: Option<String>,
    pub(crate) from_json: String,
    pub(crate) to_json: String,
    pub(crate) cc_json: String,
    pub(crate) reply_to_json: String,
    pub(crate) date_utc: i64,
    /// Threading root: the first entry of References, else
    /// In-Reply-To, else None (angle brackets stripped like
    /// message_id_hdr). Generic JWZ clustering key.
    pub(crate) root_message_id: Option<String>,
}

/// Parses a raw header block (BODY.PEEK[HEADER.FIELDS (...)] bytes).
/// Unknown or missing fields come back empty; unparseable input
/// yields an all-empty record rather than an error, so one malformed
/// message can never stall a sync.
pub(crate) fn parse_header(header: &[u8]) -> MessageRecord {
    let Some(message) = MessageParser::default().parse(header) else {
        return MessageRecord {
            message_id_hdr: None,
            subject: None,
            from_json: "[]".to_string(),
            to_json: "[]".to_string(),
            cc_json: "[]".to_string(),
            reply_to_json: "[]".to_string(),
            date_utc: 0,
            root_message_id: None,
        };
    };

    MessageRecord {
        message_id_hdr: message
            .message_id()
            .map(|id| id.trim_start_matches('<').trim_end_matches('>').to_string()),
        subject: message.subject().map(str::to_string),
        from_json: addresses_json(message.from()),
        to_json: addresses_json(message.to()),
        cc_json: addresses_json(message.cc()),
        reply_to_json: addresses_json(message.reply_to()),
        date_utc: message.date().map(|date| date.to_timestamp()).unwrap_or(0),
        root_message_id: message
            .header_raw("References")
            .and_then(root_id)
            .or_else(|| message.header_raw("In-Reply-To").and_then(root_id)),
    }
}

/// The first message-id of a References/In-Reply-To header value,
/// with angle brackets stripped; None when the header holds no id.
fn root_id(raw: &str) -> Option<String> {
    raw.split_whitespace()
        .next()
        .map(|id| id.trim_start_matches('<').trim_end_matches('>').to_string())
        .filter(|id| !id.is_empty())
}

/// Normalizes a subject for generic thread clustering: repeatedly
/// strips reply/forward prefixes, lowercases, and collapses
/// whitespace. Nothing left to cluster on yields None.
pub(crate) fn subject_norm(subject: Option<&str>) -> Option<String> {
    let mut current = subject?.trim().to_string();
    loop {
        let lower = current.to_lowercase();
        let stripped = lower
            .strip_prefix("re:")
            .or_else(|| lower.strip_prefix("fw:"))
            .or_else(|| lower.strip_prefix("fwd:"));
        let Some(rest) = stripped else {
            break;
        };
        let trimmed = rest.trim_start().to_string();
        if trimmed.is_empty() {
            return None;
        }
        current = trimmed;
    }
    let normalized = current.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

/// Resolves the message's received-time epoch: the IMAP INTERNALDATE
/// wins (that is when the server got the message), then the Date
/// header, then the caller's `now` clock.
pub(crate) fn resolve_date_utc(
    internal_date: Option<i64>,
    record: &MessageRecord,
    now: i64,
) -> i64 {
    internal_date
        .or(if record.date_utc != 0 {
            Some(record.date_utc)
        } else {
            None
        })
        .unwrap_or(now)
}

/// Serializes an address header into a JSON array of
/// `{"name":..., "address":...}` objects (group syntax flattened).
fn addresses_json(address: Option<&Address<'_>>) -> String {
    let mut entries: Vec<Value> = Vec::new();
    if let Some(address) = address {
        match address {
            Address::List(list) => {
                for addr in list {
                    push_addr(&mut entries, addr);
                }
            }
            Address::Group(group) => {
                for group in group {
                    for addr in &group.addresses {
                        push_addr(&mut entries, addr);
                    }
                }
            }
        }
    }
    Value::Array(entries).to_string()
}

fn push_addr(entries: &mut Vec<Value>, addr: &mail_parser::Addr<'_>) {
    entries.push(json!({
        "name": addr.name.as_deref(),
        "address": addr.address.as_deref(),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"Message-ID: <abc@example.com>\r\n\
        Subject: Hello there\r\n\
        From: Alice <alice@example.com>\r\n\
        To: Bob <bob@other.org>, carol@other.org\r\n\
        Cc: deb@example.net\r\n\
        Reply-To: no-reply@example.com\r\n\
        Date: Mon, 19 Oct 2026 00:00:00 +0000\r\n\
        \r\n";

    #[test]
    fn parses_headers_into_a_message_record() {
        let record = parse_header(SAMPLE);

        assert_eq!(record.message_id_hdr.as_deref(), Some("abc@example.com"));
        assert_eq!(record.subject.as_deref(), Some("Hello there"));
        assert_eq!(
            serde_json::from_str::<Value>(&record.from_json).unwrap(),
            json!([{"name": "Alice", "address": "alice@example.com"}])
        );
        assert_eq!(
            serde_json::from_str::<Value>(&record.to_json).unwrap(),
            json!([
                {"name": "Bob", "address": "bob@other.org"},
                {"name": Value::Null, "address": "carol@other.org"}
            ])
        );
        assert_eq!(
            serde_json::from_str::<Value>(&record.cc_json).unwrap(),
            json!([{"name": Value::Null, "address": "deb@example.net"}])
        );
        assert_eq!(
            serde_json::from_str::<Value>(&record.reply_to_json).unwrap(),
            json!([{"name": Value::Null, "address": "no-reply@example.com"}])
        );
        assert_eq!(record.date_utc, 1_792_368_000);
    }

    #[test]
    fn missing_fields_come_back_empty_or_none() {
        let record = parse_header(b"Subject: only a subject\r\n\r\n");

        assert_eq!(record.message_id_hdr, None);
        assert_eq!(record.subject.as_deref(), Some("only a subject"));
        assert_eq!(record.from_json, "[]");
        assert_eq!(record.to_json, "[]");
        assert_eq!(record.cc_json, "[]");
        assert_eq!(record.reply_to_json, "[]");
        assert_eq!(record.date_utc, 0);
    }

    #[test]
    fn unparseable_bytes_yield_an_empty_record() {
        let record = parse_header(b"\x00\xff not a message \xfe");

        assert_eq!(record.subject, None);
        assert_eq!(record.from_json, "[]");
        assert_eq!(record.date_utc, 0);
    }

    #[test]
    fn date_resolution_prefers_internal_date_then_header_then_now() {
        let record = parse_header(SAMPLE);

        assert_eq!(
            resolve_date_utc(Some(1_792_364_400), &record, 1_000),
            1_792_364_400
        );
        assert_eq!(resolve_date_utc(None, &record, 1_000), 1_792_368_000);
        assert_eq!(
            resolve_date_utc(None, &parse_header(b"Subject: x\r\n\r\n"), 1_000),
            1_000
        );
    }

    #[test]
    fn references_yield_the_root_message_id() {
        let record = parse_header(b"Subject: re: thread\r\nReferences: <b@y> <a@x>\r\n\r\n");

        assert_eq!(record.root_message_id.as_deref(), Some("b@y"));
    }

    #[test]
    fn in_reply_to_is_the_fallback_root() {
        let record = parse_header(b"Subject: re: thread\r\nIn-Reply-To: <parent@x>\r\n\r\n");

        assert_eq!(record.root_message_id.as_deref(), Some("parent@x"));
    }

    #[test]
    fn subject_norm_strips_reply_prefixes_and_case() {
        assert_eq!(
            subject_norm(Some("Re: FWD:  Hello   World")),
            Some("hello world".to_string())
        );
        assert_eq!(subject_norm(Some("Re:")), None);
        assert_eq!(subject_norm(None), None);
    }
}
