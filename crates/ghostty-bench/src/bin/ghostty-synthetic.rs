//! `ghostty-synthetic +<ascii|utf8|osc> [--option=value ...]`: endless
//! synthetic data on stdout (port of `synthetic/main.zig`).
use std::{
    io::{BufWriter, Write},
    process::ExitCode,
};

use ghostty_bench::{
    args::Args,
    synthetic::{OscStyle, Rng, run_ascii, run_osc, run_sgr, run_utf8},
};

fn run() -> Result<(), String> {
    let argv: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mut action = None;
    for a in &argv {
        if let Some(name) = ghostty_bench::args::action_name(a) {
            if action.is_some() {
                return Err("multiple actions specified".into());
            }
            action = Some(name);
        }
    }
    let action = action.ok_or("no action specified; available: ascii, osc, sgr, utf8")?;
    let mut args = Args::parse(argv)?;
    let mut rng = Rng::from_time();
    let mut out = BufWriter::with_capacity(2048, std::io::stdout().lock());
    let res = match action.as_str() {
        "ascii" => {
            args.finish()?;
            run_ascii(&mut out, &mut rng)
        }
        "utf8" => {
            args.finish()?;
            run_utf8(&mut out, &mut rng)
        }
        "sgr" => {
            args.finish()?;
            run_sgr(&mut out, &mut rng)
        }
        "osc" => {
            let p_valid = args.take("p-valid", 0.5f64)?;
            let style = args.take_enum(
                "style",
                OscStyle::Streaming,
                &[
                    ("streaming", OscStyle::Streaming),
                    ("parser", OscStyle::Parser),
                ],
            )?;
            args.finish()?;
            run_osc(&mut out, &mut rng, p_valid, style)
        }
        a => return Err(format!("invalid action: +{a}")),
    };
    res.map_err(|e| e.to_string())?;
    // Broken pipe on flush is a normal end.
    let _ = out.flush();
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
