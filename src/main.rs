//! The main entrypoint for the `ghostty` application (port of
//! `main_ghostty.zig` + `build_config.zig`).
//!
//! If the command line contains a `+action` it is dispatched to
//! `ghostty-cli`; otherwise the configuration is loaded by the GTK runtime
//! and the terminal application is run with the real core. Without GTK
//! (Zig `app_runtime == .none`: macOS, Windows) only actions are supported.

use std::process::ExitCode;
#[cfg(gtk)]
use std::rc::Rc;

#[cfg(gtk)]
use ghostty_apprt::ipc;
#[cfg(gtk)]
use ghostty_apprt_gtk::{adw_version, gtk_version};
use ghostty_cli::build_info::{self, BuildInfo};
#[cfg(gtk)]
use ghostty_cli::build_info::Version;
use ghostty_core::global::{self, InitOptions};

fn main() -> ExitCode {
    let code = real_main();
    // Flush/disable crash capture (Zig: `state.deinit()`).
    global::deinit();
    code
}

fn real_main() -> ExitCode {
    // Raw argv bytes (Zig `[]const u8`); `std::env::args()` would panic on
    // non-UTF-8 arguments and a lossy conversion would change them.
    let args: Vec<std::ffi::OsString> = ghostty_os::args::collect_os();

    // We first start by initializing our global state. This sets up
    // process-level state we need to run the terminal. We detect whether a
    // CLI action was requested so logging can be silenced for it.
    let action_requested = !matches!(ghostty_cli::ghostty::detect_os(&args), Ok(None));
    global::init(InitOptions {
        action: action_requested,
        #[cfg(gtk)]
        resources_dir: Some(ghostty_apprt_gtk::flatpak::resources_dir),
        ..InitOptions::default()
    });
    // Zig `global.init` runs `ensureLocale` before anything locale dependent.
    #[cfg(gtk)]
    ghostty_apprt_gtk::class::application::init_locale();
    #[cfg(not(gtk))]
    if let Some(locale) = ghostty_os::ensure_locale() {
        log::debug!("locale set locale={locale}");
    }
    build_info::set(build_info());

    // Execute our action if we have one.
    if action_requested {
        // Without a runtime there is no IPC handler; `+new-window` then
        // reports it as unsupported, like Zig's `apprt.none`.
        #[cfg(gtk)]
        ghostty_cli::new_window::set_ipc_handler(new_window_ipc);
        if let Some(code) = ghostty_cli::ghostty::run_from_os_args(&args) {
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

/// Zig `app_runtime == .none`: there is no terminal to launch, so print the
/// helper CLI usage (`main_ghostty.zig`).
#[cfg(not(gtk))]
fn run_app() -> ExitCode {
    print!(
        "Usage: ghostty +<action> [flags]

This is the Ghostty helper CLI that accompanies the graphical Ghostty app.
To launch the terminal directly, please launch the graphical app
(i.e. Ghostty.app on macOS). This CLI can be used to perform various
actions such as inspecting the version, listing fonts, etc.

On macOS, the terminal can also be launched using `open -na Ghostty.app`,
or `open -na Ghostty.app --args --foo=bar --baz=qux` to pass arguments.

We don't have proper help output yet, sorry! Please refer to the
source code or Discord community for help for now. We'll fix this in time.
"
    );
    ExitCode::SUCCESS
}

/// Create the core app and the GTK runtime and run the event loop.
#[cfg(gtk)]
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
#[cfg(gtk)]
fn new_window_ipc(target: &ipc::Target, action: &ipc::Action) -> Result<bool, String> {
    match ghostty_apprt_gtk::app::App::perform_ipc(target, action) {
        Ok(v) => Ok(v),
        Err(ipc::Error::IpcFailed) => Err(ghostty_cli::new_window::IPC_FAILED.to_owned()),
    }
}

/// Build information for `+version` (Zig: `build_config.zig`).
#[cfg(not(gtk))]
fn build_info() -> BuildInfo {
    BuildInfo::default()
}

/// Build information for `+version` (Zig: `build_config.zig`).
#[cfg(gtk)]
fn build_info() -> BuildInfo {
    let version = |(major, minor, micro): (u32, u32, u32)| Version {
        major,
        minor,
        micro,
    };
    let gtk = gtk_version::get_runtime_version();
    let adw = adw_version::get_runtime_version();
    BuildInfo {
        gtk_build: Some(version(gtk_version::COMPTIME_VERSION)),
        gtk_runtime: Some(version((gtk.0, gtk.1, gtk.2))),
        adw_build: Some(version(adw_version::COMPTIME_VERSION)),
        adw_runtime: Some(version((adw.0, adw.1, adw.2))),
        ..BuildInfo::default()
    }
}
