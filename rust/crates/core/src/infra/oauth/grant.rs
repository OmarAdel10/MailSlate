//! Token grant flows against a provider's token endpoint:
//! authorization-code + PKCE exchange, and refresh. The HTTP boundary
//! is injectable so no test ever touches the network.

use crate::infra::oauth::OAuthError;
use crate::infra::oauth::providers::{self, Provider};
use crate::infra::oauth::tokens::TokenSnapshot;

/// What a provider's token endpoint answers with, normalized from its
/// JSON body. `refresh_token` is optional because some providers only
/// rotate it occasionally.
#[derive(Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub scope: Option<String>,
    pub id_token: Option<String>,
}

/// The one HTTP call the OAuth dance needs. Real implementation talks
/// `reqwest`; tests substitute a fake — no test ever hits the network.
pub trait TokenEndpoint {
    fn post(
        &self,
        token_url: &str,
        form: &[(&str, &str)],
    ) -> impl Future<Output = Result<TokenResponse, OAuthError>> + Send;
}

use std::future::Future;

/// Production endpoint: POSTs the form and normalizes the JSON reply.
/// HTTP error bodies carrying `{"error": "invalid_grant"}` (or any
/// other provider error) surface as [`OAuthError::InvalidGrant`] or
/// [`OAuthError::Network`].
pub struct HttpTokenEndpoint;

/// Shared HTTP client for token endpoints. Bounded to 15 s per
/// request (QA round 1): a stuck token endpoint must not park the
/// sync actor's refresh path forever.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("reqwest client builds")
    })
}

impl TokenEndpoint for HttpTokenEndpoint {
    async fn post(
        &self,
        token_url: &str,
        form: &[(&str, &str)],
    ) -> Result<TokenResponse, OAuthError> {
        let client = http_client();
        let response = client
            .post(token_url)
            .form(form)
            .send()
            .await
            .map_err(|err| OAuthError::Network(err.to_string()))?;
        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|err| OAuthError::Network(format!("{status}: {err}")))?;
        if !status.is_success() {
            let kind = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("http_error");
            let detail = body
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if kind == "invalid_grant" {
                return Err(OAuthError::InvalidGrant(detail.to_string()));
            }
            return Err(OAuthError::Network(format!("{status}: {kind} {detail}")));
        }
        parse_token_response(&body)
    }
}

/// Normalizes a provider JSON body into a [`TokenResponse`].
fn parse_token_response(body: &serde_json::Value) -> Result<TokenResponse, OAuthError> {
    let obj = body
        .as_object()
        .ok_or_else(|| OAuthError::Network("token reply is not a JSON object".into()))?;
    if let Some(kind) = obj.get("error").and_then(|v| v.as_str()) {
        let detail = obj
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Err(OAuthError::InvalidGrant(format!("{kind} {detail}")));
    }
    Ok(TokenResponse {
        access_token: obj
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| OAuthError::Network("token reply lacks access_token".into()))?
            .to_string(),
        refresh_token: obj
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(Into::into),
        expires_in: obj
            .get("expires_in")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| OAuthError::Network("token reply lacks expires_in".into()))?,
        scope: obj.get("scope").and_then(|v| v.as_str()).map(Into::into),
        id_token: obj.get("id_token").and_then(|v| v.as_str()).map(Into::into),
    })
}

/// Extracts the account email from the provider's id token (a JWT):
/// decodes the payload segment and prefers the `email` claim, falling
/// back to Microsoft's `preferred_username`. A missing or unreadable
/// claim yields `None` — the GUI then asks for the address.
pub fn id_token_email(id_token: &str) -> Option<String> {
    use base64::Engine as _;
    let payload = id_token.split('.').nth(1)?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    claims
        .get("email")
        .or_else(|| claims.get("preferred_username"))
        .and_then(|v| v.as_str())
        .map(Into::into)
}

/// Exchanges an authorization code (plus PKCE verifier) for tokens at
/// the provider's token endpoint. `now` seeds the expiry clock.
pub async fn exchange_code<E: TokenPoster + ?Sized>(
    endpoint: &E,
    provider: Provider,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    client_id: &str,
    now: i64,
) -> Result<TokenSnapshot, OAuthError> {
    exchange_code_detail(
        endpoint,
        provider,
        code,
        code_verifier,
        redirect_uri,
        client_id,
        now,
    )
    .await
    .map(|(snapshot, _)| snapshot)
}

