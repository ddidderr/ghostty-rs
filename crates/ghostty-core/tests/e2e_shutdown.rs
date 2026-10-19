//! Exercise the actual Surface -> IO -> PTY reader join chain with a full
//! app queue. A subprocess watchdog turns a regression into a failure instead
//! of hanging the suite. This needs headless EGL (software Mesa is sufficient).
#![cfg(all(unix, not(target_vendor = "apple")))]

mod common;

use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

use common::{Harness, Opts};
use ghostty_core::app::Message;
use ghostty_datastruct::blocking_queue::Timeout;

#[test]
fn surface_shutdown_with_full_ui_mailbox() {
    shutdown_regression("surface_shutdown_with_full_ui_mailbox", false);
}

#[test]
fn surface_shutdown_with_search_and_full_ui_mailbox() {
    shutdown_regression("surface_shutdown_with_search_and_full_ui_mailbox", true);
}

fn shutdown_regression(test_name: &str, search: bool) {
    const CHILD: &str = "GHOSTTY_TEST_FULL_MAILBOX_CHILD";
    if std::env::var_os(CHILD).is_some() {
        eprintln!("shutdown regression: creating surface");
        let h = Harness::new(Opts::default().script(
            r"stty raw -echo; printf READY; while [ ! -f '{out}' ]; do sleep 0.01; done; printf '\033]0;shutdown-title\007PARSED'; exec cat >'{out}'"
        )).expect("headless EGL is required for the shutdown regression");
        eprintln!("shutdown regression: waiting for PTY readiness");
        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while !h.screen().contains("READY") {
            h.pump();
            assert!(
                Instant::now() < ready_deadline,
                "PTY did not start: screen={:?}, actions={:?}",
                h.screen(),
                h.action_names()
            );
            thread::sleep(Duration::from_millis(5));
        }
        h.settle(100);
        eprintln!("shutdown regression: filling UI queue");
        let mailbox = h.app.mailbox();
        while mailbox.push(Message::RedrawSurface(h.id), Timeout::Instant) != 0 {}
        if search {
            // Search notifications (including the final Quit callback) also
            // need cancellation before the UI joins the search thread.
            assert!(h.bind("search:READY"));
        }
        eprintln!("shutdown regression: triggering title output");
        std::fs::write(&h.out, b"go").expect("trigger child title output");
        // Output processing releases the terminal lock before delivering the
        // title outbox. Never pump the app: it must stay full during teardown.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !h.screen().contains("PARSED") {
            assert!(
                Instant::now() < deadline,
                "reader did not parse the title: {:?}",
                h.screen()
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(mailbox.push(Message::Quit, Timeout::Instant), 0);
        eprintln!("shutdown regression: dropping surface");
        drop(h); // used to wait forever for the reader's SetTitle send
        eprintln!("shutdown regression: surface joined");
        return;
    }

    let mut child = Command::new(std::env::current_exe().expect("test executable path"))
        .args(["--exact", test_name, "--nocapture"])
        .env(CHILD, "1")
        .spawn()
        .expect("spawn shutdown regression child");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().expect("poll shutdown regression child") {
            assert!(
                status.success(),
                "shutdown regression child failed: {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill timed-out regression child");
            child.wait().expect("reap timed-out regression child");
            panic!("surface shutdown deadlocked with a full UI mailbox");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
