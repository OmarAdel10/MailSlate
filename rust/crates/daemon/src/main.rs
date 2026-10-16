//! MailSlate sync daemon (headless).
//!
//! Owns all mail state: the SQLite writer, per-account sync engines, the
//! OAuth token lifecycle, the outbox drain, and OS notifications.
//! Autostarted at login or spawned on demand by the GUI (ARCHITECTURE.md §2).
//!
//! The real runtime (tokio, IPC listener, engines) lands with M2; this
//! skeleton keeps the binary building and its CLI contract observable.

/// Parsed command-line intent for the daemon binary.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    /// Run the daemon (default).
    Run,
    /// Print the version and exit.
    Version,
    /// Print usage and exit.
    Help,
}

/// Parses raw arguments into a [`Command`].
fn parse_args(args: &[String]) -> Command {
    match args.first().map(String::as_str) {
        Some("--version") | Some("-V") => Command::Version,
        Some("--help") | Some("-h") => Command::Help,
        _ => Command::Run,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Command::Version => println!("mailslate-daemon {}", env!("CARGO_PKG_VERSION")),
        Command::Help => println!("Usage: mailslate-daemon [--version] [--help]"),
        Command::Run => {
            // Placeholder until M2: the daemon will init tracing, acquire the
            // single-writer lock, start the IPC listener, and run sync.
            println!(
                "mailslate-daemon {} (skeleton: engines land in M2)",
                env!("CARGO_PKG_VERSION")
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_version_flag() {
        assert_eq!(parse_args(&args(&["--version"])), Command::Version);
    }

    #[test]
    fn defaults_to_run() {
        assert_eq!(parse_args(&args(&[])), Command::Run);
    }
}
