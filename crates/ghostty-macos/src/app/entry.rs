//! `main.swift`: process entry point.
//!
//! 1. `ghostty_init(argc, argv)`; on failure log, print the CLI hint when
//!    launched from a terminal, and `exit(1)`.
//! 2. `ghostty_cli_try_action()` runs a `+action` CLI command and exits.
//! 3. `NSApplicationMain`: there is no `Info.plist`-named nib here, so the nib's
//!    work is done in code: create the `NSApplication`, the `AppDelegate`
//!    (which creates `Ghostty.App`), build the menu from the xib tables
//!    (`main_menu::install`), connect the outlets, set the delegate and run.

#![allow(unsafe_code)]

use std::{
    ffi::{CString, c_char},
    io::Write,
    os::unix::ffi::OsStrExt,
};

use objc2::{MainThreadMarker, runtime::ProtocolObject};
use objc2_app_kit::NSApplication;

use super::{app_delegate::AppDelegate, main_menu};
use crate::ghostty::{
    capi::{GHOSTTY_SUCCESS, ghostty_cli_try_action, ghostty_init},
    package::{LaunchSource, launch_source},
};

/// The text printed to stderr when `ghostty_init` fails from a terminal.
pub const INIT_FAILED_MESSAGE: &str = "Ghostty failed to initialize! If you're executing Ghostty from the command line\n\
then this is usually because an invalid action or multiple actions were specified.\n\
Actions start with the `+` character.\n\n\
View all available actions by running `ghostty +help`.\n";

/// Run the app. Never returns.
pub fn run() -> ! {
    // Initialize Ghostty global state. We do this once right away because the
    // CLI APIs require it and it lets us ensure it is done immediately for the
    // rest of the app.
    let args: Vec<CString> = std::env::args_os()
        .map(|a| CString::new(a.as_bytes()).unwrap_or_default())
        .collect();
    let mut argv: Vec<*mut c_char> = args.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());
    // SAFETY: `argv` holds `args.len()` valid NUL terminated strings followed by
    // a null terminator and outlives the call.
    let rc = unsafe { ghostty_init(args.len(), argv.as_mut_ptr()) };
    if rc != GHOSTTY_SUCCESS {
        log::error!("ghostty_init failed");

        // We also write to stderr if this is executed from the CLI or zig run
        match launch_source() {
            LaunchSource::Cli | LaunchSource::ZigRun => {
                let _ = std::io::stderr().write_all(INIT_FAILED_MESSAGE.as_bytes());
                std::process::exit(1);
            }
            // For the app we exit immediately. We should handle this case more
            // gracefully in the future.
            LaunchSource::App => std::process::exit(1),
        }
    }

    // This will run the CLI action and exit if one was specified. A CLI
    // action is a command starting with a `+`, such as `ghostty +boo`.
    ghostty_cli_try_action();

    // NSApplicationMain(...)
    let mtm = MainThreadMarker::new().expect("main() runs on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let delegate = AppDelegate::new(mtm);
    let outlets = main_menu::install(mtm, &app, &delegate);
    delegate.connect_outlets(outlets);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
    // `-[NSApplication run]` only returns after `stop:`; NSApplicationMain
    // exits the process in that case.
    drop(delegate);
    std::process::exit(0);
}
