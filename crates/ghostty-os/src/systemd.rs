use std::io::Read;

/// Kernel `TASK_COMM_LEN`.
const TASK_COMM_LEN: u64 = 16;

/// Returns true if the program was launched as a systemd service.
///
/// On Linux, this returns true if the program was launched as a systemd
/// service. It will return false if Ghostty was launched any other way.
pub fn launched_by_systemd() -> bool {
    // On Linux, systemd sets the `INVOCATION_ID` (v232+) and the
    // `JOURNAL_STREAM` (v231+) environment variables. If these
    // environment variables are not present we were not launched by
    // systemd.
    if std::env::var_os("INVOCATION_ID").is_none() {
        return false;
    }
    if std::env::var_os("JOURNAL_STREAM").is_none() {
        return false;
    }

    // If `INVOCATION_ID` and `JOURNAL_STREAM` are present, check to make sure
    // that our parent process is actually `systemd`, not some other terminal
    // emulator that doesn't clean up those environment variables.
    let ppid = rustix::process::getppid().map_or(0, rustix::process::Pid::as_raw_pid);
    if ppid == 1 {
        return true;
    }

    // If the parent PID is not 1 we need to check to see if we were launched by
    // a user systemd daemon. Do that by checking the `/proc/<ppid>/comm`
    // to see if it ends with `systemd`.
    let comm_path = format!("/proc/{ppid}/comm");
    let Ok(comm_file) = std::fs::File::open(&comm_path) else {
        log::error!("unable to open '{comm_path}' for reading");
        return false;
    };

    // The maximum length of the command name is defined by
    // `TASK_COMM_LEN` in the Linux kernel. This is usually 16
    // bytes at the time of writing (Jun 2025) so its set to that.
    // Also, since we only care to compare to "systemd", anything
    // longer can be assumed to not be systemd.
    let mut comm_data = Vec::new();
    if comm_file
        .take(TASK_COMM_LEN)
        .read_to_end(&mut comm_data)
        .is_err()
    {
        log::error!("problems reading from '{comm_path}'");
        return false;
    }

    let end = comm_data
        .iter()
        .rposition(|&b| b != b'\n')
        .map_or(0, |i| i + 1);
    &comm_data[..end] == b"systemd"
}

/// systemd notifications. Used by Ghostty to inform systemd of the state of the
/// process. Currently only used to notify systemd that we are ready and that
/// configuration reloading has started.
///
/// See: https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html
///
/// These functions are re-implemented (rather than using `libsystemd`) to
/// avoid another external dependency. Linux systems that do not use
/// `systemd` should never start Ghostty with the `NOTIFY_SOCKET` environment
/// variable set and these functions essentially become a no-op.
pub mod notify {
    use rustix::net::{
        AddressFamily,
        SendFlags,
        SocketAddrUnix,
        SocketFlags,
        SocketType,
        connect,
        send,
        socket_with,
    };

    /// Send the given message to the UNIX socket specified in the NOTIFY_SOCKET
    /// environment variable. If there NOTIFY_SOCKET environment variable does
    /// not exist then no message is sent.
    pub(super) fn send_message(message: &[u8]) {
        // Get the socket address that should receive notifications.
        let Some(socket_path) = std::env::var_os("NOTIFY_SOCKET") else {
            return;
        };
        let socket_path = socket_path.as_encoded_bytes();

        // If the socket address is an empty string return.
        if socket_path.is_empty() {
            return;
        }

        // The socket address must be a path or an abstract socket.
        if socket_path[0] != b'/' && socket_path[0] != b'@' {
            log::warn!(
                "only AF_UNIX sockets with path or abstract namespace addresses are supported!"
            );
            return;
        }

        // sockaddr_un.path is 108 bytes.
        if 108 < socket_path.len() {
            log::warn!("NOTIFY_SOCKET path is too long!");
            return;
        }

        // NOTE: the Zig version copies the path verbatim (including a
        // leading '@'); the sd_notify protocol says a leading '@' denotes
        // an abstract socket, which is what we do here.
        let addr = if socket_path[0] == b'@' {
            SocketAddrUnix::new_abstract_name(&socket_path[1..])
        } else {
            SocketAddrUnix::new(socket_path)
        };
        let addr = match addr {
            Ok(addr) => addr,
            Err(e) => {
                log::warn!("invalid NOTIFY_SOCKET address: {e}");
                return;
            }
        };

        let sock = match socket_with(
            AddressFamily::UNIX,
            SocketType::DGRAM,
            SocketFlags::CLOEXEC,
            None,
        ) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("creating socket failed: {e}");
                return;
            }
        };

        if let Err(e) = connect(&sock, &addr) {
            log::warn!("unable to connect to notify socket: {e}");
            return;
        }

        match send(&sock, message, SendFlags::empty()) {
            Ok(written) if written < message.len() => {
                log::warn!(
                    "short write to notify socket: {written} < {}",
                    message.len()
                );
            }
            Ok(_) => {}
            Err(e) => log::warn!("unable to write to notify socket: {e}"),
        }
    }

    /// Tell systemd that we are ready or that we are finished reloading.
    /// See: https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html#READY=1
    pub fn ready() {
        send_message(b"READY=1");
    }

    /// Tell systemd that we have started reloading our configuration.
    /// See: https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html#RELOADING=1
    /// and: https://www.freedesktop.org/software/systemd/man/latest/sd_notify.html#MONOTONIC_USEC=%E2%80%A6
    pub fn reloading() {
        let ts = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        let now = ts.tv_sec * 1_000_000 + ts.tv_nsec / 1000;
        let message = format!("RELOADING=1\nMONOTONIC_USEC={now}");
        send_message(message.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixDatagram;

    use super::*;

    #[test]
    fn not_launched_without_env() {
        temp_env::with_vars(
            [("INVOCATION_ID", None::<&str>), ("JOURNAL_STREAM", None)],
            || assert!(!launched_by_systemd()),
        );
        temp_env::with_vars(
            [("INVOCATION_ID", Some("x")), ("JOURNAL_STREAM", None)],
            || assert!(!launched_by_systemd()),
        );
    }

    fn recv_with_socket(f: impl FnOnce()) -> String {
        let dir = crate::TempDir::init().unwrap();
        let path = dir.path().join("notify.sock");
        let sock = UnixDatagram::bind(&path).unwrap();
        sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        temp_env::with_var("NOTIFY_SOCKET", Some(path.to_str().unwrap()), f);
        let mut buf = [0u8; 128];
        let n = sock.recv(&mut buf).unwrap();
        String::from_utf8(buf[..n].to_vec()).unwrap()
    }

    #[test]
    fn notify_ready_sends_message() {
        assert_eq!(recv_with_socket(notify::ready), "READY=1");
    }

    #[test]
    fn notify_reloading_sends_message() {
        let msg = recv_with_socket(notify::reloading);
        let rest = msg.strip_prefix("RELOADING=1\nMONOTONIC_USEC=").unwrap();
        assert!(rest.parse::<i64>().unwrap() > 0);
    }

    #[test]
    fn notify_without_socket_is_noop() {
        temp_env::with_var("NOTIFY_SOCKET", None::<&str>, notify::ready);
        temp_env::with_var("NOTIFY_SOCKET", Some(""), notify::ready);
        temp_env::with_var("NOTIFY_SOCKET", Some("relative"), notify::ready);
    }
}
