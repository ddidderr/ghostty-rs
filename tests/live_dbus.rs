#![allow(clippy::unwrap_used)]
//! Runs `scripts/live-dbus-test.sh` in golden mode: the Rust build is started
//! on a private D-Bus session bus inside a private headless weston, with mock
//! Notifications / systemd1 / portal services (and the real
//! xdg-desktop-portal + gtk backend), and its observable D-Bus behaviour is
//! compared with `scripts/live-dbus/golden/*.txt`, recorded from the original
//! Ghostty 1.3.1 (`UPDATE_GOLDEN=1 just live-dbus` re-records them against the
//! installed original). See `docs/rewrite/notes/live-dbus.md`.
//!
//! Skipped (passes) when a needed tool is missing, when there is no
//! release-profile Rust build to test (debug builds use a different D-Bus
//! application id, `com.mitchellh.ghostty-debug`) or when that build is older
//! than the sources (build it with `scripts/live-dbus/build-rust.sh`). Set
//! `GHOSTTY_SKIP_LIVE_DBUS` to skip explicitly.

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

fn newest_source(dir: &Path, newest: &mut SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name();
        if p.is_dir() {
            if name != "target" && name != ".git" {
                newest_source(&p, newest);
            }
        } else if matches!(
            p.extension().and_then(|x| x.to_str()),
            Some("rs" | "toml" | "blp" | "ui" | "css")
        ) && let Ok(m) = e.metadata().and_then(|m| m.modified())
        {
            *newest = (*newest).max(m);
        }
    }
}

fn have(tool: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {tool}")])
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn rust_build_matches_recorded_original_on_private_bus() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("GHOSTTY_SKIP_LIVE_DBUS").is_some() {
        return;
    }
    for tool in ["weston", "dbus-run-session", "busctl", "python3", "xdotool"] {
        if !have(tool) {
            eprintln!("live-dbus: skipped ({tool} missing)");
            return;
        }
    }
    let bin = std::env::var_os("RUST_BIN")
        .map_or_else(|| root.join("target/vp/production/ghostty"), PathBuf::from);
    let Ok(built) = std::fs::metadata(&bin).and_then(|m| m.modified()) else {
        eprintln!("live-dbus: skipped (no release build at {})", bin.display());
        return;
    };
    let mut newest = SystemTime::UNIX_EPOCH;
    for d in ["crates", "src", "resources"] {
        newest_source(&root.join(d), &mut newest);
    }
    if newest > built {
        eprintln!(
            "live-dbus: skipped ({} is older than the sources)",
            bin.display()
        );
        return;
    }
    let out = Command::new("bash")
        .arg(root.join("scripts/live-dbus-test.sh"))
        .env("LABELS", "rust")
        .env("RUST_BIN", &bin)
        .env("OUT", root.join("target/live-dbus"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    eprintln!("{text}");
    assert!(out.status.success(), "live-dbus comparison failed:\n{text}");
}
