//! CLI contract tests: run the real binary and pin its observable behavior.

use std::process::Command;

fn daemon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mailslate-daemon"))
}

#[test]
fn version_flag_prints_name_and_version() {
    let out = daemon().arg("--version").output().expect("run daemon");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("mailslate-daemon "), "got: {stdout}");
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")), "got: {stdout}");
}

#[test]
fn help_flag_prints_usage() {
    let out = daemon().arg("--help").output().expect("run daemon");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim(),
        "Usage: mailslate-daemon [--version] [--help]"
    );
}

#[test]
fn no_args_prints_placeholder_banner() {
    let out = daemon().output().expect("run daemon");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("skeleton: engines land in M2"),
        "got: {stdout}"
    );
}
