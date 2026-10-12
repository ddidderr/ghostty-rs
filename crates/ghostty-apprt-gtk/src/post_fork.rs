//! Parent post-fork hook creating the transient scope (port of
//! `apprt/gtk/post_fork.zig`).

use ghostty_config::{
    config::Config,
    types_b::{GtkSingleInstance, LinuxCgroup},
};
use gtk::{gio, prelude::*};

use crate::cgroup;

/// Allowable error from a post-fork hook; fails surface creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("post fork hook failed")]
pub struct PostForkError;

/// Config values needed by [`post_fork`].
#[derive(Debug, Clone, Copy)]
pub struct PostForkInfo {
    pub gtk_single_instance: GtkSingleInstance,
    pub linux_cgroup: LinuxCgroup,
    pub linux_cgroup_hard_fail: bool,
    pub linux_cgroup_memory_limit: Option<u64>,
    pub linux_cgroup_processes_limit: Option<u64>,
}

impl PostForkInfo {
    pub fn new(cfg: &Config) -> Self {
        Self {
            gtk_single_instance: cfg.gtk_single_instance,
            linux_cgroup: cfg.linux_cgroup,
            linux_cgroup_hard_fail: cfg.linux_cgroup_hard_fail,
            linux_cgroup_memory_limit: cfg.linux_cgroup_memory_limit,
            linux_cgroup_processes_limit: cfg.linux_cgroup_processes_limit,
        }
    }
}

/// Called in the parent after fork with the child's `pid`. Must run on the
/// GTK main thread (uses the default `gio::Application` D-Bus connection)
/// or at least where the application's connection is usable.
pub fn post_fork(info: &PostForkInfo, pid: Option<u32>) -> Result<(), PostForkError> {
    match info.linux_cgroup {
        LinuxCgroup::Always => {}
        LinuxCgroup::Never => return Ok(()),
        LinuxCgroup::SingleInstance => match info.gtk_single_instance {
            GtkSingleInstance::True => {}
            GtkSingleInstance::False => return Ok(()),
            GtkSingleInstance::Detect => {
                log::error!("gtk-single-instance is set to detect which should be impossible!");
                return Err(PostForkError);
            }
        },
    }

    let Some(pid) = pid else {
        log::error!("PID of child not known!");
        return Err(PostForkError);
    };

    let expected = cgroup::fmt_scope(pid);
    log::debug!("beginning transition to transient systemd scope {expected}");

    let Some(dbus) = gio::Application::default().and_then(|a| a.dbus_connection()) else {
        if info.linux_cgroup_hard_fail {
            log::error!("dbus connection required for cgroup isolation, exiting");
            return Err(PostForkError);
        }
        return Ok(());
    };

    if let Err(err) = cgroup::create_scope(
        &dbus,
        pid,
        cgroup::Options {
            memory_high: info.linux_cgroup_memory_limit,
            tasks_max: info.linux_cgroup_processes_limit,
        },
    ) {
        if info.linux_cgroup_hard_fail {
            log::error!("unable to create transient systemd scope {expected}: {err}");
            return Err(PostForkError);
        }
        log::warn!("unable to create transient systemd scope {expected}: {err}");
        return Ok(());
    }

    if ghostty_os::cgroup::wait_for_scope(pid, &expected) {
        log::debug!("transition to transient systemd scope {expected} complete");
    } else {
        // Zig checks `rt_pre_exec_info.linux_cgroup_hard_fail` here;
        // both infos come from the same config so they're identical.
        if info.linux_cgroup_hard_fail {
            log::error!("transition to new transient systemd scope {expected} took too long");
            return Err(PostForkError);
        }
        log::warn!("transition to transient systemd scope {expected} took too long");
    }

    Ok(())
}
