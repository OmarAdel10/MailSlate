//! SASL XOAUTH2 string construction (RFC 7628 / Google XOAUTH2).
//!
//! The client response for `AUTHENTICATE XOAUTH2` is the base64 of
//! `user=<user>\x01auth=Bearer <token>\x01\x01`; the two trailing NULs
//! are part of the format and must not be trimmed.

/// Builds the XOAUTH2 client response for `email` authenticated by the
/// OAuth access `token`. Callers base64-encode the result for the wire.
pub fn xoauth2_string(email: &str, token: &str) -> String {
    format!("user={email}\u{1}auth=Bearer {token}\u{1}\u{1}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xoauth2_string_matches_the_sasl_format() {
        let built = xoauth2_string("me@gmail.com", "tok-1");
        assert_eq!(built, "user=me@gmail.com\u{1}auth=Bearer tok-1\u{1}\u{1}");
    }

    #[test]
    fn xoauth2_string_escapes_nothing_but_includes_both_separators() {
        let built = xoauth2_string("user@outlook.office365.com", "EwA=");
        assert_eq!(
            built,
            "user=user@outlook.office365.com\u{1}auth=Bearer EwA=\u{1}\u{1}"
        );
    }
}
