//! Child pre-exec hook for cgroup transitions (port of `apprt/gtk/pre_exec.zig`).
//!
//! Runs in the child between fork and exec, so it only reads procfs and
//! sleeps. The logic is also mirrored by `ghostty-termio`'s `Command`.

use std::time::{Duration, Instant};

use ghostty_config::config::Config;
use ghostty_config::types_b::{GtkSingleInstance, LinuxCgroup};

use crate::cgroup;

/// Config values needed by [`pre_exec`].
#[derive(Debug, Clone, Copy)]
pub struct PreExecInfo {
    pub gtk_single_instance: GtkSingleInstance,
    pub linux_cgroup: LinuxCgroup,
    pub linux_cgroup_hard_fail: bool,
}

impl PreExecInfo {
    pub fn new(cfg: &Config) -> Self {
        Self {
            gtk_single_instance: cfg.gtk_single_instance,
            linux_cgroup: cfg.linux_cgroup,
            linux_cgroup_hard_fail: cfg.linux_cgroup_hard_fail,
        }
    }
}

/// Waits (up to 250ms) for the process to be moved to its transient scope.
/// Returns `Some(exit_code)` if the child must exit.
pub fn pre_exec(info: &PreExecInfo) -> Option<u8> {
    match info.linux_cgroup {
        LinuxCgroup::Always => {}
        LinuxCgroup::Never => return None,
        LinuxCgroup::SingleInstance => match info.gtk_single_instance {
            GtkSingleInstance::True => {}
            GtkSingleInstance::False => return None,
            GtkSingleInstance::Detect => {
                log::error!("gtk-single-instance is set to detect");
                return Some(127);
            }
        },
    }

    let pid = std::process::id();
    let expected = cgroup::fmt_scope(pid);
    let start = Instant::now();

    loop {
        if start.elapsed() > Duration::from_millis(250) {
            if info.linux_cgroup_hard_fail {
                log::error!("transition to new transient systemd scope took too long");
                return Some(127);
            }
            break;
        }

        if current_scope_matches(pid, &expected) {
            return None;
        }

        std::thread::sleep(Duration::from_millis(25));
    }

    None
}

/// True if the final path component of `pid`'s cgroup equals `expected`.
pub(crate) fn current_scope_matches(pid: u32, expected: &str) -> bool {
    let Some(raw) = ghostty_os::cgroup::current(pid) else {
        return false;
    };
    raw.rsplit_once('/')
        .is_some_and(|(_, last)| last == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_returns_immediately() {
        let info = PreExecInfo {
            gtk_single_instance: GtkSingleInstance::Detect,
            linux_cgroup: LinuxCgroup::Never,
            linux_cgroup_hard_fail: true,
        };
        assert_eq!(pre_exec(&info), None);
    }

    #[test]
    fn detect_fails() {
        let info = PreExecInfo {
            gtk_single_instance: GtkSingleInstance::Detect,
            linux_cgroup: LinuxCgroup::SingleInstance,
            linux_cgroup_hard_fail: false,
        };
        assert_eq!(pre_exec(&info), Some(127));
    }
}
