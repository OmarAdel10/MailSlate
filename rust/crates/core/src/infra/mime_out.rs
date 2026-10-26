//! Outgoing RFC 5322 message construction for the compose/send
//! pipeline (PRD F5.3/F5.7, USER_FLOW §6, ARCHITECTURE §9.6
//! idempotency prerequisites). Hand-rolled emission: full control
//! over header hygiene (injection guards, ASCII-only header block,
//! encoded-words), Message-ID angle-bracket stamping (server-echo
//! dedupe), and deterministic multipart attachment encoding.
//! Parsing stays in `mime.rs`; this module never parses.
//!
//! The compose domain (later M4 slices) is the first consumer;
//! until it lands these items are intentionally unused outside
//! tests, so dead-code is allowed module-wide (same precedent as
//! `mime.rs`).

#![allow(dead_code)]

use base64::Engine as _;

/// One outgoing address: optional display name plus the bare
/// address. Injection-guarded at build time.
#[derive(Debug)]
pub(crate) struct OutAddress {
    pub(crate) name: Option<String>,
    pub(crate) address: String,
}

/// One staged attachment: its stored bytes travel with the
/// message; blob digest resolution happens in the compose domain.
pub(crate) struct OutAttachment {
    pub(crate) filename: String,
    pub(crate) mime_type: String,
    pub(crate) content: Vec<u8>,
}

/// Everything needed to emit one complete RFC 5322 message.
/// `message_id` is the bare local@domain id; the wire form gets
/// angle brackets (PRD F5.7 — stamped locally, dedupes server echo).
pub(crate) struct OutgoingMessage {
    pub(crate) from: OutAddress,
    pub(crate) to: Vec<OutAddress>,
    pub(crate) cc: Vec<OutAddress>,
    /// Envelope-only recipients: validated, never emitted on the wire.
    pub(crate) bcc: Vec<OutAddress>,
    pub(crate) subject: String,
    pub(crate) body: String,
    pub(crate) message_id: String,
    pub(crate) in_reply_to: Option<String>,
    pub(crate) references: Vec<String>,
    /// Unix epoch; rendered as an RFC 5322 UTC date.
    pub(crate) date_epoch: i64,
    pub(crate) attachments: Vec<OutAttachment>,
}

/// Build failures that must block a send.
#[derive(Debug)]
pub(crate) enum BuildError {
    /// A caller-supplied field carries CR/LF (SMTP header injection).
    HeaderInjection(&'static str),
    /// To, Cc, and Bcc are all empty — nothing to send to.
    NoRecipient,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::HeaderInjection(field) => {
                write!(f, "header injection blocked in {field}")
            }
            BuildError::NoRecipient => write!(f, "message has no recipient"),
        }
    }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Renders the whole message: validated headers, then the body
