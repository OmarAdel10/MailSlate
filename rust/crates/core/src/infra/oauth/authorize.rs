//! Builds the provider consent-screen URL with PKCE S256 parameters.

use crate::infra::oauth::providers::ProviderProfile;

/// Builds the consent-screen URL: `response_type=code`, the PKCE S256
/// challenge, the loopback redirect, and the provider's scope. The
/// URL builder percent-encodes the scope's spaces for the multi-scope
/// providers.
pub fn authorize_url(
    profile: &ProviderProfile,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    let url = reqwest::Url::parse_with_params(
        profile.auth_url,
        [
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("state", state),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
            ("scope", profile.scope),
        ],
    )
    .expect("profile auth_url is a valid url");
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::oauth::providers::{Provider, profile};

    #[test]
    fn authorize_url_carries_pkce_and_scope_parameters() {
        let url = authorize_url(
            profile(Provider::Google),
            "client-id",
            "http://127.0.0.1:53123/callback",
            "st-1",
            "challenge-1",
        );

        let parsed = reqwest::Url::parse(&url).expect("built url parses");
        assert!(
            parsed
                .as_str()
                .starts_with("https://accounts.google.com/o/oauth2/v2/auth")
        );
        let pairs: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();
        let get = |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| panic!("missing {k}"))
        };
        assert_eq!(get("response_type"), "code");
        assert_eq!(get("client_id"), "client-id");
        assert_eq!(get("redirect_uri"), "http://127.0.0.1:53123/callback");
        assert_eq!(get("state"), "st-1");
        assert_eq!(get("code_challenge"), "challenge-1");
        assert_eq!(get("code_challenge_method"), "S256");
        assert_eq!(get("scope"), "https://mail.google.com/");
    }

    #[test]
    fn microsoft_scope_survives_round_trip_with_all_scopes() {
        let url = authorize_url(
            profile(Provider::Microsoft),
            "ms-id",
            "http://127.0.0.1:53124/callback",
            "st-2",
            "challenge-2",
        );
        let parsed = reqwest::Url::parse(&url).expect("built url parses");
        let scope = parsed
            .query_pairs()
            .find(|(k, _)| k == "scope")
            .map(|(_, v)| v.into_owned())
            .expect("scope present");
        assert_eq!(scope, "offline_access IMAP.AccessAsUser.All SMTP.Send");
        // Spaces must not survive raw in the serialized URL.
        assert!(url.contains("scope=offline_access%20") || url.contains("scope=offline_access+"));
    }
}
