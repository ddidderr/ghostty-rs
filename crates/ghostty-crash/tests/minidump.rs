//! The fatal-signal path writes an envelope whose second item is a minidump
//! (`event.minidump` attachment) of the crashed process. A child process
//! (this test executable re-run with `GHOSTTY_MD_CHILD`) crashes on a spawned
//! thread; the parent parses the minidump with the `minidump` crate and checks
//! that it contains the crashing thread, its stack and the exception.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, unsafe_code)]

use std::{os::unix::process::ExitStatusExt, path::Path, process::Command};

use ghostty_crash::{
    Options,
    envelope::{Envelope, Item, ItemType},
};
use minidump::{
    Minidump,
    MinidumpException,
    MinidumpMemoryList,
    MinidumpModuleList,
    MinidumpSystemInfo,
    MinidumpThreadList,
    Module,
};

fn child_main(mode: &str) {
    let dir = std::env::var("GHOSTTY_CRASH_DIR").unwrap();
    let tid_file = std::env::var("GHOSTTY_MD_TID").unwrap();
    ghostty_crash::install(&Options {
        dir: Some(dir.into()),
        ..Options::default()
    })
    .unwrap();
    let mode = mode.to_owned();
    // Crash on a non-main thread so "the crashing thread" is distinguishable
    // from the thread that is merely waiting.
    let h = std::thread::Builder::new()
        .name("crasher".into())
        .spawn(move || {
            // SAFETY: gettid has no preconditions.
            let tid = unsafe { libc::syscall(libc::SYS_gettid) };
            std::fs::write(tid_file, tid.to_string()).unwrap();
            match mode.as_str() {
                "segv" => {
                    // SAFETY: intentionally invalid write to fault.
                    unsafe { std::ptr::write_volatile(8 as *mut u8, 1) };
                }
                "abort" => std::process::abort(),
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
                _ => unreachable!(),
            }
        })
        .unwrap();
    let _ = h.join();
    std::thread::sleep(std::time::Duration::from_secs(30));
}

#[test]
fn minidump_child() {
    if let Ok(mode) = std::env::var("GHOSTTY_MD_CHILD") {
        child_main(&mode);
    }
}

/// Run the crashing child; returns (exit status, crashing tid, temp dir).
fn crash(mode: &str) -> (std::process::ExitStatus, i32, tempfile::TempDir) {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("crash");
    let tid_file = t.path().join("tid");
    let st = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "minidump_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("GHOSTTY_MD_CHILD", mode)
        .env("GHOSTTY_CRASH_DIR", &dir)
        .env("GHOSTTY_MD_TID", &tid_file)
        .env("RUST_BACKTRACE", "0")
        .status()
        .unwrap();
    let tid = std::fs::read_to_string(tid_file).unwrap().parse().unwrap();
    (st, tid, t)
}

fn only_report(dir: &Path) -> Vec<u8> {
    let files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(1, files.len(), "{files:?}");
    std::fs::read(&files[0]).unwrap()
}

fn check(mode: &str, signal: i32) {
    let (st, tid, t) = crash(mode);
    assert_eq!(Some(signal), st.signal(), "{st:?}");

    // Envelope: event + minidump attachment.
    let data = only_report(&t.path().join("crash"));
    let mut env = Envelope::parse_slice(&data).unwrap();
    assert_eq!(2, env.items.len(), "no minidump attached");
    assert_eq!(ItemType::Event, env.items[0].item_type());
    env.items[1].decode().unwrap();
    let Item::Attachment(a) = &env.items[1] else {
        panic!("second item is not an attachment")
    };
    assert_eq!("minidump.dmp", a.filename);
    assert_eq!(Some("event.minidump"), a.attachment_type.as_deref());

    // The minidump parses.
    let dump = Minidump::read(a.payload.clone()).expect("minidump parses");

    // The crashing thread is in the thread list, with stack memory.
    let threads = dump.get_stream::<MinidumpThreadList>().unwrap();
    let ids: Vec<u32> = threads.threads.iter().map(|t| t.raw.thread_id).collect();
    let crasher = threads
        .threads
        .iter()
        .find(|th| th.raw.thread_id == tid as u32)
        .unwrap_or_else(|| panic!("tid {tid} not in {ids:?}"));
    assert!(crasher.raw.stack.memory.data_size > 0, "no stack captured");
    assert!(ids.len() >= 2, "main thread missing: {ids:?}");

    // The exception stream blames that thread and carries registers + signal.
    let exc = dump.get_stream::<MinidumpException>().unwrap();
    assert_eq!(tid as u32, exc.get_crashing_thread_id());
    assert_eq!(signal as u32, exc.raw.exception_record.exception_code);
    let sys = dump.get_stream::<MinidumpSystemInfo>().unwrap();
    let ctx = exc
        .context(&sys, None)
        .expect("exception context (registers)");
    assert!(ctx.valid_registers().count() > 10);

    // Modules and memory were captured (needed to symbolicate/stack-walk).
    let modules = dump.get_stream::<MinidumpModuleList>().unwrap();
    assert!(modules.iter().count() > 1);
    assert!(modules.iter().any(|m| m.code_file().contains("minidump")));
    let mem = dump.get_stream::<MinidumpMemoryList>().unwrap();
    assert!(mem.iter().count() > 0);
}

#[test]
fn sigsegv_report_contains_minidump() {
    check("segv", libc::SIGSEGV);
}

#[test]
fn abort_report_contains_minidump() {
    check("abort", libc::SIGABRT);
}

#[test]
fn stack_overflow_report_contains_minidump() {
    check("overflow", libc::SIGSEGV);
}