/// (and multipart attachments when staged).
pub(crate) fn build_message(msg: &OutgoingMessage) -> Result<Vec<u8>, BuildError> {
    guard_injection(msg)?;
    if msg.to.is_empty() && msg.cc.is_empty() && msg.bcc.is_empty() {
        return Err(BuildError::NoRecipient);
    }

    let mut out = String::new();
    push_address_header(&mut out, "From", std::iter::once(&msg.from));
    push_address_header(&mut out, "To", msg.to.iter());
    if !msg.cc.is_empty() {
        push_address_header(&mut out, "Cc", msg.cc.iter());
    }
    push_header(&mut out, "Subject", &unstructured(&msg.subject));
    push_header(&mut out, "Date", &format_rfc5322_date(msg.date_epoch));
    push_header(&mut out, "Message-ID", &format!("<{}>", msg.message_id));
    if let Some(parent) = &msg.in_reply_to {
        push_header(&mut out, "In-Reply-To", &format!("<{parent}>"));
    }
    if !msg.references.is_empty() {
        let refs = msg
            .references
            .iter()
            .map(|id| format!("<{id}>"))
            .collect::<Vec<_>>()
            .join(" ");
        push_header(&mut out, "References", &refs);
    }
    push_header(&mut out, "MIME-Version", "1.0");

    let body = normalize_crlf(&msg.body);
    if msg.attachments.is_empty() {
        let (encoded, cte) = encode_body(&body);
        push_header(&mut out, "Content-Type", "text/plain; charset=utf-8");
        push_header(&mut out, "Content-Transfer-Encoding", cte);
        out.push_str("\r\n");
        out.push_str(&encoded);
    } else {
        let boundary = format!("=_mailslate_{}", uuid::Uuid::new_v4().simple());
        push_header(
            &mut out,
            "Content-Type",
            &format!("multipart/mixed; boundary=\"{boundary}\""),
        );
        out.push_str("\r\n");
        out.push_str(&format!("--{boundary}\r\n"));
        let (encoded, cte) = encode_body(&body);
        out.push_str("Content-Type: text/plain; charset=utf-8\r\n");
        out.push_str(&format!("Content-Transfer-Encoding: {cte}\r\n"));
        out.push_str("\r\n");
        out.push_str(&encoded);
        out.push_str("\r\n");
        for att in &msg.attachments {
            out.push_str(&format!("--{boundary}\r\n"));
            out.push_str(&format!("Content-Type: {}\r\n", att.mime_type));
            out.push_str(&format!(
                "Content-Disposition: attachment; filename=\"{}\"\r\n",
                unstructured(&att.filename)
            ));
            out.push_str("Content-Transfer-Encoding: base64\r\n");
            out.push_str("\r\n");
            out.push_str(&wrap_base64(&att.content));
            out.push_str("\r\n");
        }
        out.push_str(&format!("--{boundary}--\r\n"));
    }

    Ok(out.into_bytes())
}

/// Rejects CR/LF anywhere a caller value would land in a header:
/// the only way an untrusted string may smuggle a second header
/// line onto the wire.
fn guard_injection(msg: &OutgoingMessage) -> Result<(), BuildError> {
    fn clean(value: &str, field: &'static str) -> Result<(), BuildError> {
        if value.contains(['\r', '\n']) {
            Err(BuildError::HeaderInjection(field))
        } else {
            Ok(())
        }
    }

    clean(&msg.subject, "subject")?;
    clean(&msg.message_id, "message id")?;
    clean(&msg.from.address, "sender address")?;
    if let Some(name) = &msg.from.name {
        clean(name, "sender display name")?;
    }
    for list in [&msg.to, &msg.cc, &msg.bcc] {
        for recipient in list {
            clean(&recipient.address, "recipient address")?;
            if let Some(name) = &recipient.name {
                clean(name, "recipient display name")?;
            }
        }
    }
    if let Some(parent) = &msg.in_reply_to {
        clean(parent, "reply header")?;
    }
    for id in &msg.references {
        clean(id, "reply header")?;
    }
    for att in &msg.attachments {
        clean(&att.filename, "attachment filename")?;
        clean(&att.mime_type, "attachment media type")?;
    }
    Ok(())
}

/// `Name: value\r\n` with the value pre-rendered.
fn push_header(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push_str(": ");
    out.push_str(value);
    out.push_str("\r\n");
}

/// Emits `Header: Name <a@x>, b@y` from validated addresses.
/// Bcc is never routed through here.
fn push_address_header<'a>(
    out: &mut String,
    name: &str,
    addresses: impl Iterator<Item = &'a OutAddress>,
) {
    let rendered = addresses.map(render_address).collect::<Vec<_>>().join(", ");
    push_header(out, name, &rendered);
}

/// `Display Name <addr>` / `addr`. Display names either match the
/// raw-safe set (no comma, quote, or non-ASCII bytes — anything
/// else would break address-list parsing) or become encoded-words.
fn render_address(address: &OutAddress) -> String {
    match &address.name {
        Some(name) if raw_name_safe(name) => format!("{name} <{}>", address.address),
        Some(name) => format!("{} <{}>", encoded_word(name), address.address),
        None => address.address.clone(),
    }
}

/// Alphanumerics plus the punctuation that cannot break a
/// structured header: no comma, colon, quote, angle bracket,
/// backslash, or anything non-ASCII.
fn raw_name_safe(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b' ' | b'\''
                        | b'-'
                        | b'.'
                        | b'&'
                        | b'!'
                        | b'*'
                        | b'+'
                        | b'/'
                        | b'_'
                        | b'`'
                        | b'{'
                )
        })
}

