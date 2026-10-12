//! `ghostty-bench +<action> [--option=value ...]` (port of `main_bench.zig`).
use ghostty_bench::{args::Args, benches};
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut action = None;
    for a in &argv {
        if let Some(name) = a.strip_prefix('+') {
            if action.is_some() {
                eprintln!("error: multiple actions specified");
                return ExitCode::FAILURE;
            }
            action = Some(name.to_owned());
        }
    }
    let Some(action) = action else {
        eprintln!(
            "error: no action specified; available: {}",
            benches::ACTIONS.join(", ")
        );
        return ExitCode::FAILURE;
    };
    let mut args = match Args::parse(argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Rust-only convenience: print the measured step time to stderr.
    let time = args.take_flag("time").unwrap_or(false);
    match benches::main_action(&action, args) {
        Ok(r) => {
            if time {
                eprintln!(
                    "iterations={} duration_ns={}",
                    r.iterations,
                    r.duration.as_nanos()
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
