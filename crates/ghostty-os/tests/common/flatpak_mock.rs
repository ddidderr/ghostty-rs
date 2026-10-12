//! A mock `org.freedesktop.Flatpak` session bus service implementing the
//! `org.freedesktop.Flatpak.Development` interface (HostCommand,
//! HostCommandSignal, HostCommandExited), used by the Flatpak host command
//! integration tests. It really runs the requested command locally (with the
//! received stdio fds), so the whole path can be observed end to end.
//!
//! Shared by `ghostty-os` and `ghostty-termio` tests via `#[path]`.
#![allow(dead_code, clippy::unwrap_used, clippy::needless_pass_by_value)]

use std::{
    collections::HashMap,
    ffi::OsStr,
    os::{
        fd::OwnedFd,
        unix::{
            ffi::OsStrExt,
            process::{CommandExt, ExitStatusExt},
        },
    },
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use zbus::{blocking::Connection, zvariant};

pub const BUS_NAME: &str = "org.freedesktop.Flatpak";
pub const OBJECT_PATH: &str = "/org/freedesktop/Flatpak/Development";
pub const INTERFACE: &str = "org.freedesktop.Flatpak.Development";

/// What the mock saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawned {
    pub pid: u32,
    pub argv: Vec<String>,
    pub cwd: String,
    pub env: HashMap<String, String>,
    pub fds: Vec<u32>,
}

#[derive(Default)]
pub struct Record {
    pub spawned: Vec<Spawned>,
    /// (pid, signal, to_process_group)
    pub signals: Vec<(u32, u32, bool)>,
}

pub struct Service {
    pub record: Arc<Mutex<Record>>,
    // Keeps the bus name and the object server alive.
    _conn: Connection,
}

struct Dev {
    record: Arc<Mutex<Record>>,
    conn: Connection,
}

#[zbus::interface(name = "org.freedesktop.Flatpak.Development")]
impl Dev {
    fn host_command(
        &self,
        cwd_path: Vec<u8>,
        argv: Vec<Vec<u8>>,
        fds: HashMap<u32, zvariant::OwnedFd>,
        envs: HashMap<String, String>,
        _flags: u32,
    ) -> zbus::fdo::Result<u32> {
        let strip = |v: &[u8]| -> String {
            let v = v.strip_suffix(&[0]).unwrap_or(v);
            String::from_utf8_lossy(v).into_owned()
        };
        let argv: Vec<String> = argv.iter().map(|a| strip(a)).collect();
        let cwd = strip(&cwd_path);
        let fd_keys: Vec<u32> = {
            let mut k: Vec<u32> = fds.keys().copied().collect();
            k.sort_unstable();
            k
        };
        let mut fds = fds;
        let mut take = |i: u32| -> Stdio {
            fds.remove(&i)
                .map_or_else(Stdio::null, |fd| Stdio::from(OwnedFd::from(fd)))
        };

        let mut cmd = Command::new(OsStr::from_bytes(argv[0].as_bytes()));
        cmd.args(&argv[1..])
            .env_clear()
            .envs(&envs)
            .current_dir(&cwd)
            .stdin(take(0))
            .stdout(take(1))
            .stderr(take(2))
            // The real service runs the command in its own process group
            // so that `to_process_group` signals reach the whole group.
            .process_group(0);
        let mut child = cmd
            .spawn()
            .map_err(|e| zbus::fdo::Error::Failed(format!("spawn {}: {e}", argv[0])))?;
        let pid = child.id();
        self.record.lock().unwrap().spawned.push(Spawned {
            pid,
            argv,
            cwd,
            env: envs,
            fds: fd_keys,
        });

        // Report the exit like the portal: raw wait status.
        let conn = self.conn.clone();
        std::thread::spawn(move || {
            let status = child.wait().unwrap();
            let _ = conn.emit_signal(
                None::<&str>,
                OBJECT_PATH,
                INTERFACE,
                "HostCommandExited",
                &(pid, status.into_raw() as u32),
            );
        });
        Ok(pid)
    }

    fn host_command_signal(&self, pid: u32, signal: u32, to_process_group: bool) {
        self.record
            .lock()
            .unwrap()
            .signals
            .push((pid, signal, to_process_group));
        let (Some(rpid), Some(sig)) = (
            rustix::process::Pid::from_raw(pid as i32),
            rustix::process::Signal::from_named_raw(signal as i32),
        ) else {
            return;
        };
        // The child leads its own group (see above).
        let _ = if to_process_group {
            rustix::process::kill_process_group(rpid, sig)
        } else {
            rustix::process::kill_process(rpid, sig)
        };
    }
}

impl Service {
    /// Claim `org.freedesktop.Flatpak` on the session bus.
    pub fn start() -> Self {
        let record = Arc::new(Mutex::new(Record::default()));
        let conn = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name(BUS_NAME)
            .unwrap()
            .build()
            .unwrap();
        let dev = Dev {
            record: Arc::clone(&record),
            conn: conn.clone(),
        };
        conn.object_server().at(OBJECT_PATH, dev).unwrap();
        Self {
            record,
            _conn: conn,
        }
    }

    pub fn signals(&self) -> Vec<(u32, u32, bool)> {
        self.record.lock().unwrap().signals.clone()
    }

    pub fn spawned(&self) -> Vec<Spawned> {
        self.record.lock().unwrap().spawned.clone()
    }
}

/// Run `test_name` of the current test binary under `dbus-run-session`
/// (once; the inner run sees `GHOSTTY_FLATPAK_MOCK_INNER=1`). Returns true if
/// the caller is the inner process and should run the real test body.
/// Returns false when the outer process ran the inner one (and asserted it
/// passed) or when `dbus-run-session` is unavailable.
pub fn under_dbus_session(test_name: &str) -> bool {
    if std::env::var_os("GHOSTTY_FLATPAK_MOCK_INNER").is_some() {
        return true;
    }
    let exe = std::env::current_exe().unwrap();
    let out = match Command::new("dbus-run-session")
        .arg("--")
        .arg(exe)
        .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
        .env("GHOSTTY_FLATPAK_MOCK_INNER", "1")
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("dbus-run-session unavailable ({e}); skipping {test_name}");
            return false;
        }
    };
    assert!(
        out.status.success(),
        "inner test failed\n--- stdout\n{}\n--- stderr\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    false
}
