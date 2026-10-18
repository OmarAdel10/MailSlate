//! OAuth2 provider plumbing for Gmail and Microsoft (ARCHITECTURE §8).
//!
//! This module owns the provider table (endpoint URLs, scopes, and the
//! IMAP/SMTP defaults a new account is seeded with), the PKCE S256 +
//! state nonce pair, the RFC 8252 loopback redirect listener, and the
//! token exchange/refresh client. Tokens are persisted only through the
//! daemon-side [`crate::infra::keyring::KeyStore`].

pub mod authorize;
pub mod grant;
pub mod loopback;
pub mod pkce;
pub mod providers;
pub mod tokens;

/// Errors surfaced by the OAuth plumbing.
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    /// The build/push environment did not configure this provider's
    /// public client id.
    #[error("provider {0} is not configured (missing public client id)")]
    NotConfigured(&'static str),
    /// The loopback listener was opened but the user never completed
    /// the browser flow in time.
    #[error("oauth flow timed out before the browser callback arrived")]
    ListenerTimeout,
    /// The redirect carried a state that does not match the nonce we
    /// minted (CSRF guard; possible forged callback).
    #[error("oauth callback state does not match the issued nonce")]
    StateMismatch,
    /// The provider rejected a code/token exchange or a refresh grant
    /// (bad code, revoked consent, expired refresh token).
    #[error("provider rejected the token request: {0}")]
    InvalidGrant(String),
    /// Any other transport/HTTP failure talking to the provider.
    #[error("oauth transport failure: {0}")]
    Network(String),
}
