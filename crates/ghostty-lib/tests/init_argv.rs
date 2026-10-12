//! `ghostty_init(argc, argv)` semantics from `main_c.zig`: the supplied
//! argument vector (not the process arguments) feeds the later config CLI
//! loading, and SIGPIPE is ignored for C / Swift hosts.
#![cfg(unix)]
#![allow(
    unsafe_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::undocumented_unsafe_blocks
)]

use std::ffi::{CString, c_char};

use ghostty::{config_capi::*, main_c::ghostty_init, types::ghostty_config_t};

fn get_bool(cfg: ghostty_config_t, key: &str) -> bool {
    let mut out = false;
    let ok =
        unsafe { ghostty_config_get(cfg, (&raw mut out).cast(), key.as_ptr().cast(), key.len()) };
    assert!(ok);
    out
}

#[test]
fn init_argv_feeds_config_cli_and_ignores_sigpipe() {
    // Start from the default disposition so that the check is meaningful.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };

    // Not the process argv: the test harness argv has no `--maximize`.
    let owned: Vec<CString> = [&b"ghostty"[..], b"--maximize=true"]
        .iter()
        .map(|a| CString::new(*a).unwrap())
        .collect();
    let mut argv: Vec<*mut c_char> = owned.iter().map(|c| c.as_ptr().cast_mut()).collect();
    assert_eq!(unsafe { ghostty_init(argv.len(), argv.as_mut_ptr()) }, 0);

    let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
    unsafe { libc::sigaction(libc::SIGPIPE, std::ptr::null(), &mut sa) };
    assert_eq!(sa.sa_sigaction, libc::SIG_IGN);

    let cfg = ghostty_config_new();
    unsafe { ghostty_config_load_cli_args(cfg) };
    assert!(get_bool(cfg, "maximize"));
    unsafe { ghostty_config_free(cfg) };
}

#[test]
fn config_load_file_accepts_non_utf8_path() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let mut name = dir.path().as_os_str().as_bytes().to_vec();
    name.extend_from_slice(b"/cfg\xff");
    let path = std::ffi::OsStr::from_bytes(&name);
    std::fs::write(path, "maximize = true\n").unwrap();

    let cfg = ghostty_config_new();
    let c = CString::new(name).unwrap();
    unsafe { ghostty_config_load_file(cfg, c.as_ptr()) };
    assert!(get_bool(cfg, "maximize"));
    unsafe { ghostty_config_free(cfg) };
}
