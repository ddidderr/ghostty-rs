//! Subprocess tests for crash capture: a child process installs the crash
//! handler, crashes (panic / SIGSEGV / abort), and the parent verifies that a
//! valid `.ghosttycrash` envelope appeared and that `ghostty +crash-report`
//! lists it.
//!
//! The child is this test executable re-run with `GHOSTTY_CRASH_CHILD`.
#![allow(clippy::unwrap_used)]

use std::{os::unix::process::ExitStatusExt, path::Path, process::Command};

use ghostty_crash::{
    DimsSnapshot,
    Options,
    SurfaceDims,
    ThreadGuard,
    ThreadType,
    envelope::{Envelope, Item, ItemType},
};

fn child_main(mode: &str) {
    let dir = std::env::var("GHOSTTY_CRASH_DIR").unwrap();
    ghostty_crash::install(&Options {
        dir: Some(dir.into()),
        ..Options::default()
    })
    .unwrap();
    let dims = SurfaceDims::new();
    dims.set(DimsSnapshot {
        screen_width: 1024,
        screen_height: 768,
        grid_columns: 100,
        grid_rows: 40,
        cell_width: 10,
        cell_height: 19,
    });
    let _g = ThreadGuard::new(ThreadType::Renderer, Some(dims));
    match mode {
        "panic" => {
            // Panic on a spawned thread, like a renderer-thread crash.
            let d = ghostty_crash::current_dims();
            let r = std::thread::spawn(move || {
                let _g = ThreadGuard::new(ThreadType::Renderer, d);
                panic!("intentional test panic")
            })
            .join();
            assert!(r.is_err());
            // Make the process die like a real crash would.
            std::process::exit(101);
        }
        "segv" => {
            let me = rustix::process::getpid();
            rustix::process::kill_process(me, rustix::process::Signal::SEGV).unwrap();
            // The handler re-raises with the default action; never get here.
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
        // A real fault on the guard page, handled on the alternate stack.
        "overflow" => {
            // Unbounded on purpose: overflowing the stack is the test.
            #[allow(unconditional_recursion)]
            fn recurse(n: u64) -> u64 {
                let pad = [n as u8; 256];
                std::hint::black_box(&pad);
                recurse(n + 1) + u64::from(pad[0])
            }
            std::process::exit(i32::from(recurse(0) == 1));
        }
        "abort" => std::process::abort(),
        _ => unreachable!(),
    }
}

fn run_child(mode: &str, dir: &Path) -> std::process::ExitStatus {
    let exe = std::env::current_exe().unwrap();
    Command::new(exe)
        .args(["--exact", "crash_child", "--nocapture", "--test-threads=1"])
        .env("GHOSTTY_CRASH_CHILD", mode)
        .env("GHOSTTY_CRASH_DIR", dir)
        .env("RUST_BACKTRACE", "0")
        .status()
        .unwrap()
}

/// Entry point of the child process; a no-op in a normal test run.
#[test]
fn crash_child() {
    if let Ok(mode) = std::env::var("GHOSTTY_CRASH_CHILD") {
        child_main(&mode);
    }
}

fn reports(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    rd.map(|e| e.unwrap().path()).collect()
}

/// Parse the single report in `dir` and check its contents.
///
/// `thread_type` is `None` when the signal can't be targeted at the thread
/// with the crash context (process-directed signals).
fn check_report(dir: &Path, kind: &str, thread_type: Option<&str>) {
    let files = reports(dir);
    assert_eq!(1, files.len(), "{files:?}");
    let f = &files[0];
    let name = f.file_name().unwrap().to_str().unwrap();
    let stem = name.strip_suffix(".ghosttycrash").unwrap();
    assert_eq!(36, stem.len(), "{name}");
    assert_eq!(Some('4'), stem.chars().nth(14));

    let data = std::fs::read(f).unwrap();
    let env = Envelope::parse_slice(&data).unwrap();
    assert!(env.headers.contains_key("event_id"));
    let Some(Item::Encoded(ev)) = env.items.first() else {
        panic!("no items")
    };
    assert_eq!(ItemType::Event, ev.item_type);
    let v: serde_json::Value = serde_json::from_slice(&ev.payload).unwrap();
    assert_eq!("1.3.1", v["release"]);
    assert_eq!("fatal", v["level"]);
    if let Some(t) = thread_type {
        assert_eq!(t, v["tags"]["thread-type"]);
        assert_eq!(100, v["contexts"]["Dimensions"]["grid-columns"]);
        assert_eq!(768, v["contexts"]["Dimensions"]["screen-height"]);
        assert_eq!(19, v["contexts"]["Dimensions"]["cell-height"]);
    }
    assert_eq!("opengl", v["tags"]["renderer"]);
    assert_eq!(kind, v["exception"]["values"][0]["type"]);

    // +crash-report lists the file.
    let out = Command::new(env!("CARGO_BIN_EXE_ghostty"))
        .arg("+crash-report")
        .env("XDG_STATE_HOME", dir.parent().unwrap().parent().unwrap())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with(name), "{stdout:?}");
    assert!(
        stdout.contains(" ago)") || stdout.contains("(now)"),
        "{stdout:?}"
    );
}

/// A temp state home with the crash dir path `<state>/ghostty/crash`.
fn state_dirs() -> (tempfile::TempDir, std::path::PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("ghostty").join("crash");
    (t, dir)
}

#[test]
fn panic_writes_report() {
    let (_t, dir) = state_dirs();
    let st = run_child("panic", &dir);
    assert_eq!(Some(101), st.code());
    check_report(&dir, "panic", Some("renderer"));
    // The panic report carries a backtrace attachment.
    let data = std::fs::read(&reports(&dir)[0]).unwrap();
    let mut env = Envelope::parse_slice(&data).unwrap();
    assert_eq!(2, env.items.len());
    env.items[1].decode().unwrap();
    let Item::Attachment(a) = &env.items[1] else {
        panic!()
    };
    assert_eq!("backtrace.txt", a.filename);
}

#[test]
fn sigsegv_writes_report() {
    let (_t, dir) = state_dirs();
    let st = run_child("segv", &dir);
    assert_eq!(Some(11), st.signal());
    check_report(&dir, "SIGSEGV", None);
}

#[test]
fn stack_overflow_writes_report() {
    let (_t, dir) = state_dirs();
    let st = run_child("overflow", &dir);
    assert_eq!(Some(11), st.signal());
    check_report(&dir, "SIGSEGV", Some("renderer"));
}

#[test]
fn abort_writes_report() {
    let (_t, dir) = state_dirs();
    let st = run_child("abort", &dir);
    assert_eq!(Some(6), st.signal());
    check_report(&dir, "SIGABRT", Some("renderer"));
}