/// [`exchange_code`] that also surfaces the id_token, so the caller
/// can extract the account email without a userinfo round-trip.
pub async fn exchange_code_detail<E: TokenPoster + ?Sized>(
    endpoint: &E,
    provider: Provider,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
    client_id: &str,
    now: i64,
) -> Result<(TokenSnapshot, Option<String>), OAuthError> {
    let profile = providers::profile(provider);
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
    ];
    let response = endpoint.post(profile.token_url, &form).await?;
    let id_token = response.id_token.as_deref().and_then(id_token_email);
    let snapshot = to_snapshot(response, now, None);
    if snapshot.refresh_token.is_empty() {
        // Without a refresh token the account dies after the first hour;
        // surface it now, while the user is still in the wizard.
        return Err(OAuthError::InvalidGrant(
            "provider returned no refresh token; the consent screen was skipped".into(),
        ));
    }
    Ok((snapshot, id_token))
}

/// Object-safe stand-in for [`TokenEndpoint`]: AFIT makes the original
/// trait `dyn`-incompatible, and shared daemon state must hold the
/// endpoint behind a pointer so tests can inject a fake.
pub trait TokenPoster: Send + Sync {
    fn post<'a>(
        &'a self,
        token_url: &'a str,
        form: &'a [(&'a str, &'a str)],
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<TokenResponse, OAuthError>> + Send + 'a>>;
}

impl<T: TokenEndpoint + Send + Sync + ?Sized> TokenPoster for T {
    fn post<'a>(
        &'a self,
        token_url: &'a str,
        form: &'a [(&'a str, &'a str)],
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<TokenResponse, OAuthError>> + Send + 'a>>
    {
        Box::pin(TokenEndpoint::post(self, token_url, form))
    }
}

/// Refreshes an access token with a refresh token grant. When the
/// provider does not rotate the refresh token, the old one stays in
/// the snapshot.
pub async fn refresh<E: TokenPoster + ?Sized>(
    endpoint: &E,
    provider: Provider,
    refresh_token: &str,
    client_id: &str,
    now: i64,
) -> Result<TokenSnapshot, OAuthError> {
    let profile = providers::profile(provider);
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];
    let response = endpoint.post(profile.token_url, &form).await?;
    Ok(to_snapshot(response, now, Some(refresh_token)))
}

