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
#[derive(Debug)]
pub(crate) struct OutAttachment {
    pub(crate) filename: String,
    pub(crate) mime_type: String,
    pub(crate) content: Vec<u8>,
}

/// Everything needed to emit one complete RFC 5322 message.
/// `message_id` is the bare local@domain id; the wire form gets
/// angle brackets (PRD F5.7 — stamped locally, dedupes server echo).
#[derive(Debug)]
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
#[derive(Debug, thiserror::Error)]
pub(crate) enum BuildError {
    /// A caller-supplied field carries CR/LF (SMTP header injection).
    #[error("header injection blocked in {field}")]
    HeaderInjection {
        /// The offending field, for send-time error reporting.
        field: &'static str,
    },
    /// To, Cc, and Bcc are all empty — nothing to send to.
    #[error("message has no recipient")]
    NoRecipient,
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
    // Bcc-only sends carry no To/Cc headers: an empty address
    // list is invalid RFC 5322 and trips strict relays.
    if !msg.to.is_empty() {
        push_address_header(&mut out, "To", msg.to.iter());
    }
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
                quoted_string(&att.filename)
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
            Err(BuildError::HeaderInjection { field })
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
        clean(parent, "in-reply-to")?;
    }
    for id in &msg.references {
        clean(id, "references")?;
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
/// Empty or whitespace-only names render as bare addresses.
fn render_address(address: &OutAddress) -> String {
    match &address.name {
        Some(name) if !name.trim().is_empty() && raw_name_safe(name) => {
            format!("{name} <{}>", address.address)
        }
        Some(name) if !name.trim().is_empty() => {
            format!("{} <{}>", encoded_word(name), address.address)
        }
        _ => address.address.clone(),
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

/// Unstructured header value (Subject, filename): raw only when
/// every byte is printable ASCII and the text contains no `=?`
/// (which would self-decode as an encoded-word in conformant
/// MUAs); anything else — control bytes included — becomes a
/// utf-8 q-encoded word, keeping the header block 7-bit clean.
fn unstructured(value: &str) -> String {
    let printable = value.bytes().all(|b| (32..=126).contains(&b));
    if printable && !value.contains("=?") {
        value.to_string()
    } else {
        encoded_word(value)
    }
}

/// Quoted-string parameter value (RFC 5322 §3.2.4): keeps the
/// header block ASCII via an encoded-word when needed, then
/// escapes the two characters that could otherwise break out of
/// the surrounding double quotes. Encoded-words contain neither
/// `"` nor `\`, so the escapes only ever apply to the raw path.
fn quoted_string(value: &str) -> String {
    unstructured(value)
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
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

/// Bare CR, bare LF, and CRLF variants all settle to CRLF, and
/// the body always ends with exactly one line break.
fn normalize_crlf(body: &str) -> String {
    let mut normalized = body
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n");
    if !normalized.ends_with("\r\n") {
        normalized.push_str("\r\n");
    }
    normalized
}

/// Returns (encoded body, transfer encoding). Bodies confined to
/// the RFC 2045 7bit ranges — printable ASCII, TAB, and the CRLF
/// pairs `normalize_crlf` guarantees — and to the 998-char line
/// limit ship verbatim; anything else (control bytes, overlong
/// lines) is quoted-printable.
fn encode_body(body: &str) -> (String, &'static str) {
    let mut seven_bit = true;
    let mut line_len = 0usize;
    for &byte in body.as_bytes() {
        match byte {
            b'\r' => {}
            b'\n' => line_len = 0,
            _ => {
                if !matches!(byte, b'\t' | 32..=126) || {
                    line_len += 1;
                    line_len > 998
                } {
                    seven_bit = false;
                    break;
                }
            }
        }
    }
    if seven_bit {
        (body.to_string(), "7bit")
    } else {
        (quoted_printable(body.as_bytes()), "quoted-printable")
    }
}

/// RFC 2045 quoted-printable: printable ASCII (space included)
/// stays literal; `=` and non-ASCII bytes become =XX. Trailing
/// spaces before a (soft or hard) break are all encoded. Input is
/// expected CRLF-normalized and to end with a line break, so no
/// final flush is needed. Admission budgets a space at its =20
/// worst case — including the whole trailing run — so a line
/// flushed at the 75-char budget can never exceed §6.7(a)'s
/// 76-char wire cap after spaces expand.
fn quoted_printable(bytes: &[u8]) -> String {
    let encode_into = |line: &mut String, byte: u8| {
        if byte == b' ' || ((33..=126).contains(&byte) && byte != b'=') {
            line.push(byte as char);
        } else {
            line.push_str(&format!("={byte:02X}"));
        }
    };
    let encode_trailing_spaces = |line: &mut String, run: &mut usize| {
        for _ in 0..*run {
            line.pop();
        }
        for _ in 0..*run {
            line.push_str("=20");
        }
        *run = 0;
    };

    let mut out = String::new();
    let mut line = String::new();
    let mut trailing_spaces = 0usize;
    for &byte in bytes {
        match byte {
            b'\r' => continue,
            b'\n' => {
                encode_trailing_spaces(&mut line, &mut trailing_spaces);
                out.push_str(&line);
                out.push_str("\r\n");
                line.clear();
            }
            _ => {
                let is_space = byte == b' ';
                let literal = is_space || ((33..=126).contains(&byte) && byte != b'=');
                let mut new_run = if is_space { trailing_spaces + 1 } else { 0 };
                let worst_case = if literal {
                    line.len() + 1 + 2 * new_run
                } else {
                    line.len() + 3
                };
                if worst_case > 75 {
                    encode_trailing_spaces(&mut line, &mut trailing_spaces);
                    out.push_str(&line);
                    out.push_str("=\r\n");
                    line.clear();
                    // The line restarts empty: recompute the run.
                    new_run = if is_space { 1 } else { 0 };
                }
                encode_into(&mut line, byte);
                trailing_spaces = new_run;
            }
        }
    }
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

    fn extract_boundary(wire: &str) -> String {
        let marker = "boundary=\"";
        let start = wire.find(marker).expect("boundary param") + marker.len();
        let end = wire[start..].find('"').expect("closing quote") + start;
        wire[start..end].to_string()
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
    fn guard_blocks_every_header_landing_field_with_exact_label() {
        type Injector = Box<dyn Fn(&mut OutgoingMessage)>;
        let cases: Vec<(&'static str, Injector)> = vec![
            (
                "subject",
                Box::new(|m: &mut OutgoingMessage| {
                    m.subject = "fine\r\nBcc: evil@x".to_string();
                }),
            ),
            (
                "message id",
                Box::new(|m: &mut OutgoingMessage| {
                    m.message_id = "bad\r\nBcc: evil@x".to_string();
                }),
            ),
            (
                "sender address",
                Box::new(|m: &mut OutgoingMessage| {
                    m.from.address = "bad\r\nBcc: evil@x".to_string();
                }),
            ),
            (
                "sender display name",
                Box::new(|m: &mut OutgoingMessage| {
                    m.from = addr(Some("Bad\r\nBcc: evil@x"), "alice@example.com");
                }),
            ),
            (
                "recipient address",
                Box::new(|m: &mut OutgoingMessage| {
                    m.to = vec![addr(None, "bad\r\nBcc: evil@x")];
                }),
            ),
            (
                "recipient display name",
                Box::new(|m: &mut OutgoingMessage| {
                    m.to = vec![addr(Some("Bad\r\nBcc: evil@x"), "bob@other.org")];
                }),
            ),
            (
                "in-reply-to",
                Box::new(|m: &mut OutgoingMessage| {
                    m.in_reply_to = Some("bad\r\nBcc: evil@x".to_string());
                }),
            ),
            (
                "references",
                Box::new(|m: &mut OutgoingMessage| {
                    m.references = vec!["bad\r\nBcc: evil@x".to_string()];
                }),
            ),
            (
                "attachment filename",
                Box::new(|m: &mut OutgoingMessage| {
                    m.attachments = vec![attachment(
                        "bad\r\nBcc: evil@x",
                        "application/pdf",
                        b"bytes",
                    )];
                }),
            ),
            (
                "attachment media type",
                Box::new(|m: &mut OutgoingMessage| {
                    m.attachments = vec![attachment(
                        "ok.pdf",
                        "application/pdf\r\nBcc: evil@x",
                        b"bytes",
                    )];
                }),
            ),
        ];

        for (label, inject) in cases {
            let mut msg = base_message();
            inject(&mut msg);
            match build_message(&msg) {
                Err(BuildError::HeaderInjection { field }) => assert_eq!(
                    field, label,
                    "injection in {label} must name the offending field"
                ),
                other => panic!("expected HeaderInjection({label}), got {other:?}"),
            }
        }
    }

    #[test]
    fn bcc_only_send_omits_to_and_cc_headers() {
        let mut msg = base_message();
        msg.to.clear();
        msg.bcc = vec![addr(Some("Secret"), "secret@hidden.org")];

        let wire = build(&msg);

        assert!(!wire.contains("\r\nTo:"), "no empty To header: {wire}");
        assert!(!wire.contains("\r\nCc:"));
        assert!(wire.contains("From: Alice <alice@example.com>\r\n"));
        assert!(wire.contains("Subject: Hi\r\n"));
        assert!(!wire.contains("secret@hidden.org"));
    }

    #[test]
    fn control_bytes_force_quoted_printable_not_7bit() {
        let mut msg = base_message();
        msg.body = "bell\u{7}and nul\u{0}here".to_string();

        let wire = build(&msg);

        assert!(wire.contains("Content-Transfer-Encoding: quoted-printable\r\n"));
        assert!(wire.contains("=07"));
        assert!(wire.contains("=00"));
    }

    #[test]
    fn quoted_printable_lines_never_exceed_76_chars() {
        // 74 literals then a multi-byte accent: the overflow check
        // must fire before the =XX push, not after (RFC 2045 6.7(a)
        // caps a wire line at 76 characters including the soft `=`).
        let mut msg = base_message();
        msg.body = format!("{}\u{e9}{}", "a".repeat(74), "b".repeat(50));

        let wire = build(&msg);

        for line in wire.split("\r\n") {
            assert!(
                line.len() <= 76,
                "wire line is {} chars (limit 76): {line:?}",
                line.len()
            );
        }
    }

    #[test]
    fn quoted_printable_encodes_hard_breaks_trailing_spaces_and_equals() {
        let mut msg = base_message();
        // ö forces the QP path; the trailing space and `=` exercise
        // the trim-before-hard-break and =3D branches.
        msg.body = "wörld \nsecond=half".to_string();

        let wire = build(&msg);

        assert!(
            wire.contains("w=C3=B6rld=20\r\nsecond=3Dhalf\r\n"),
            "{wire}"
        );
    }

    #[test]
    fn attachment_filename_with_quote_is_escaped_in_quoted_string() {
        let mut msg = base_message();
        msg.attachments = vec![attachment("He said \"hi\".txt", "text/plain", b"x")];

        let wire = build(&msg);

        assert!(
            wire.contains("filename=\"He said \\\"hi\\\".txt\"\r\n"),
            "quote must not break the quoted-string: {wire}"
        );
    }

    #[test]
    fn non_ascii_attachment_filename_uses_encoded_word() {
        let mut msg = base_message();
        msg.attachments = vec![attachment("résumé.pdf", "application/pdf", b"x")];

        let wire = build(&msg);

        assert!(
            headers_of(wire.as_bytes()).is_ascii(),
            "header block must stay ASCII"
        );
        assert!(wire.contains("filename=\"=?utf-8?q?r=C3=A9sum=C3=A9=2Epdf?=\"\r\n"));
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
    fn boundary_is_unique_and_delimits_exactly_n_plus_one_times() {
        let content = b"boundary collision bait: ==mailslate==";
        let mut msg = base_message();
        msg.body = "also bait: ==mailslate==".to_string();
        msg.attachments = vec![attachment("b.txt", "text/plain", content)];

        let wire_one = build(&msg);
        let wire_two = build(&msg);

        let b_one = extract_boundary(&wire_one);
        let b_two = extract_boundary(&wire_two);
        assert_ne!(b_one, b_two, "each build must stamp a fresh boundary");

        // Wire scan: one delimiter per part (2) plus the closing
        // delimiter — no more. A boundary occurring inside body or
        // attachment content would inflate this count.
        let delimiter = format!("--{b_one}");
        assert_eq!(
            wire_one.matches(&delimiter).count(),
            3,
            "boundary may not appear inside part content: {wire_one}"
        );
        assert!(wire_one.contains(&format!("{delimiter}--\r\n")));
    }

    #[test]
    fn multipart_orders_text_first_and_roundtrips_the_attachment() {
        use base64::Engine as _;

        let content: Vec<u8> = (0..75u8).collect();
        let mut msg = base_message();
        msg.body = "text part".to_string();
        msg.attachments = vec![attachment("bin.dat", "application/octet-stream", &content)];

        let wire = build(&msg);

        let text_pos = wire.find("Content-Type: text/plain").expect("text part");
        let att_pos = wire
            .find("Content-Type: application/octet-stream")
            .expect("attachment part");
        assert!(text_pos < att_pos, "text part must precede attachments");

        // 75 bytes -> 100 base64 chars: one 76-char line plus a
        // 24-char tail, and the pair decodes back to the input.
        let encoded = base64::engine::general_purpose::STANDARD.encode(&content);
        let (first, tail) = encoded.split_at(76);
        assert_eq!(tail.len(), 24);
        assert!(wire.contains(&format!("\r\n{first}\r\n{tail}\r\n")));
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(format!("{first}{tail}").as_bytes())
            .expect("valid base64");
        assert_eq!(decoded, content);
    }

    #[test]
    fn base64_of_57_bytes_stays_on_a_single_line() {
        use base64::Engine as _;

        let content: Vec<u8> = (0..57u8).collect();
        let mut msg = base_message();
        msg.attachments = vec![attachment("bin.dat", "application/octet-stream", &content)];

        let wire = build(&msg);

        let encoded = base64::engine::general_purpose::STANDARD.encode(&content);
        assert_eq!(encoded.len(), 76);
        assert!(wire.contains(&format!("\r\n{encoded}\r\n")));
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

    #[test]
    fn built_messages_parse_back_with_mail_parser() {
        let mut msg = base_message();
        msg.body = "Héllo wörld\r\nsecond line".to_string();
        let content: Vec<u8> = (1u8..=9).rev().collect();
        msg.attachments = vec![attachment("doc.bin", "application/octet-stream", &content)];

        let wire = build(&msg);
        let parsed = mail_parser::MessageParser::default()
            .parse(wire.as_bytes())
            .expect("built message must parse back");

        assert_eq!(
            parsed.body_text(0).expect("text body").trim_end(),
            "Héllo wörld\r\nsecond line"
        );
        let attachment = parsed.attachment(0).expect("attachment part");
        assert_eq!(attachment.contents(), content.as_slice());
    }

    #[test]
    fn formats_negative_epoch_as_1969() {
        assert_eq!(
            format_rfc5322_date(-86_400),
            "Wed, 31 Dec 1969 00:00:00 +0000"
        );
    }

    #[test]
    fn formats_leap_day_and_nonzero_clock() {
        assert_eq!(
            format_rfc5322_date(1_709_164_800),
            "Thu, 29 Feb 2024 00:00:00 +0000"
        );
        assert_eq!(
            format_rfc5322_date(86_399),
            "Thu, 01 Jan 1970 23:59:59 +0000"
        );
    }

    #[test]
    fn encoded_word_escapes_exactly() {
        assert_eq!(encoded_word(" "), "=?utf-8?q?_?=");
        assert_eq!(encoded_word("=?"), "=?utf-8?q?=3D=3F?=");
        assert_eq!(encoded_word("a_b"), "=?utf-8?q?a=5Fb?=");
        assert_eq!(
            encoded_word("Ünal Şimşek"),
            "=?utf-8?q?=C3=9Cnal_=C5=9Eim=C5=9Fek?="
        );
    }

    #[test]
    fn raw_name_safe_rejects_list_breakers() {
        assert!(raw_name_safe("O'Brien"));
        assert!(!raw_name_safe("Smith, John"), "comma splits address lists");
        assert!(!raw_name_safe("He said \"hi\""), "quote breaks the name");
        assert!(!raw_name_safe(""), "empty name must be encoded, not raw");
    }

    #[test]
    fn normalize_crlf_handles_edge_bodies() {
        assert_eq!(normalize_crlf(""), "\r\n");
        assert_eq!(normalize_crlf("done\r\n"), "done\r\n");
        assert_eq!(normalize_crlf("a\nb\r\nc"), "a\r\nb\r\nc\r\n");
    }

    #[test]
    fn build_error_display_strings() {
        assert_eq!(
            BuildError::HeaderInjection { field: "subject" }.to_string(),
            "header injection blocked in subject"
        );
        assert_eq!(
            BuildError::NoRecipient.to_string(),
            "message has no recipient"
        );
    }

    #[test]
    fn qp_trailing_spaces_never_break_the_76_char_cap() {
        // Round-2 finding: a space admitted near the 75-char budget
        // re-inflates to =20 at flush time (and multi-space runs
        // only had their last space encoded). Pin exact wires.
        let cases = [
            // é (6 chars) + 68 literals ends at the cap; the space
            // must soft-wrap BEFORE it is admitted.
            (
                format!("{}{}", "é", "a".repeat(68)) + " \nb",
                format!("=C3=A9{}=\r\n=20\r\nb\r\n", "a".repeat(68)),
            ),
            // The 76-char boundary itself stays legal.
            (
                "a".repeat(72) + "é \nb",
                format!("{}=C3=\r\n=A9=20\r\nb\r\n", "a".repeat(72)),
            ),
            // A space run expands every trailing space, never
            // leaving a raw space before a break (ü keeps this
            // body on the quoted-printable path).
            (
                "a".repeat(70) + "   \nb ü",
                format!("{}=20=\r\n=20=20\r\nb =C3=BC\r\n", "a".repeat(70)),
            ),
        ];
        for (body, expected_body) in cases {
            let mut msg = base_message();
            msg.body = body;

            let wire = build(&msg);
            let (_, wire_body) = wire.split_once("\r\n\r\n").expect("body");
            assert_eq!(wire_body, expected_body);
            for line in wire.split("\r\n") {
                assert!(
                    line.len() <= 76,
                    "wire line is {} chars (limit 76): {line:?}",
                    line.len()
                );
            }
        }
    }

    #[test]
    fn normalizes_bare_cr_to_crlf() {
        let mut msg = base_message();
        msg.body = "a\rb ü".to_string();

        let wire = build(&msg);

        // Lone CR settles to a hard CRLF break in the 7bit view,
        // and the QP path (forced by ü) must not swallow it.
        assert!(wire.contains("Content-Transfer-Encoding: quoted-printable\r\n"));
        assert!(wire.contains("a\r\nb =C3=BC\r\n"), "{wire}");
    }

    #[test]
    fn subject_control_bytes_and_lookalike_encoded_words_are_encoded() {
        let mut msg = base_message();
        msg.subject = "\u{1}flag".to_string();
        assert!(
            build(&msg).contains("Subject: =?utf-8?q?=01flag?=\r\n"),
            "control bytes must not ship raw in headers"
        );

        let mut lookalike = base_message();
        lookalike.subject = "a =?utf-8?q?x?= b".to_string();
        let lookalike_wire = build(&lookalike);
        assert!(
            lookalike_wire.contains("Subject: =?utf-8?q?a_=3D=3Futf-8=3Fq=3Fx=3F=3D_b?=\r\n"),
            "an encoded-word lookalike must not self-decode in MUAs; actual: {lookalike_wire}"
        );
    }

    #[test]
    fn overlong_ascii_lines_force_quoted_printable() {
        let mut msg = base_message();
        msg.body = "a".repeat(1_200);

        let wire = build(&msg);

        assert!(wire.contains("Content-Transfer-Encoding: quoted-printable\r\n"));
        for line in wire.split("\r\n") {
            assert!(line.len() <= 76, "line {} chars", line.len());
        }
    }

    #[test]
    fn empty_display_name_renders_bare_address() {
        let mut msg = base_message();
        msg.to = vec![addr(Some(""), "bare@other.org")];

        let wire = build(&msg);

        assert!(wire.contains("To: bare@other.org\r\n"), "{wire}");
        assert!(!wire.contains("=?utf-8?q??="));

        // Whitespace-only names are just as empty: an obs-phrase of
        // blanks must degrade to the bare address too.
        let mut spaces = base_message();
        spaces.to = vec![addr(Some("   "), "bare@other.org")];

        let spaces_wire = build(&spaces);

        assert!(
            spaces_wire.contains("To: bare@other.org\r\n"),
            "{spaces_wire}"
        );
        assert!(!spaces_wire.contains("<bare@other.org>"));
    }

    #[test]
    fn quoted_string_escapes_backslash_then_quote() {
        let mut msg = base_message();
        msg.attachments = vec![attachment("a\\b.txt", "text/plain", b"x")];

        let wire = build(&msg);

        assert!(wire.contains("filename=\"a\\\\b.txt\"\r\n"), "{wire}");

        // Combined sequence pins the replacement ORDER: the
        // backslash must double before the quote is escaped.
        let mut combo = base_message();
        combo.attachments = vec![attachment("a\\\"b.txt", "text/plain", b"x")];

        let combo_wire = build(&combo);

        assert!(
            combo_wire.contains(r##"filename="a\\\"b.txt""##),
            "{combo_wire}"
        );
    }

    #[test]
    fn seven_bit_boundary_pins_the_998_char_line_limit() {
        let mut legal = base_message();
        legal.body = "a".repeat(998);
        assert!(build(&legal).contains("Content-Transfer-Encoding: 7bit\r\n"));

        let mut over = base_message();
        over.body = "a".repeat(999);
        assert!(build(&over).contains("Content-Transfer-Encoding: quoted-printable\r\n"));
    }

    #[test]
    fn two_attachments_each_get_a_part() {
        let first: Vec<u8> = vec![1, 2, 3];
        let second: Vec<u8> = vec![4, 5, 6, 7];
        let mut msg = base_message();
        msg.attachments = vec![
            attachment("one.bin", "application/octet-stream", &first),
            attachment("two.bin", "application/octet-stream", &second),
        ];

        let wire = build(&msg);
        let boundary = extract_boundary(&wire);

        assert_eq!(wire.matches(&format!("--{boundary}")).count(), 4);
        assert!(wire.contains("filename=\"one.bin\"\r\n"));
        assert!(wire.contains("filename=\"two.bin\"\r\n"));

        let parsed = mail_parser::MessageParser::default()
            .parse(wire.as_bytes())
            .expect("parses");
        assert_eq!(
            parsed.attachment(0).expect("first").contents(),
            first.as_slice()
        );
        assert_eq!(
            parsed.attachment(1).expect("second").contents(),
            second.as_slice()
        );
    }

    #[test]
    fn base64_wraps_three_times_beyond_two_lines() {
        use base64::Engine as _;

        let content: Vec<u8> = (0..200u8).collect();
        let mut msg = base_message();
        msg.attachments = vec![attachment("bin.dat", "application/octet-stream", &content)];

        let wire = build(&msg);

        let encoded = base64::engine::general_purpose::STANDARD.encode(&content);
        assert_eq!(encoded.len(), 268);
        let expected = format!(
            "\r\n{}\r\n{}\r\n{}\r\n{}\r\n",
            &encoded[..76],
            &encoded[76..152],
            &encoded[152..228],
            &encoded[228..]
        );
        assert!(wire.contains(&expected));
    }

    #[test]
    fn tab_survives_7bit_and_encodes_under_qp() {
        let mut seven_bit = base_message();
        seven_bit.body = "k\tv".to_string();
        assert!(build(&seven_bit).contains("Content-Transfer-Encoding: 7bit\r\n"));
        assert!(build(&seven_bit).contains("\r\n\r\nk\tv\r\n"));

        let mut quoted = base_message();
        quoted.body = "k\tv ü".to_string();
        assert!(build(&quoted).contains("k=09v =C3=BC\r\n"));
    }

    #[test]
    fn non_ascii_filename_roundtrips_through_mail_parser() {
        use mail_parser::MimeHeaders;

        let content = b"roundtrip payload";
        let mut msg = base_message();
        msg.attachments = vec![attachment("résumé.pdf", "application/pdf", content)];

        let wire = build(&msg);
        let parsed = mail_parser::MessageParser::default()
            .parse(wire.as_bytes())
            .expect("parses");

        let attachment = parsed.attachment(0).expect("attachment part");
        assert_eq!(attachment.contents(), content.as_slice());
        // RFC 2047-in-quoted-string is the de-facto filename
        // convention (RFC 2231 emission is parked in MAILSLATE-79);
        // pin whatever the parser resolves so drift is visible.
        let filename = attachment
            .attachment_name()
            .expect("filename param decodes");
        assert!(
            filename == "résumé.pdf" || filename == "=?utf-8?q?r=C3=A9sum=C3=A9=2Epdf?=",
            "unexpected filename resolution: {filename}"
        );
    }
}
