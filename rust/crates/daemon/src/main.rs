//! MailSlate sync daemon (headless).
//!
//! Owns all mail state: the SQLite writer, per-account sync engines, the
//! OAuth token lifecycle, the outbox drain, and OS notifications.
//! Autostarted at login or spawned on demand by the GUI; it talks to the
//! GUI over the `mailslate-protocol` IPC (ARCHITECTURE.md §2, §5).

mod dispatch;
mod logging;
mod server;

use std::process::ExitCode;

#[cfg(unix)]
use mailslate_core::infra::{db, paths};

use crate::server::ServerError;

/// Command line surface of the daemon.
enum Command {
    /// Run the IPC server.
    Run,
    /// Print the version.
    Version,
    /// Print usage.
    Help,
}

fn parse_args(args: &[String]) -> Command {
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => Command::Version,
        Some("--help" | "-h") => Command::Help,
        _ => Command::Run,
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Command::Version => {
            println!("mailslate-daemon {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Help => {
            println!("Usage: mailslate-daemon [--version] [--help]");
            ExitCode::SUCCESS
        }
        Command::Run => match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(ServerError::AlreadyRunning) => {
                println!("mailslate-daemon: another instance is already running");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("mailslate-daemon: {err}");
                ExitCode::FAILURE
            }
        },
    }
}

#[cfg(unix)]
fn run() -> Result<(), ServerError> {
    let data_dir = paths::data_dir();
    std::fs::create_dir_all(&data_dir)?;
    let _logs = logging::init(&paths::logs_dir())?;

    let db_path = paths::db_path();
    let mut conn = db::open_rw(&db_path)?;
    let schema = db::migrations::run(&mut conn)?;
    db::assert_fts5()?;
    drop(conn); // the writer actor owns the rw connection from here on
    tracing::info!(schema, "database ready");

    let keystore: std::sync::Arc<dyn mailslate_core::infra::keyring::KeyStore> =
        std::sync::Arc::new(mailslate_core::infra::keyring::OsKeyStore);
    let ctx = std::sync::Arc::new(crate::dispatch::DispatchCtx {
        writer: std::sync::Arc::new(db::writer::WriterActor::spawn(&db_path)),
        keystore,
        reader: std::sync::Arc::new(db::pool::ReadPool::open(&db_path)?),
        bus: mailslate_core::domain::events::EventBus::new(),
        oauth: std::sync::Mutex::new(None),
        token_poster: std::sync::Arc::new(mailslate_core::infra::oauth::grant::HttpTokenEndpoint),
        oauth_deadline: std::time::Duration::from_secs(300),
    });

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(server::run(paths::socket_path(), paths::pid_file(), ctx))
}

#[cfg(not(unix))]
fn run() -> Result<(), ServerError> {
    Err(ServerError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn parses_version_flag() {
        assert!(matches!(
            parse_args(&args(&["--version"])),
            Command::Version
        ));
    }

    #[test]
    fn parses_short_version_flag() {
        assert!(matches!(parse_args(&args(&["-V"])), Command::Version));
    }

    #[test]
    fn parses_help_flags() {
        assert!(matches!(parse_args(&args(&["--help"])), Command::Help));
        assert!(matches!(parse_args(&args(&["-h"])), Command::Help));
    }

    #[test]
    fn defaults_to_run() {
        assert!(matches!(parse_args(&args(&[])), Command::Run));
    }

    #[test]
    fn unknown_first_arg_runs() {
        assert!(matches!(parse_args(&args(&["--bogus"])), Command::Run));
    }

    #[test]
    fn extra_args_are_ignored_after_the_first() {
        assert!(matches!(
            parse_args(&args(&["--version", "extra"])),
            Command::Version
        ));
    }

    #[test]
    fn flag_after_positional_is_ignored() {
        assert!(matches!(
            parse_args(&args(&["something", "--version"])),
            Command::Run
        ));
    }
}
