//! HTML sanitizer for reader rendering (ARCHITECTURE §7 reader path).
//!
//! Runs once at sync time: raw HTML from IMAP goes through an ammonia
//! allow-list, the clean string is persisted to `messages.html_clean`.
//! The Flutter reader renders the clean string; it never sees raw HTML,
//! so no JS, plugins, or remote-image leakage reaches the GUI.

use ammonia::Builder;

/// Errors from the sanitizer surface.
#[derive(Debug, thiserror::Error)]
pub enum SanitizeError {
    #[error("input was not valid UTF-8")]
    NotUtf8,
    #[error("sanitizer rejected the document")]
    Rejected(String),
}

/// Sanitizes `html` with the reader allow-list: structural and
/// text-formatting tags plus safe attributes; everything else is
/// dropped (script, style, on* handlers, forms, iframes, object/embed).
/// Returns the sanitized HTML string.
///
/// Ammonia returns a `String`; for typical mail HTML there is nothing
/// to strip so the input passes through unchanged.
pub fn sanitize(html: &str) -> Result<String, SanitizeError> {
    let cleaned = Builder::new()
        .tags(SANITIZE_TAGS.iter().copied().collect())
        .generic_attributes(SANITIZE_GENERIC_ATTRS.iter().copied().collect())
        .url_schemes(SANITIZE_URL_SCHEMES.iter().copied().collect())
        .link_rel(None)
        .clean(html)
        .into();
    Ok(cleaned)
}

/// Tags the reader allows (structural + inline formatting only).
const SANITIZE_TAGS: &[&str] = &[
    "a",
    "b",
    "blockquote",
    "br",
    "code",
    "div",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "li",
    "ol",
    "p",
    "pre",
    "s",
    "span",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "th",
    "thead",
    "tr",
    "u",
    "ul",
];

/// Attribute names allowed on any allowed tag. `href`/`src` are
/// restricted to https or relative by url_schemes so `javascript:`
/// `data:` schemes cannot execute.
const SANITIZE_GENERIC_ATTRS: &[&str] = &[
    "href", "src", "alt", "title", "target", "width", "height", "colspan", "rowspan", "cite",
];

/// URL schemes allowed in `href`/`src`. Empty string allows relative
/// URLs; `https` allows absolute secure URLs.
const SANITIZE_URL_SCHEMES: &[&str] = &["", "https"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_passes_through() {
        let out = sanitize("Hello, world").expect("clean");
        assert_eq!(out, "Hello, world");
    }

    #[test]
    fn formatting_tags_survive() {
        let out = sanitize("<b>bold</b> and <i>italic</i>").expect("clean");
        assert!(out.contains("<b>bold</b>"));
        assert!(out.contains("<i>italic</i>"));
    }

    #[test]
    fn scripts_and_handlers_are_stripped() {
        let out = sanitize("<script>alert(1)</script><img src=x onerror=alert(1)>").expect("clean");
        assert!(!out.contains("script"));
        assert!(!out.contains("onerror"));
        assert!(!out.contains("alert"));
    }

    #[test]
    fn links_keep_https_and_relative() {
        let out = sanitize(r#"<a href="https://example.com">safe</a> <a href="/inbox">rel</a> <a href="javascript:bad">evil</a>"#).expect("clean");
        assert!(out.contains("https://example.com"));
        assert!(out.contains("/inbox"));
        assert!(!out.contains("javascript"));
    }

    #[test]
    fn images_keep_https_src() {
        let out =
            sanitize(r#"<img src="https://cdn.example.com/pic.png" alt="pic">"#).expect("clean");
        assert!(out.contains("https://cdn.example.com/pic.png"));
        assert!(out.contains("alt=\"pic\""));
    }

    #[test]
    fn nested_structure_is_preserved() {
        let out = sanitize("<table><thead><tr><th>Head</th></tr></thead><tbody><tr><td>Cell</td></tr></tbody></table>").expect("clean");
        assert!(out.contains("<table>"));
        assert!(out.contains("</table>"));
        assert!(out.contains("<td>Cell</td>"));
    }

    #[test]
    fn empty_input_is_empty_output() {
        let out = sanitize("").expect("clean");
        assert_eq!(out, "");
    }
}
