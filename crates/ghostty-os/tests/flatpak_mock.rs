#![cfg(unix)]
#![allow(clippy::unwrap_used)]
//! `FlatpakHostCommand` against a mock `org.freedesktop.Flatpak.Development`
//! DBus service on a private session bus (`dbus-run-session`).

mod common {
    pub mod flatpak_mock;
}

use std::{
    io::Read,
    os::fd::OwnedFd,
    time::{Duration, Instant},
};

use common::flatpak_mock::Service;
use ghostty_os::flatpak::{self, Error, FlatpakHostCommand};

fn devnull() -> OwnedFd {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
        .unwrap()
        .into()
}

#[test]
fn host_command_spawn_exit_status_and_signal() {
    if !common::flatpak_mock::under_dbus_session("host_command_spawn_exit_status_and_signal") {
        return;
    }

    // The injection seam: pretend /.flatpak-info exists.
    let info = std::env::temp_dir().join(format!("flatpak-info-{}", std::process::id()));
    std::fs::write(&info, "[Application]\nname=com.mitchellh.ghostty\n").unwrap();
    assert!(flatpak::set_flatpak_info_path(&info));
    assert!(flatpak::is_flatpak());

    let svc = Service::start();

    // 1. Spawn returns the host pid; stdout goes through the passed fd;
    //    cwd and env arrive; the exit status is propagated.
    {
        let (r, w) = ghostty_os::pipe::pipe().unwrap();
        let mut cmd = FlatpakHostCommand::new(
            vec![
                "/bin/sh".into(),
                "-c".into(),
                "echo \"$FOO:$(pwd)\"; exit 7".into(),
            ],
            devnull(),
            w,
            devnull(),
        );
        cmd.cwd = Some(ghostty_os::osbytes::EncodedOs::from_encoded("/"));
        let mut env = ghostty_os::env::EnvMap::default();
        env.insert("FOO", "bar");
        cmd.env = Some(env);
        let pid = cmd.spawn().unwrap();
        assert!(pid > 1);
        assert_eq!(cmd.wait(), Ok(7));
        // wait() resets the state: a second wait is an error.
        assert_eq!(cmd.wait(), Err(Error::FlatpakMustBeStarted));
        drop(cmd); // closes our write end so the read below sees EOF
        let mut out = String::new();
        std::fs::File::from(r).read_to_string(&mut out).unwrap();
        assert_eq!(out, "bar:/\n");

        let spawned = svc.spawned();
        assert_eq!(spawned.len(), 1);
        assert_eq!(spawned[0].pid, pid);
        assert_eq!(spawned[0].argv[0], "/bin/sh");
        assert_eq!(spawned[0].cwd, "/");
        assert_eq!(spawned[0].env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(spawned[0].fds, vec![0, 1, 2]);
    }

    // 2. Exit code 0 and the async wait path.
    {
        let mut cmd = FlatpakHostCommand::new(
            vec!["/bin/sh".into(), "-c".into(), ":".into()],
            devnull(),
            devnull(),
            devnull(),
        );
        cmd.cwd = Some(ghostty_os::osbytes::EncodedOs::from_encoded("/"));
        cmd.spawn().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        cmd.wait_callback(move |r| tx.send(r).unwrap()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), Ok(0));
    }

    // 3. Spawn failure on the host is reported.
    {
        let mut cmd = FlatpakHostCommand::new(
            vec!["/nonexistent/binary".into()],
            devnull(),
            devnull(),
            devnull(),
        );
        cmd.cwd = Some(ghostty_os::osbytes::EncodedOs::from_encoded("/"));
        assert_eq!(cmd.spawn(), Err(Error::FlatpakSpawnFail));
    }

    // 4. SIGHUP to the process group (what Exec::stop sends) reaches the
    //    host command, which then exits; the signal handle works after the
    //    command moved into the exit watcher.
    {
        let mut cmd = FlatpakHostCommand::new(
            vec!["/bin/sleep".into(), "30".into()],
            devnull(),
            devnull(),
            devnull(),
        );
        cmd.cwd = Some(ghostty_os::osbytes::EncodedOs::from_encoded("/"));
        let pid = cmd.spawn().unwrap();
        let handle = cmd.signal_handle();
        let (tx, rx) = std::sync::mpsc::channel();
        cmd.wait_callback(move |r| tx.send(r).unwrap()).unwrap();
        let start = Instant::now();
        assert_eq!(handle.signal(1, true), Ok(()));
        let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(r.is_ok(), "{r:?}");
        assert!(start.elapsed() < Duration::from_secs(10));
        assert_eq!(svc.signals(), vec![(pid, 1, true)]);
    }

    let _ = std::fs::remove_file(&info);
}
