//! Integration test: run a real child attached to a pty.

#![cfg(unix)]
#![allow(clippy::unwrap_used)]

use std::{
    fs::File,
    io::Read,
    os::fd::{AsRawFd, OwnedFd},
};

use ghostty_termio::{Command, Pty, Winsize, command::Exit, pty::child_pre_exec};

fn dup(fd: &OwnedFd) -> OwnedFd {
    fd.try_clone().unwrap()
}

/// Read from the master until the child side hangs up (EIO on Linux).
fn read_until_hangup(master: OwnedFd) -> String {
    let mut f = File::from(master);
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match f.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn run_on_pty(script: &str, size: Winsize) -> (String, Exit) {
    let mut pty = Pty::open(size).unwrap();
    let slave = pty.slave.take().unwrap();

    let (master_raw, _) = pty.raw_fds();
    let slave_raw = slave.as_raw_fd();

    let mut cmd = Command::new("/bin/sh", &["/bin/sh", "-c", script]);
    cmd.stdin = Some(dup(&slave));
    cmd.stdout = Some(dup(&slave));
    cmd.stderr = Some(dup(&slave));
    cmd.os_pre_exec = Some(Box::new(move || {
        // Make the slave our controlling terminal in a new session; this
        // also closes the master/slave descriptors in the child.
        match child_pre_exec(master_raw, slave_raw) {
            Ok(()) => None,
            Err(_) => Some(1),
        }
    }));
    cmd.start().unwrap();

    // Close our copies of the slave so we see a hangup when the child exits.
    drop(cmd.stdin.take());
    drop(cmd.stdout.take());
    drop(cmd.stderr.take());
    drop(slave);

    let exit = cmd.wait(true).unwrap();
    let out = read_until_hangup(pty.master);
    (out, exit)
}

#[test]
fn echo_and_stty_size_on_pty() {
    let size = Winsize {
        ws_row: 31,
        ws_col: 97,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let (out, exit) = run_on_pty("echo hi; stty size", size);
    assert_eq!(exit, Exit::Exited(0));
    // A pty translates \n to \r\n.
    assert_eq!(out, "hi\r\n31 97\r\n");
}

#[test]
fn child_has_controlling_terminal() {
    // `tty` succeeds only with a terminal on stdin; /dev/tty only opens when
    // we have a controlling terminal (TIOCSCTTY).
    let (out, exit) = run_on_pty(
        "tty >/dev/null && echo tty; echo x > /dev/tty && echo ctty",
        Winsize::default(),
    );
    assert_eq!(exit, Exit::Exited(0));
    assert!(out.contains("tty\r\n"), "{out:?}");
    assert!(out.contains("ctty\r\n"), "{out:?}");
}

#[test]
fn resize_is_visible_to_child() {
    let size = Winsize {
        ws_row: 10,
        ws_col: 20,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let (out, _) = run_on_pty("stty size", size);
    assert_eq!(out, "10 20\r\n");
}