/// Builds the persisted snapshot: expiry = now + expires_in, and the
/// fallback refresh token wins when the provider skipped rotation.
fn to_snapshot(response: TokenResponse, now: i64, fallback_refresh: Option<&str>) -> TokenSnapshot {
    TokenSnapshot {
        access_token: response.access_token,
        refresh_token: response
            .refresh_token
            .or_else(|| fallback_refresh.map(Into::into))
            .unwrap_or_default(),
        expires_at: now + response.expires_in,
        scopes: response.scope.unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::oauth::providers::Provider;
    use crate::infra::oauth::providers::profile;
    use std::sync::Mutex;

    const GOOGLE: Provider = Provider::Google;

    fn canned(access: &str, refresh: Option<&str>, expires_in: i64) -> TokenResponse {
        TokenResponse {
            access_token: access.into(),
            refresh_token: refresh.map(Into::into),
            expires_in,
            scope: Some("https://mail.google.com/".into()),
            id_token: None,
        }
    }

    struct FakeEndpoint {
        response: TokenResponse,
        calls: Mutex<Vec<Vec<(String, String)>>>,
    }

    impl FakeEndpoint {
        fn new(response: TokenResponse) -> Self {
            Self {
                response,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn last_form(&self) -> Vec<(String, String)> {
            self.calls
                .lock()
                .expect("fake calls lock")
                .last()
                .expect("a call happened")
                .clone()
        }
    }

    impl TokenEndpoint for FakeEndpoint {
        async fn post(
            &self,
            _token_url: &str,
            form: &[(&str, &str)],
        ) -> Result<TokenResponse, OAuthError> {
            self.calls.lock().expect("fake calls lock").push(
                form.iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
            );
            Ok(self.response.clone())
        }
    }

    fn form_value<'a>(form: &'a [(String, String)], key: &str) -> &'a str {
        &form
            .iter()
            .find(|(k, _)| k == key)
            .expect("form field present")
            .1
    }

    #[tokio::test]
    async fn exchange_code_posts_pkce_form_and_builds_snapshot() {
        let fake = FakeEndpoint::new(canned("at-1", Some("rt-1"), 3600));
        let snapshot = exchange_code(
            &fake,
            GOOGLE,
            "auth-code",
            "verifier-43x",
            "http://127.0.0.1:1/callback",
            "client-id",
            1_000,
        )
        .await
        .expect("exchange");

        let form = fake.last_form();
        assert_eq!(form_value(&form, "grant_type"), "authorization_code");
        assert_eq!(form_value(&form, "code"), "auth-code");
        assert_eq!(form_value(&form, "client_id"), "client-id");
        assert_eq!(
            form_value(&form, "redirect_uri"),
            "http://127.0.0.1:1/callback"
        );
        assert_eq!(form_value(&form, "code_verifier"), "verifier-43x");

        assert_eq!(snapshot.access_token, "at-1");
        assert_eq!(snapshot.refresh_token, "rt-1");
        assert_eq!(snapshot.expires_at, 1_000 + 3600);
        assert_eq!(snapshot.scopes, "https://mail.google.com/");
        assert!(profile(GOOGLE).token_url.contains("oauth2.googleapis.com"));
    }

    fn b64url(raw: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
    }

    #[test]
    fn id_token_email_extracts_the_email_claim() {
        let jwt = format!(
            "{}.{}.{}",
            b64url(br#"{"alg":"RS256"}"#),
            b64url(br#"{"email":"ada@example.com"}"#),
            b64url(b"sig"),
        );
        assert_eq!(id_token_email(&jwt).as_deref(), Some("ada@example.com"));
    }

    #[test]
    fn id_token_email_falls_back_to_preferred_username() {
        let jwt = format!(
            "{}.{}.{}",
            b64url(br#"{"alg":"RS256"}"#),
            b64url(br#"{"preferred_username":"ada@outlook.com"}"#),
            b64url(b"sig"),
        );
        assert_eq!(id_token_email(&jwt).as_deref(), Some("ada@outlook.com"));
    }

    #[test]
    fn id_token_email_is_none_for_garbage() {
        assert!(id_token_email("not-a-jwt").is_none());
        assert!(id_token_email("a.b.c").is_none());
    }

    #[tokio::test]
    async fn refresh_posts_refresh_grant_and_keeps_unrotated_token() {
        let fake = FakeEndpoint::new(canned("at-2", None, 1800));
        let snapshot = refresh(&fake, GOOGLE, "rt-old", "client-id", 2_000)
            .await
            .expect("refresh");

        let form = fake.last_form();
        assert_eq!(form_value(&form, "grant_type"), "refresh_token");
        assert_eq!(form_value(&form, "refresh_token"), "rt-old");
        assert_eq!(form_value(&form, "client_id"), "client-id");

        // Provider did not rotate: the old refresh token stays valid.
        assert_eq!(snapshot.refresh_token, "rt-old");
        assert_eq!(snapshot.access_token, "at-2");
        assert_eq!(snapshot.expires_at, 2_000 + 1800);
    }

    #[tokio::test]
    async fn exchange_code_rejects_a_grant_without_a_refresh_token() {
        let fake = FakeEndpoint::new(canned("at-3", None, 3600));
        let err = exchange_code(
            &fake,
            GOOGLE,
            "auth-code",
            "verifier",
            "http://127.0.0.1:1/callback",
            "client-id",
            1_000,
        )
        .await
        .expect_err("a grant without a refresh token is unusable");
        assert!(matches!(
            err,
            OAuthError::InvalidGrant(ref message) if message.contains("refresh token")
        ));
    }

    #[test]
    fn parse_token_response_maps_provider_error_bodies() {
        let body = serde_json::json!({
            "error": "invalid_grant",
            "error_description": "token has been revoked",
        });
        let err = parse_token_response(&body).expect_err("error body is not a token");
        assert!(matches!(
            err,
            OAuthError::InvalidGrant(ref message) if message.contains("revoked")
        ));
    }

    #[test]
    fn parse_token_response_requires_object_and_token_fields() {
        let not_an_object = parse_token_response(&serde_json::json!([])).expect_err("array body");
        assert!(matches!(not_an_object, OAuthError::Network(_)));

        let no_access = parse_token_response(&serde_json::json!({})).expect_err("empty body");
        assert!(matches!(
            no_access,
            OAuthError::Network(ref message) if message.contains("access_token")
        ));

        let no_expiry = parse_token_response(&serde_json::json!({"access_token": "at"}))
            .expect_err("missing expiry");
        assert!(matches!(
            no_expiry,
            OAuthError::Network(ref message) if message.contains("expires_in")
        ));
    }
}
