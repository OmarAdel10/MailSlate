//! Secrets storage: the `KeyStore` trait and the OS keyring implementation
//! (ARCHITECTURE.md §8.3, §12).
//!
//! Only tokens and passwords ever go through this layer; the database stays
//! plaintext by design (ADR 9). Tests use [`FakeKeyStore`] — they never touch
//! the real platform store.

use std::collections::HashMap;
use std::sync::Mutex;

/// Keyring service name for all MailSlate secrets.
pub const KEYRING_SERVICE: &str = "mailslate";

/// Key name for an account's OAuth token bundle (JSON).
pub fn oauth_key(account_id: i64) -> String {
    format!("account:{account_id}:oauth")
}

/// Key name for an account's IMAP password.
pub fn imap_password_key(account_id: i64) -> String {
    format!("account:{account_id}:imap_password")
}

/// Errors from the secrets store.
#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    /// The OS keyring is not available (locked, headless, policy).
    #[error("keyring unavailable: {0}")]
    Unavailable(String),
    /// Any other platform error from the keyring backend.
    #[error("keyring error: {0}")]
    Platform(String),
}

/// Secrets store interface (ARCHITECTURE §3.1 — `KeyStore` trait).
pub trait KeyStore: Send + Sync {
    /// Writes `secret` under `key`, replacing any previous value.
    fn store(&self, key: &str, secret: &str) -> Result<(), KeyStoreError>;
    /// Reads the secret for `key`; `None` when absent.
    fn get(&self, key: &str) -> Result<Option<String>, KeyStoreError>;
    /// Deletes `key`; deleting a missing key succeeds.
    fn delete(&self, key: &str) -> Result<(), KeyStoreError>;
}

/// Real OS keyring implementation (Linux secret-service, macOS Keychain,
/// Windows Credential Manager).
pub struct OsKeyStore;

impl OsKeyStore {
    /// Probes whether the platform keyring store is usable without creating
    /// an entry.
    pub fn is_available() -> bool {
        keyring::Entry::store_status().is_ok()
    }
}

impl KeyStore for OsKeyStore {
    fn store(&self, key: &str, secret: &str) -> Result<(), KeyStoreError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, key).map_err(map_store_init_error)?;
        entry.set_password(secret).map_err(map_operation_error)
    }

    fn get(&self, key: &str) -> Result<Option<String>, KeyStoreError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, key).map_err(map_store_init_error)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_operation_error(err)),
        }
    }

    fn delete(&self, key: &str) -> Result<(), KeyStoreError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, key).map_err(map_store_init_error)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_operation_error(err)),
        }
    }
}

fn map_store_init_error(err: keyring::Error) -> KeyStoreError {
    match err {
        keyring::Error::NoDefaultStore => {
            KeyStoreError::Unavailable("no default keyring store".to_string())
        }
        other => KeyStoreError::Platform(other.to_string()),
    }
}

fn map_operation_error(err: keyring::Error) -> KeyStoreError {
    match err {
        keyring::Error::NoStorageAccess(platform) => {
            KeyStoreError::Unavailable(platform.to_string())
        }
        keyring::Error::NoDefaultStore => {
            KeyStoreError::Unavailable("no default keyring store".to_string())
        }
        other => KeyStoreError::Platform(other.to_string()),
    }
}

/// In-memory implementation for tests and dev tooling; never touches the OS.
#[derive(Default)]
pub struct FakeKeyStore {
    entries: Mutex<HashMap<String, String>>,
}

impl FakeKeyStore {
    /// Creates an empty fake store.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.entries.lock().expect("fake keystore poisoned")
    }
}

impl KeyStore for FakeKeyStore {
    fn store(&self, key: &str, secret: &str) -> Result<(), KeyStoreError> {
        self.lock().insert(key.to_string(), secret.to_string());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>, KeyStoreError> {
        Ok(self.lock().get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<(), KeyStoreError> {
        self.lock().remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_store_round_trip() {
        let store = FakeKeyStore::new();
        assert_eq!(store.get("account:1:oauth").unwrap(), None);

        store
            .store("account:1:oauth", "{\"access\":\"x\"}")
            .unwrap();
        assert_eq!(
            store.get("account:1:oauth").unwrap().as_deref(),
            Some("{\"access\":\"x\"}")
        );

        store.delete("account:1:oauth").unwrap();
        assert_eq!(store.get("account:1:oauth").unwrap(), None);

        // Deleting a missing key is not an error.
        store.delete("account:1:oauth").unwrap();
    }

    #[test]
    fn key_names_follow_the_account_scheme() {
        assert_eq!(oauth_key(42), "account:42:oauth");
        assert_eq!(imap_password_key(42), "account:42:imap_password");
    }

    #[test]
    fn trait_objects_are_usable() {
        let store: Box<dyn KeyStore> = Box::new(FakeKeyStore::new());
        store.store("k", "v").unwrap();
        assert_eq!(store.get("k").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn store_init_errors_map_to_unavailable() {
        let mapped = map_store_init_error(keyring::Error::NoDefaultStore);
        assert!(matches!(
            mapped,
            KeyStoreError::Unavailable(ref message)
                if message.as_str() == "no default keyring store"
        ));
    }

    #[test]
    fn storage_access_errors_map_to_unavailable_with_detail() {
        let platform: Box<dyn std::error::Error + Send + Sync> =
            Box::new(std::io::Error::other("secret service is locked"));
        let mapped = map_operation_error(keyring::Error::NoStorageAccess(platform));
        assert!(matches!(
            mapped,
            KeyStoreError::Unavailable(ref message)
                if message.contains("secret service is locked")
        ));
    }

    #[test]
    fn operation_errors_map_no_default_store_and_platform() {
        let missing = map_operation_error(keyring::Error::NoDefaultStore);
        assert!(matches!(
            missing,
            KeyStoreError::Unavailable(ref message)
                if message.as_str() == "no default keyring store"
        ));

        let platform = map_operation_error(keyring::Error::BadEncoding(vec![0xff, 0xfe]));
        assert!(matches!(platform, KeyStoreError::Platform(_)));
    }

    #[test]
    fn os_key_store_availability_smoke() {
        // The boolean is environment-dependent; the call must never panic.
        let _ = OsKeyStore::is_available();
    }
}
