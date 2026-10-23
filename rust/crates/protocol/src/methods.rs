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

/// Re-authorizes an existing OAuth account after its refresh token was
/// rejected (PRD F1.6); the daemon saves the new snapshot to that
/// account and restarts its sync actor.
pub const START_REAUTH: &str = "start_reauth";

/// Finalizes a finished OAuth flow with the user-provided profile
/// (name, email, color); creates the account row (M2).
pub const COMPLETE_OAUTH_ACCOUNT: &str = "complete_oauth_account";

/// Aborts an in-flight OAuth flow: releases the session slot and drops
/// staged tokens so a new flow can start immediately (wizard back /
/// timeout / retry) (M2).
pub const ABORT_OAUTH_ACCOUNT: &str = "abort_oauth_account";

/// Asks the daemon to re-run the sync for one account (or all when
/// `account_id` is absent) (M2).
pub const SYNC_NOW: &str = "sync_now";

/// Lists every account with its current sync state (M2).
pub const GET_STATUS: &str = "get_status";

/// Full-text search across all messages of one account (M3 F7); the
/// daemon replies with `{ messages: [...], total: n }`.
pub const SEARCH: &str = "search";

/// Lists one account's threads newest-first (M3 F3.3); the daemon
/// replies with `{ threads: [...], total: n }`.
pub const GET_THREADS: &str = "get_threads";

/// Lists one thread's messages newest-first (M3 F3.4); the daemon
/// replies with `{ messages: [...], total: n }`.
pub const GET_THREAD_MESSAGES: &str = "get_thread_messages";

/// Returns one message's headers and body state (M3 F3.4); the daemon
/// replies with `{ id, account_id, subject, from_json, to_json,
/// date_utc, snippet, has_attachments, body_state }`.
pub const GET_MESSAGE: &str = "get_message";

/// Returns one message's decoded body (M3 F3.4); the daemon replies
/// with `{ message_id, text_body, html_clean }`.
pub const GET_MESSAGE_BODY: &str = "get_message_body";

/// Returns one message's attachments (M3 F3.4); the daemon replies
/// with `{ attachments: [...] }`.
pub const GET_ATTACHMENTS: &str = "get_attachments";

/// Marks a message as read or unread (M3 F3.4); the daemon replies
/// with `{ ok: true }`.
pub const MARK_READ: &str = "mark_read";
