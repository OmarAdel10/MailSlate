//! Logging: stderr plus a daily-rotated file under the data directory.
//!
//! `MAILSLATE_LOG` sets the filter (default `info`), matching the debugging
//! notes in DEVELOPMENT_ENVIRONMENT §3.

use std::path::Path;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Errors while setting up logging.
#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    /// The logs directory could not be created.
    #[error("could not create the logs directory: {0}")]
    Io(#[from] std::io::Error),
    /// The rotating log file could not be opened.
    #[error("could not open the log file: {0}")]
    Appender(#[from] tracing_appender::rolling::InitError),
}

/// Keeps the non-blocking file writer alive for the process lifetime;
/// dropping it flushes pending lines and stops the writer thread.
pub struct LoggingGuard(#[allow(dead_code)] tracing_appender::non_blocking::WorkerGuard);

/// Initializes tracing: stderr plus `daemon.log` in `logs_dir` (daily
/// rotation, seven files kept).
pub fn init(logs_dir: &Path) -> Result<LoggingGuard, LoggingError> {
    crate::secure_dir(logs_dir)?;
    let file = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("daemon")
        .filename_suffix("log")
        .max_log_files(7)
        .build(logs_dir)?;
    let (file_writer, guard) = tracing_appender::non_blocking(file);

    let filter =
        EnvFilter::try_from_env("MAILSLATE_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false),
        )
        .init();

    Ok(LoggingGuard(guard))
}