/// Unstructured header value (Subject, filename): raw when pure
/// ASCII, otherwise a utf-8 q-encoded word keeps the header block
/// 7-bit clean.
fn unstructured(value: &str) -> String {
    if value.is_ascii() {
        value.to_string()
    } else {
        encoded_word(value)
    }
}

/// RFC 2047 q-encoded word, utf-8 charset.
fn encoded_word(input: &str) -> String {
    let mut out = String::from("=?utf-8?q?");
    for byte in input.bytes() {
        match byte {
            b' ' => out.push('_'),
            b'!' | b'*' | b'+' | b'-' | b'/' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("={byte:02X}")),
        }
    }
    out.push_str("?=");
    out
}

/// RFC 5322 UTC date from a unix epoch.
fn format_rfc5322_date(epoch: i64) -> String {
    let days = epoch.div_euclid(86_400);
    let secs = epoch.rem_euclid(86_400);
    let (hour, minute, second) = (secs / 3_600, (secs % 3_600) / 60, secs % 60);

    // Civil date from a day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    // 1970-01-01 was a Thursday; Monday-based weekday index.
    let weekday = ((days + 3).rem_euclid(7)) as usize;

    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        WEEKDAYS[weekday],
        day,
        MONTHS[(month - 1) as usize],
        year,
        hour,
        minute,
        second
    )
}

/// Bare LF and CRLF variants both settle to CRLF, and the body
/// always ends with exactly one line break.
fn normalize_crlf(body: &str) -> String {
    let mut normalized = body.replace("\r\n", "\n").replace('\n', "\r\n");
    if !normalized.ends_with("\r\n") {
        normalized.push_str("\r\n");
    }
    normalized
}

/// Returns (encoded body, transfer encoding). ASCII-only bodies go
/// out verbatim as 7bit; anything else is quoted-printable.
fn encode_body(body: &str) -> (String, &'static str) {
    if body.is_ascii() {
        (body.to_string(), "7bit")
    } else {
        (quoted_printable(body.as_bytes()), "quoted-printable")
    }
}

/// RFC 2045 quoted-printable: printable ASCII (space included)
/// stays literal; `=` and non-ASCII bytes become =XX. Trailing
/// spaces before a (soft or hard) break are encoded. Input is
/// expected CRLF-normalized.
fn quoted_printable(bytes: &[u8]) -> String {
    let encode_into = |line: &mut String, byte: u8| {
        if byte == b' ' || ((33..=126).contains(&byte) && byte != b'=') {
            line.push(byte as char);
        } else {
            line.push_str(&format!("={byte:02X}"));
        }
    };
    let trim_trailing_spaces = |line: &mut String| {
        while line.ends_with(' ') {
            line.pop();
            line.push_str("=20");
        }
    };

    let mut out = String::new();
    let mut line = String::new();
    for &byte in bytes {
        match byte {
            b'\r' => continue,
            b'\n' => {
                trim_trailing_spaces(&mut line);
                out.push_str(&line);
                out.push_str("\r\n");
                line.clear();
            }
            _ => {
                encode_into(&mut line, byte);
                if line.len() >= 75 {
                    trim_trailing_spaces(&mut line);
                    out.push_str(&line);
                    out.push_str("=\r\n");
                    line.clear();
                }
            }
        }
    }
    trim_trailing_spaces(&mut line);
    out.push_str(&line);
    out
}

