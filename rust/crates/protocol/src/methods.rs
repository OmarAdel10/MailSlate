//! Method-name constants for the v1 IPC surface (ARCHITECTURE.md §5.3).
//!
//! Only methods implemented so far are listed; the rest (`get_status`,
//! `sync_now`, `mutate`, …) land with their features so nothing here is dead
//! weight.

/// Handshake: returns [`crate::PingResult`]. First request of every connection.
pub const PING: &str = "ping";

/// Asks the daemon to shut down gracefully (Settings → Daemon).
pub const SHUTDOWN: &str = "shutdown";

/// Creates a generic IMAP/SMTP account (Settings → Accounts, M2).
pub const ADD_IMAP_ACCOUNT: &str = "add_imap_account";

/// `test_connection`: probes IMAP login and SMTP reachability without
/// persisting anything. Used by the M2 account wizard (USER_FLOW §3).
pub const TEST_CONNECTION: &str = "test_connection";

/// Removes an account and scrubs its secrets from the keyring (M2).
pub const REMOVE_ACCOUNT: &str = "remove_account";

/// Patches mutable account fields: name, color, sync window (M2).
pub const UPDATE_ACCOUNT: &str = "update_account";

/// Lists every configured account (Settings → Accounts, M2).
pub const GET_ACCOUNTS: &str = "get_accounts";

/// Begins the OAuth authorize-code flow for a provider; replies with
/// the authorize URL the GUI must open in a browser (M2).
pub const START_OAUTH: &str = "start_oauth";

/// Finalizes a finished OAuth flow with the user-provided profile
/// (name, email, color); creates the account row (M2).
pub const COMPLETE_OAUTH_ACCOUNT: &str = "complete_oauth_account";

/// Asks the daemon to re-run the sync for one account (or all when
/// `account_id` is absent) (M2).
pub const SYNC_NOW: &str = "sync_now";

/// Lists every account with its current sync state (M2).
pub const GET_STATUS: &str = "get_status";
