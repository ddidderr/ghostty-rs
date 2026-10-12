//! Functional test: a C program using the unchanged `ghostty.h` (init,
//! CLI actions, config get/load/finalize/diagnostics, app lifecycle,
//! key bindings, actions callback) compiled with `cc` and linked against
//! the built `libghostty`.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::{path::PathBuf, process::Command};

fn host() -> PathBuf {
    static ONCE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let lib_dir = common::build_lib();
        let dir = common::scratch_dir("host");
        let exe = dir.join("host");
        let src = common::manifest_dir().join("tests/c/host.c");
        common::compile_c(&lib_dir, &[&src], &exe, &["-Wall", "-Wextra", "-pthread"]);
        exe
    })
    .clone()
}

fn run(args: &[&str]) -> std::process::Output {
    let dir = tempfile::tempdir().unwrap();
    Command::new(host())
        .args(args)
        .env("XDG_CONFIG_HOME", dir.path())
        .env("HOME", dir.path())
        .env_remove("GHOSTTY_LOG")
        .output()
        .unwrap()
}

#[test]
fn host_program_passes() {
    let out = run(&[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "host failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("OK"), "stdout:\n{stdout}");
    assert!(stdout.contains("version=1.3.1"), "stdout:\n{stdout}");
}

#[test]
fn cli_try_action_runs_version_and_exits() {
    let out = run(&["+version"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.starts_with("Ghostty 1.3.1"), "stdout:\n{stdout}");
    // The action ran instead of the rest of the program.
    assert!(!stdout.contains("OK"));
}

#[test]
fn cli_try_action_runs_other_actions() {
    let out = run(&["+list-keybinds"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("new_window"), "stdout:\n{stdout}");

    let out = run(&["+list-actions"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("new_window"));
}

#[test]
fn init_rejects_multiple_actions() {
    let out = run(&["+version", "+help"]);
    // ghostty_init fails with a non-zero code, which the host CHECKs; the
    // program then still runs on.
    assert!(!out.status.success());
}
