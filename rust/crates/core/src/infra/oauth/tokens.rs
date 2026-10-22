//! OAuth token persistence: a `TokenSnapshot` lives as JSON in the OS
//! keyring under the account's oauth key (`account:{id}:oauth`).

use crate::infra::keyring::{KeyStore, KeyStoreError, oauth_key};
use serde_json::json;

/// The token set an OAuth account needs to keep talking to its
/// provider. Stored as one JSON blob per account.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenSnapshot {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds after which the access token must be refreshed.
    pub expires_at: i64,
    /// Space-separated scopes granted by the provider.
    pub scopes: String,
}

impl TokenSnapshot {
    /// Fresh until 60 seconds before expiry: refresh margin so an
    /// in-flight request never races the expiry wall.
    pub fn is_fresh(&self, now: i64) -> bool {
        now + 60 <= self.expires_at
    }
}

/// Persists the snapshot; overwrite-on-save keeps refresh rotations
/// simple (no read-before-write). Serialized manually per the
/// deps-frozen ruling — field additions must update this shape.
pub fn save(
    store: &dyn KeyStore,
    account_id: i64,
    snapshot: &TokenSnapshot,
) -> Result<(), KeyStoreError> {
    let body = json!({
        "access_token": snapshot.access_token,
        "refresh_token": snapshot.refresh_token,
        "expires_at": snapshot.expires_at,
        "scopes": snapshot.scopes,
    });
    store.store(&oauth_key(account_id), &body.to_string())
}

/// Loads the snapshot; `None` when the account has no stored tokens.
pub fn load(store: &dyn KeyStore, account_id: i64) -> Result<Option<TokenSnapshot>, KeyStoreError> {
    let Some(json) = store.get(&oauth_key(account_id))? else {
        return Ok(None);
    };
    parse_snapshot(&json).map(Some).map_err(|detail| {
        KeyStoreError::Platform(format!("stored oauth snapshot is invalid: {detail}"))
    })
}

/// Removes the snapshot (best-effort callers may ignore errors).
pub fn delete(store: &dyn KeyStore, account_id: i64) -> Result<(), KeyStoreError> {
    store.delete(&oauth_key(account_id))
}

/// Parses the stored JSON shape back into a snapshot.
fn parse_snapshot(json: &str) -> Result<TokenSnapshot, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|err| err.to_string())?;
    let obj = value.as_object().ok_or("not a JSON object")?;
    Ok(TokenSnapshot {
        access_token: obj
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or("missing access_token")?
            .to_string(),
        refresh_token: obj
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .ok_or("missing refresh_token")?
            .to_string(),
        expires_at: obj
            .get("expires_at")
            .and_then(|v| v.as_i64())
            .ok_or("missing expires_at")?,
        scopes: obj
            .get("scopes")
            .and_then(|v| v.as_str())
            .ok_or("missing scopes")?
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::keyring::FakeKeyStore;

    fn snapshot() -> TokenSnapshot {
        TokenSnapshot {
            access_token: "access-token".into(),
            refresh_token: "refresh-token".into(),
            expires_at: 1_900_000_000,
            scopes: "https://mail.google.com/".into(),
        }
    }

    #[test]
    fn snapshot_round_trips_through_the_keystore() {
        let store = FakeKeyStore::new();
        save(&store, 7, &snapshot()).expect("save");
        let loaded = load(&store, 7).expect("load").expect("snapshot stored");
        assert_eq!(loaded, snapshot());
    }

    #[test]
    fn load_for_unknown_account_is_none() {
        let store = FakeKeyStore::new();
        let loaded = load(&store, 42).expect("load");
        assert!(loaded.is_none());
    }

    #[test]
    fn delete_clears_the_snapshot() {
        let store = FakeKeyStore::new();
        save(&store, 7, &snapshot()).expect("save");
        delete(&store, 7).expect("delete");
        let loaded = load(&store, 7).expect("load");
        assert!(loaded.is_none());
    }

    #[test]
    fn corrupted_entry_maps_to_keystore_error() {
        let store = FakeKeyStore::new();
        store
            .store(&crate::infra::keyring::oauth_key(9), "not-json{")
            .expect("seed corrupt");
        let loaded = load(&store, 9);
        assert!(loaded.is_err(), "corrupt JSON must surface as an error");
    }

    #[test]
    fn token_is_fresh_only_past_the_60s_margin() {
        let snap = TokenSnapshot {
            expires_at: 1_000,
            ..snapshot()
        };
        // 60s before expiry the token is already considered stale.
        assert!(!snap.is_fresh(941));
        assert!(snap.is_fresh(940));
        assert!(snap.is_fresh(500));
    }
}
