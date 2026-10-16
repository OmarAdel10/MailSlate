//! Per-OS path resolution (ARCHITECTURE.md §16).
//!
//! All paths resolve from the data directory, which `MAILSLATE_DATA_DIR`
//! overrides (scratch profiles, tests). Resolution never creates directories;
//! the daemon creates what it needs at startup.

use std::path::{Path, PathBuf};

/// Environment variable that overrides the data directory.
pub const DATA_DIR_ENV: &str = "MAILSLATE_DATA_DIR";

#[cfg(any(target_os = "macos", windows))]
const APP_DIR_NAME: &str = "MailSlate";
#[cfg(not(any(target_os = "macos", windows)))]
const APP_DIR_NAME: &str = "mailslate";

/// Resolves the per-user data directory.
///
/// `MAILSLATE_DATA_DIR` wins when set and non-empty; otherwise the platform
/// default is used: `$XDG_DATA_HOME/mailslate` (Linux),
/// `~/Library/Application Support/MailSlate` (macOS), `%APPDATA%\MailSlate`
/// (Windows).
pub fn data_dir() -> PathBuf {
    data_dir_from(|key| std::env::var(key).ok())
}

pub(crate) fn data_dir_from(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(dir) = env(DATA_DIR_ENV) {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    dirs::data_dir()
        .expect("no platform data directory found; set MAILSLATE_DATA_DIR")
        .join(APP_DIR_NAME)
}

/// Path of the SQLite database file.
pub fn db_path() -> PathBuf {
    data_dir().join("mailslate.db")
}

/// Directory for rotated daemon logs (ARCHITECTURE §16).
pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

/// Directory for content-addressed blob storage (ARCHITECTURE §7.5).
pub fn blobs_dir() -> PathBuf {
    data_dir().join("blobs")
}

/// Directory for outgoing attachment staging (ARCHITECTURE §7.5).
pub fn staging_dir() -> PathBuf {
    data_dir().join("staging")
}

/// PID file written by the daemon at startup (ARCHITECTURE §2.1).
pub fn pid_file() -> PathBuf {
    data_dir().join("daemon.pid")
}

/// IPC endpoint the daemon listens on (unix sockets; ARCHITECTURE §5).
#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    socket_path_for(&data_dir())
}

/// IPC endpoint for an explicit data directory.
///
/// Mirrors [`socket_path`] but resolves the fallback from `data_dir` instead
/// of the environment, so the GUI and a daemon spawned with an explicit
/// profile always agree on the socket.
#[cfg(unix)]
pub fn socket_path_for(data_dir: &Path) -> PathBuf {
    socket_path_from_env(|key| std::env::var(key).ok(), data_dir)
}

#[cfg(unix)]
fn socket_path_from_env(env: impl Fn(&str) -> Option<String>, data_dir: &Path) -> PathBuf {
    #[cfg(target_os = "linux")]
    if let Some(runtime) = env("XDG_RUNTIME_DIR") {
        let trimmed = runtime.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("mailslate").join("daemon.sock");
        }
    }
    data_dir.join("daemon.sock")
}

/// Named pipe the daemon listens on (Windows; ARCHITECTURE §5).
#[cfg(windows)]
pub fn socket_pipe_name() -> &'static str {
    r"\\.\pipe\mailslate"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_env_override_wins() {
        let dir =
            data_dir_from(|key| (key == DATA_DIR_ENV).then(|| "/tmp/mailslate-test".to_string()));
        assert_eq!(dir, PathBuf::from("/tmp/mailslate-test"));
    }

    #[test]
    fn blank_env_override_is_ignored() {
        let dir = data_dir_from(|key| (key == DATA_DIR_ENV).then(|| "   ".to_string()));
        assert!(dir.ends_with(APP_DIR_NAME));
    }

    #[test]
    fn data_dir_default_is_platform_default() {
        let dir = data_dir_from(|_| None);
        assert_eq!(dir, dirs::data_dir().expect("data dir").join(APP_DIR_NAME));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn socket_prefers_xdg_runtime_dir() {
        let path = socket_path_from_env(
            |key| (key == "XDG_RUNTIME_DIR").then(|| "/run/user/1000".to_string()),
            Path::new("/data"),
        );
        assert_eq!(
            path,
            PathBuf::from("/run/user/1000")
                .join("mailslate")
                .join("daemon.sock")
        );
    }

    #[cfg(unix)]
    #[test]
    fn socket_falls_back_to_data_dir() {
        let path = socket_path_from_env(|_| None, Path::new("/data"));
        assert_eq!(path, PathBuf::from("/data").join("daemon.sock"));
    }

    #[test]
    fn helpers_live_under_the_data_dir() {
        let base = data_dir();
        assert_eq!(db_path(), base.join("mailslate.db"));
        assert_eq!(logs_dir(), base.join("logs"));
        assert_eq!(blobs_dir(), base.join("blobs"));
        assert_eq!(staging_dir(), base.join("staging"));
        assert_eq!(pid_file(), base.join("daemon.pid"));
    }
}
