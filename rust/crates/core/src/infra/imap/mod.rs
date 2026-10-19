//! IMAP adapter: connects the daemon to IMAP servers (ARCHITECTURE §3.1).
//!
//! The adapter is the only place that speaks the IMAP wire protocol;
//! the sync engine consumes it through narrow functions and never
//! sees raw protocol types. Authentication uses XOAUTH2 for OAuth
//! accounts and LOGIN for password accounts.

pub mod auth;
pub mod connect;
pub mod mailbox;
pub mod tls;

#[cfg(test)]
pub(crate) mod testing;

/// Errors surfaced by the IMAP adapter. The sync engine maps these to
/// per-account sync states; the wire details stay in this module.
#[derive(Debug, thiserror::Error)]
pub enum ImapError {
    #[error("imap connect failed: {0}")]
    Connect(String),
    #[error("imap tls handshake failed: {0}")]
    Tls(String),
    #[error("imap authentication failed: {0}")]
    Auth(String),
    #[error("imap protocol error: {0}")]
    Protocol(String),
}
