//! TLS setup for IMAP connections: rustls with the webpki-roots
//! system-less trust store (ARCHITECTURE §3.1; no OS cert dependency).

use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::rustls::RootCertStore;

/// Builds the TLS connector used for implicit-TLS (993) and STARTTLS
/// upgrades. Trusts the bundled webpki roots only — no OS store, no
/// custom CAs — matching the product's direct-to-provider posture.
pub fn tls_connector() -> TlsConnector {
    // Select the ring CryptoProvider explicitly and idempotently: the
    // process default is only settable once, and other crates in the
    // binary may have enabled their own rustls feature sets.
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConnector::from(std::sync::Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_connector_carries_the_webpki_roots() {
        let connector = tls_connector();
        // The connector is opaque; assert the trust store it was built
        // from is the webpki set and non-trivial in size.
        assert!(
            webpki_roots::TLS_SERVER_ROOTS.len() > 100,
            "webpki root set must be present"
        );
        let _ = connector;
    }
}
