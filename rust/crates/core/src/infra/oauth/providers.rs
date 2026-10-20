//! The provider table: Google and Microsoft OAuth2 + mail endpoints.

use crate::domain::accounts::{AuthKind, Security};

use super::OAuthError;

/// Which OAuth2 identity provider an account uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Google,
    Microsoft,
}

impl Provider {
    /// Environment variable carrying the PUBLIC client id (never a
    /// secret; public clients register a redirect to loopback only).
    pub fn client_id_env(self) -> &'static str {
        match self {
            Provider::Google => "MAILSLATE_GOOGLE_CLIENT_ID",
            Provider::Microsoft => "MAILSLATE_MICROSOFT_CLIENT_ID",
        }
    }

    /// Human-facing name used in errors and events.
    pub fn name(self) -> &'static str {
        match self {
            Provider::Google => "google",
            Provider::Microsoft => "microsoft",
        }
    }

    /// Wire identity stored in the `accounts.provider` column; the
    /// schema CHECK constrains values to `gmail`/`outlook`/`imap`,
    /// which differ from the OAuth wire names above.
    pub fn db_name(self) -> &'static str {
        match self {
            Provider::Google => "gmail",
            Provider::Microsoft => "outlook",
        }
    }
}

/// Serializes client-id env mutation across ALL core tests (providers
/// tests and the sync-actor refresh test share MAILSLATE_*_CLIENT_ID).
#[cfg(test)]
pub(crate) static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Everything the OAuth flow and the account wizard need to know about
/// a provider (ARCHITECTURE §8 provider table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderProfile {
    pub auth_url: &'static str,
    pub token_url: &'static str,
    /// Space-separated OAuth scopes requested at the authorize step.
    pub scope: &'static str,
    /// Google refresh tokens require `access_type=offline` on the
    /// authorize URL; Microsoft covers offline access via its scope.
    pub access_type_offline: bool,
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub imap_security: Security,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    pub smtp_security: Security,
    pub auth_kind: AuthKind,
}

/// Static table lookup; the set is closed (only gmail + outlook today).
pub fn profile(provider: Provider) -> &'static ProviderProfile {
    const GOOGLE: ProviderProfile = ProviderProfile {
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        scope: "https://mail.google.com/",
        access_type_offline: true,
        imap_host: "imap.gmail.com",
        imap_port: 993,
        imap_security: Security::Tls,
        smtp_host: "smtp.gmail.com",
        smtp_port: 587,
        smtp_security: Security::StartTls,
        auth_kind: AuthKind::OAuth2,
    };
    const MICROSOFT: ProviderProfile = ProviderProfile {
        auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
        token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        scope: "offline_access IMAP.AccessAsUser.All SMTP.Send",
        access_type_offline: false,
        imap_host: "outlook.office365.com",
        imap_port: 993,
        imap_security: Security::Tls,
        smtp_host: "smtp.office365.com",
        smtp_port: 587,
        smtp_security: Security::StartTls,
        auth_kind: AuthKind::OAuth2,
    };
    match provider {
        Provider::Google => &GOOGLE,
        Provider::Microsoft => &MICROSOFT,
    }
}

/// Reads the provider's PUBLIC client id from the environment. A missing
/// id means the build/push environment did not configure this provider;
/// that is a configuration error, never a panic (PRD §11 open question).
pub fn client_id(provider: Provider) -> Result<String, OAuthError> {
    let Some(id) = std::env::var(provider.client_id_env())
        .ok()
        .filter(|id| !id.trim().is_empty())
    else {
        return Err(OAuthError::NotConfigured(provider.name()));
    };
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::{AuthKind, Security};

    /// Profiles expose the endpoints and scopes from ARCHITECTURE §8.
    #[test]
    fn google_profile_matches_spec() {
        let p = profile(Provider::Google);
        assert_eq!(p.auth_url, "https://accounts.google.com/o/oauth2/v2/auth");
        assert_eq!(p.token_url, "https://oauth2.googleapis.com/token");
        assert_eq!(p.scope, "https://mail.google.com/");
        assert!(
            p.access_type_offline,
            "Google grants refresh tokens only with access_type=offline"
        );
        assert_eq!(p.imap_host, "imap.gmail.com");
        assert_eq!(p.imap_port, 993);
        assert_eq!(p.imap_security, Security::Tls);
        assert_eq!(p.smtp_host, "smtp.gmail.com");
        assert_eq!(p.smtp_port, 587);
        assert_eq!(p.smtp_security, Security::StartTls);
        assert_eq!(p.auth_kind, AuthKind::OAuth2);
    }

    #[test]
    fn microsoft_profile_matches_spec() {
        let p = profile(Provider::Microsoft);
        assert_eq!(
            p.auth_url,
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
        );
        assert_eq!(
            p.token_url,
            "https://login.microsoftonline.com/common/oauth2/v2.0/token"
        );
        assert_eq!(p.scope, "offline_access IMAP.AccessAsUser.All SMTP.Send");
        assert!(
            !p.access_type_offline,
            "Microsoft requests offline access via the scope, not access_type"
        );
        assert_eq!(p.imap_host, "outlook.office365.com");
        assert_eq!(p.imap_port, 993);
        assert_eq!(p.imap_security, Security::Tls);
        assert_eq!(p.smtp_host, "smtp.office365.com");
        assert_eq!(p.smtp_port, 587);
        assert_eq!(p.smtp_security, Security::StartTls);
    }

    /// Client ids come from build/push env; missing id is a config error,
    /// never a panic.
    #[test]
    fn client_id_reads_the_provider_env_var() {
        // These keys are disjoint from the ipc.rs env-mutating tests
        // (MAILSLATE_DAEMON_PATH/XDG_RUNTIME_DIR), so no cross-module
        // serialization is needed beyond this fn. set_var/remove_var are
        // unsafe in edition 2024; single-threaded under the lock, the
        // process has no other threads reading these vars.
        let guard = ENV_LOCK.blocking_lock();
        // SAFETY: serialized by ENV_LOCK; no concurrent readers of these
        // keys in this test process.
        unsafe {
            std::env::remove_var("MAILSLATE_GOOGLE_CLIENT_ID");
        }
        assert!(matches!(
            client_id(Provider::Google),
            Err(OAuthError::NotConfigured(_))
        ));

        // SAFETY: as above.
        unsafe {
            std::env::set_var("MAILSLATE_GOOGLE_CLIENT_ID", "google-app-id");
        }
        assert_eq!(
            client_id(Provider::Google).expect("configured"),
            "google-app-id"
        );

        // SAFETY: as above.
        unsafe {
            std::env::set_var("MAILSLATE_MICROSOFT_CLIENT_ID", "ms-app-id");
        }
        assert_eq!(
            client_id(Provider::Microsoft).expect("configured"),
            "ms-app-id"
        );
        // SAFETY: as above.
        unsafe {
            std::env::remove_var("MAILSLATE_GOOGLE_CLIENT_ID");
            std::env::remove_var("MAILSLATE_MICROSOFT_CLIENT_ID");
        }
        drop(guard);
    }
}