/// Base64 with 76-character CRLF line wraps and a trailing CRLF.
fn wrap_base64(content: &[u8]) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(content);
    let mut out = String::new();
    let mut rest = encoded.as_str();
    while rest.len() > 76 {
        let (line, tail) = rest.split_at(76);
        out.push_str(line);
        out.push_str("\r\n");
        rest = tail;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Epoch of Mon, 19 Oct 2026 00:00:00 UTC (same fixture as
    /// `mime.rs` tests).
    const EPOCH_MON_19: i64 = 1_792_368_000;

    fn addr(name: Option<&str>, address: &str) -> OutAddress {
        OutAddress {
            name: name.map(str::to_string),
            address: address.to_string(),
        }
    }

    fn attachment(filename: &str, mime_type: &str, content: &[u8]) -> OutAttachment {
        OutAttachment {
            filename: filename.to_string(),
            mime_type: mime_type.to_string(),
            content: content.to_vec(),
        }
    }

    /// The minimal well-formed message: one from, one to, ASCII
    /// subject/body, no attachments.
    fn base_message() -> OutgoingMessage {
        OutgoingMessage {
            from: addr(Some("Alice"), "alice@example.com"),
            to: vec![addr(Some("Bob"), "bob@other.org")],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "Hi".to_string(),
            body: "Hello body\r\nsecond line".to_string(),
            message_id: "abc123@example.com".to_string(),
            in_reply_to: None,
            references: Vec::new(),
            date_epoch: EPOCH_MON_19,
            attachments: Vec::new(),
        }
    }

    fn headers_of(bytes: &[u8]) -> &str {
        let text = std::str::from_utf8(bytes).expect("wire bytes are utf-8");
        let (headers, _) = text
            .split_once("\r\n\r\n")
            .expect("headers and body separated by a blank line");
        headers
    }

    fn build(msg: &OutgoingMessage) -> String {
        String::from_utf8(build_message(msg).expect("build succeeds")).expect("wire bytes utf-8")
    }

    #[test]
    fn builds_minimal_plaintext_message() {
        let wire = build(&base_message());

        assert_eq!(
            wire,
            concat!(
                "From: Alice <alice@example.com>\r\n",
                "To: Bob <bob@other.org>\r\n",
                "Subject: Hi\r\n",
                "Date: Mon, 19 Oct 2026 00:00:00 +0000\r\n",
                "Message-ID: <abc123@example.com>\r\n",
                "MIME-Version: 1.0\r\n",
                "Content-Type: text/plain; charset=utf-8\r\n",
                "Content-Transfer-Encoding: 7bit\r\n",
                "\r\n",
                "Hello body\r\n",
                "second line\r\n",
            )
        );
    }

    #[test]
    fn emits_cc_when_present_and_omits_when_empty() {
        let mut msg = base_message();
        msg.cc = vec![addr(None, "carol@other.org")];

        assert!(build(&msg).contains("Cc: carol@other.org\r\n"));
        assert!(!build(&base_message()).contains("\r\nCc:"));
    }

    #[test]
    fn formats_epoch_zero_date() {
        let mut msg = base_message();
        msg.date_epoch = 0;

        assert!(build(&msg).contains("Date: Thu, 01 Jan 1970 00:00:00 +0000\r\n"));
    }

    #[test]
    fn emits_reply_headers_when_set() {
        let mut msg = base_message();
        msg.in_reply_to = Some("parent@x".to_string());
        msg.references = vec!["root@x".to_string(), "parent@x".to_string()];

        let wire = build(&msg);

        assert!(wire.contains("In-Reply-To: <parent@x>\r\n"));
        assert!(wire.contains("References: <root@x> <parent@x>\r\n"));
    }

    #[test]
    fn bcc_recipients_never_appear_on_the_wire() {
        let mut msg = base_message();
        msg.bcc = vec![addr(Some("Secret"), "secret@hidden.org")];

        let wire = build(&msg);

        assert!(!wire.contains("Bcc"));
        assert!(!wire.contains("secret@hidden.org"));
        assert!(!wire.contains("Secret"));
    }

    #[test]
    fn formats_named_and_bare_addresses_together() {
        let mut msg = base_message();
        msg.to = vec![
            addr(Some("Bob"), "bob@other.org"),
            addr(None, "carol@other.org"),
        ];

        assert!(build(&msg).contains("To: Bob <bob@other.org>, carol@other.org\r\n"));
    }

    #[test]
    fn rejects_header_injection_in_subject() {
        let mut msg = base_message();
        msg.subject = "fine\r\nBcc: evil@x".to_string();

        assert!(matches!(
            build_message(&msg),
            Err(BuildError::HeaderInjection(_))
        ));
    }

    #[test]
    fn rejects_header_injection_in_recipient_name() {
        let mut msg = base_message();
        msg.to = vec![addr(Some("Bob\r\nBcc: evil@x"), "bob@other.org")];

        assert!(matches!(
            build_message(&msg),
            Err(BuildError::HeaderInjection(_))
        ));
    }

    #[test]
    fn rejects_header_injection_in_attachment_filename() {
        let mut msg = base_message();
        msg.attachments = vec![attachment(
            "a.pdf\r\nBcc: evil@x",
            "application/pdf",
            b"bytes",
        )];

        assert!(matches!(
            build_message(&msg),
            Err(BuildError::HeaderInjection(_))
        ));
    }

    #[test]
    fn rejects_message_without_any_recipient() {
        let mut msg = base_message();
        msg.to.clear();
        msg.cc.clear();
        msg.bcc.clear();

        assert!(matches!(build_message(&msg), Err(BuildError::NoRecipient)));
    }

    #[test]
    fn multipart_mixed_encodes_attachment_with_base64() {
        use base64::Engine as _;

        let content = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let mut msg = base_message();
        msg.attachments = vec![attachment("doc.pdf", "application/pdf", &content)];

        let wire = build(&msg);
        let expected_b64 = base64::engine::general_purpose::STANDARD.encode(&content);

        assert!(wire.contains("Content-Type: multipart/mixed; boundary=\""));
        assert!(wire.contains("Content-Type: application/pdf\r\n"));
        assert!(wire.contains("Content-Disposition: attachment; filename=\"doc.pdf\"\r\n"));
        assert!(wire.contains(&format!("\r\n{expected_b64}\r\n")));
        // The plaintext part still ships inside the multipart.
        assert!(wire.contains("Content-Type: text/plain; charset=utf-8\r\n"));
    }

    #[test]
    fn boundary_is_unique_per_message_and_absent_from_parts() {
        let content = b"boundary collision bait: ==mailslate==";
        let mut msg = base_message();
        msg.body = "also bait: ==mailslate==".to_string();
        msg.attachments = vec![attachment("b.txt", "text/plain", content)];

        let wire_one = build(&msg);
        let wire_two = build(&msg);

        let extract_boundary = |wire: &str| -> String {
            let marker = "boundary=\"";
            let start = wire.find(marker).expect("boundary param") + marker.len();
            let end = wire[start..].find('"').expect("closing quote") + start;
            wire[start..end].to_string()
        };

        let b_one = extract_boundary(&wire_one);
        let b_two = extract_boundary(&wire_two);
        assert_ne!(b_one, b_two, "each build must stamp a fresh boundary");
        // The boundary token must never occur inside the parts it delimits.
        assert!(!msg.body.contains(&b_one));
        assert!(!String::from_utf8_lossy(content).contains(&b_one));
    }

    #[test]
    fn utf8_body_is_quoted_printable_and_decodes_back() {
        let mut msg = base_message();
        msg.body = "Héllo ☺ wörld".to_string();

        let wire = build(&msg);

        assert!(wire.contains("Content-Transfer-Encoding: quoted-printable\r\n"));
        assert!(wire.contains("H=C3=A9llo =E2=98=BA w=C3=B6rld\r\n"));
    }

    #[test]
    fn non_ascii_display_names_keep_the_header_block_ascii() {
        let mut msg = base_message();
        msg.from = addr(Some("Ünal Şimşek"), "unal@example.com");
        msg.to = vec![addr(Some("Ömer"), "omer@example.com")];
        msg.subject = "Sübjekt ✓".to_string();

        let wire = build(&msg);

        let headers = headers_of(wire.as_bytes());
        assert!(
            headers.is_ascii(),
            "header block must stay ASCII: {headers}"
        );
        assert!(headers.contains("From: =?utf-8?q?"));
        assert!(headers.contains("Subject: =?utf-8?q?"));
    }

    #[test]
    fn normalizes_bare_lf_to_crlf_in_body() {
        let mut msg = base_message();
        msg.body = "line one\nline two".to_string();

        let wire = build(&msg);

        assert!(wire.contains("\r\n\r\nline one\r\nline two\r\n"), "{wire}");
    }
}
