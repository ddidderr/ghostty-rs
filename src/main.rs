//! The main entrypoint for the `ghostty` application (port of
//! `main_ghostty.zig` + `build_config.zig`).
//!
//! If the command line contains a `+action` it is dispatched to
//! `ghostty-cli`; otherwise the configuration is loaded by the GTK runtime
//! and the terminal application is run with the real core.

use std::process::ExitCode;
use std::rc::Rc;

use ghostty_apprt::ipc;
use ghostty_apprt_gtk::{adw_version, gtk_version};
use ghostty_cli::build_info::{self, BuildInfo, Version};
use ghostty_core::global::{self, InitOptions};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    // We first start by initializing our global state. This sets up
    // process-level state we need to run the terminal. We detect whether a
    // CLI action was requested so logging can be silenced for it.
    let action_requested = !matches!(ghostty_cli::ghostty::detect(&args), Ok(None));
    global::init(InitOptions {
        action: action_requested,
        resources_dir: Some(ghostty_apprt_gtk::flatpak::resources_dir),
    });
    // Zig `global.init` runs `ensureLocale` before anything locale dependent.
    ghostty_apprt_gtk::class::application::init_locale();
    build_info::set(build_info());

    // Execute our action if we have one.
    if action_requested {
        ghostty_cli::new_window::set_ipc_handler(new_window_ipc);
        if let Some(code) = ghostty_cli::run_from_args(&args) {
            return ExitCode::from(code);
        }
    }

    if cfg!(debug_assertions) {
        log::warn!("This is a debug build. Performance will be very poor.");
        log::warn!("You should only use a debug build for developing Ghostty.");
        log::warn!("Otherwise, please rebuild in a release mode.");
    }

    run_app()
}

/// Create the core app and the GTK runtime and run the event loop.
fn run_app() -> ExitCode {
    // The waker lets core threads (renderer, IO) wake the GTK main loop.
    let waker = ghostty_apprt_gtk::app::waker();

    // Create our app state
    let core = match ghostty_core::App::new(waker) {
        Ok(core) => Rc::new(core),
        Err(err) => {
            log::error!("failed to create the app err={err}");
            return ExitCode::FAILURE;
        }
    };

    // Create our runtime app
    let app = ghostty_apprt_gtk::app::App::new(core);

    // Run the GUI event loop
    let result = app.run();
    app.terminate();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("app runtime failed err={err:?}");
            ExitCode::FAILURE
        }
    }
}

/// `+new-window`: send the IPC through the GTK runtime.
fn new_window_ipc(target: &ipc::Target, action: &ipc::Action) -> Result<bool, String> {
    match ghostty_apprt_gtk::app::App::perform_ipc(target, action) {
        Ok(v) => Ok(v),
        Err(ipc::Error::IpcFailed) => Err(ghostty_cli::new_window::IPC_FAILED.to_owned()),
    }
}

/// Build information for `+version` (Zig: `build_config.zig`).
fn build_info() -> BuildInfo {
    let version = |(major, minor, micro): (u32, u32, u32)| Version {
        major,
        minor,
        micro,
    };
    let gtk = gtk_version::get_runtime_version();
    let adw = adw_version::get_runtime_version();
    BuildInfo {
        gtk_build: Some(Version {
            major: 4,
            minor: 14,
            micro: 0,
        }),
        gtk_runtime: Some(version((gtk.0, gtk.1, gtk.2))),
        adw_build: Some(Version {
            major: 1,
            minor: 5,
            micro: 0,
        }),
        adw_runtime: Some(version((adw.0, adw.1, adw.2))),
        ..BuildInfo::default()
    }
}
