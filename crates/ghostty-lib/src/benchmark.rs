//! `ghostty_benchmark_cli` (ZIG/src/benchmark/CApi.zig).
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char};

use ghostty_bench::{args::Args, benches};

/// Split the argument string like `std.process.ArgIteratorGeneral(.{})`:
/// whitespace separated, with double quotes grouping and backslash
/// escapes inside quotes.
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut in_quote = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quote = !in_quote;
                have = true;
            }
            '\\' if in_quote => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !in_quote => {
                if have {
                    out.push(std::mem::take(&mut cur));
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
        out.push(cur);
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
    let argv = split_args(&args.to_string_lossy());
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
        assert_eq!(split_args("a b  c"), ["a", "b", "c"]);
        assert_eq!(split_args("--x=\"a b\" c"), ["--x=a b", "c"]);
        assert_eq!(split_args("   ").len(), 0);
    }

    #[test]
    fn unknown_action_returns_false() {
        // SAFETY: valid NUL terminated strings.
        assert!(!unsafe { ghostty_benchmark_cli(c"nope".as_ptr(), c"".as_ptr()) });
    }
}
