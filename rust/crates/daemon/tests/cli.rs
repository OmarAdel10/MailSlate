//! CLI surface: `--version` and `--help`.
//!
//! Daemon startup itself is covered end-to-end by `tests/ipc.rs`.

use std::process::Command;

fn daemon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mailslate-daemon"))
}

#[test]
fn version_flag_prints_name_and_version() {
    let output = daemon().arg("--version").output().expect("run daemon");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("mailslate-daemon "));
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_flag_prints_usage() {
    let output = daemon().arg("--help").output().expect("run daemon");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        "Usage: mailslate-daemon [--version] [--help]"
    );
}
