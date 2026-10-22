//! Sync state machine types (ARCHITECTURE §9).

/// Lifecycle of one account's sync loop, as reported in `account_status`
/// events (ARCHITECTURE §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Disconnected,
    Connecting,
    Authenticating,
    Discovering,
    InitialSync,
    Idle,
    Busy,
    Polling,
    Backoff(String),
    ReauthRequired,
    Stopped,
}

impl SyncState {
    /// The wire label used in `account_status` event payloads.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connecting => "connecting",
            Self::Authenticating => "authenticating",
            Self::Discovering => "discovering",
            Self::InitialSync => "initial_sync",
            Self::Idle => "idle",
            Self::Busy => "busy",
            Self::Polling => "polling",
            Self::Backoff(_) => "backoff",
            Self::ReauthRequired => "reauth_required",
            Self::Stopped => "stopped",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_states_render_wire_strings() {
        assert_eq!(SyncState::Disconnected.as_str(), "disconnected");
        assert_eq!(SyncState::Connecting.as_str(), "connecting");
        assert_eq!(SyncState::Authenticating.as_str(), "authenticating");
        assert_eq!(SyncState::Discovering.as_str(), "discovering");
        assert_eq!(SyncState::InitialSync.as_str(), "initial_sync");
        assert_eq!(SyncState::Idle.as_str(), "idle");
        assert_eq!(SyncState::Busy.as_str(), "busy");
        assert_eq!(SyncState::Polling.as_str(), "polling");
        assert_eq!(SyncState::Backoff("imap down".into()).as_str(), "backoff");
        assert_eq!(SyncState::ReauthRequired.as_str(), "reauth_required");
        assert_eq!(SyncState::Stopped.as_str(), "stopped");
    }

    #[test]
    fn backoff_states_carry_their_reason() {
        let state = SyncState::Backoff("connection refused".into());
        match &state {
            SyncState::Backoff(reason) => assert_eq!(reason, "connection refused"),
            other => panic!("expected Backoff, got {other:?}"),
        }
        assert_ne!(SyncState::Idle, SyncState::Busy);
    }
}
