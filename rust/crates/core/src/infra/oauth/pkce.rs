//! PKCE (RFC 7636) primitives: verifier generation, S256 challenge
//! derivation, and the `state` nonce that ties a callback to a
//! pending authorization.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Generates a fresh code verifier: 96 unreserved hex characters
/// (three UUID v4s, ~366 bits of entropy), inside the RFC 7636 §4.1
/// range of 43..=128 — without pulling in a separate `rand` crate.
pub fn new_verifier() -> String {
    let mut verifier = String::with_capacity(96);
    for _ in 0..3 {
        verifier.push_str(&Uuid::new_v4().simple().to_string());
    }
    verifier
}

/// Derives the S256 challenge for a verifier:
/// BASE64URL-unpadded(SHA256(verifier)) (RFC 7636 §4.2).
pub fn challenge_for(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Generates the `state` nonce tying the loopback callback to this
/// authorization attempt.
pub fn new_state() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7636 appendix B: the S256 challenge of the sample verifier
    /// is a pinned vector — any drift in encoding (padding, alphabet,
    /// digest) fails here.
    #[test]
    fn challenge_matches_rfc7636_appendix_b_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = challenge_for(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    /// RFC 7636 §4.1: the verifier is 43..=128 unreserved characters.
    #[test]
    fn verifier_has_length_between_43_and_128() {
        for _ in 0..50 {
            let verifier = new_verifier();
            assert!(
                (43..=128).contains(&verifier.len()),
                "verifier length {} out of range",
                verifier.len()
            );
        }
    }

    #[test]
    fn verifier_uses_only_unreserved_characters() {
        let verifier = new_verifier();
        assert!(
            verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')),
            "verifier must stick to the unreserved set: {verifier}"
        );
    }

    /// `state` is a UUID v4 string: 36 chars with the version nibble
    /// set, so a callback carrying anything else is rejectable.
    #[test]
    fn state_is_a_uuid_v4_string() {
        let state = new_state();
        assert_eq!(state.len(), 36);
        assert_eq!(state.as_bytes()[14], b'4', "version nibble must be 4");
    }

    #[test]
    fn verifiers_are_unique_across_calls() {
        let first = new_verifier();
        let second = new_verifier();
        assert_ne!(first, second);
    }
}
