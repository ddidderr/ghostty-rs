//! `ghostty_benchmark_cli` (ZIG/src/benchmark/CApi.zig).
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char};

use ghostty_bench::{args::Args, benches};

/// Split the argument string like `std.process.ArgIteratorGeneral(.{})`:
/// whitespace separated, with double quotes grouping and backslash
/// escapes inside quotes. Works on raw bytes: file names need not be UTF-8.
fn split_args(s: &[u8]) -> Vec<std::ffi::OsString> {
    let mut out = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut have = false;
    let mut in_quote = false;
    let mut bytes = s.iter().copied();
    while let Some(c) = bytes.next() {
        match c {
            b'"' => {
                in_quote = !in_quote;
                have = true;
            }
            b'\\' if in_quote => {
                if let Some(n) = bytes.next() {
                    cur.push(n);
                }
            }
            c if c.is_ascii_whitespace() && !in_quote => {
                if have {
                    out.push(ghostty_os::osbytes::os_from_bytes(std::mem::take(&mut cur)));
                    have = false;
                }
            }
            c => {
                cur.push(c);
                have = true;
            }
        }
    }
    if have {
        out.push(ghostty_os::osbytes::os_from_bytes(cur));
    }
    out
}

/// Run the Ghostty benchmark CLI with the given action and arguments.
///
/// # Safety
/// `action_name` and `args` must be NUL terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_benchmark_cli(
    action_name: *const c_char,
    args: *const c_char,
) -> bool {
    if action_name.is_null() || args.is_null() {
        return false;
    }
    // SAFETY: per the function contract.
    let (action, args) = unsafe { (CStr::from_ptr(action_name), CStr::from_ptr(args)) };
    let action = action.to_string_lossy();
    if !benches::ACTIONS.contains(&action.as_ref()) {
        log::warn!("unknown action={action}");
        return false;
    }
    let argv = split_args(args.to_bytes());
    let parsed = match Args::parse(argv) {
        Ok(a) => a,
        Err(err) => {
            log::warn!("failed to run action={action} err={err}");
            return false;
        }
    };
    match benches::main_action(&action, parsed) {
        Ok(_) => true,
        Err(err) => {
            log::warn!("failed to run action={action} err={err}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_args_basic() {
        assert_eq!(split_args(b"a b  c"), ["a", "b", "c"]);
        assert_eq!(split_args(b"--x=\"a b\" c"), ["--x=a b", "c"]);
        assert_eq!(split_args(b"   ").len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn split_args_keeps_non_utf8_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let v = split_args(b"--data=/tmp/a\xffb x");
        assert_eq!(v[0].as_bytes(), b"--data=/tmp/a\xffb");
        assert_eq!(v[1], "x");
    }

    #[test]
    fn unknown_action_returns_false() {
        // SAFETY: valid NUL terminated strings.
        assert!(!unsafe { ghostty_benchmark_cli(c"nope".as_ptr(), c"".as_ptr()) });
    }
}
