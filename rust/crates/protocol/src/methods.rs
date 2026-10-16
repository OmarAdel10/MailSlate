//! Method-name constants for the v1 IPC surface (ARCHITECTURE.md §5.3).
//!
//! Only methods implemented so far are listed; the rest (`get_status`,
//! `sync_now`, `mutate`, …) land with their features so nothing here is dead
//! weight.

/// Handshake: returns [`crate::PingResult`]. First request of every connection.
pub const PING: &str = "ping";

/// Asks the daemon to shut down gracefully (Settings → Daemon).
pub const SHUTDOWN: &str = "shutdown";
