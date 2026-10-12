//! `ghostty-build-data`: prints generated build data to stdout
//! (port of `src/main_build_data.zig`).
//!
//! Usage: `ghostty-build-data +bash|+zsh|+fish|+sublime|+vim-syntax|
//! +vim-ftdetect|+vim-ftplugin|+vim-compiler|+terminfo|+mdgen-ghostty-1|
//! +mdgen-ghostty-5`

use std::io::Write;
use std::process::ExitCode;

use ghostty_build_data::Mode;

fn main() -> ExitCode {
    let Some(mode) = std::env::args().nth(1).and_then(|a| Mode::parse(&a)) else {
        let names: Vec<_> = Mode::ALL.iter().map(|(n, _)| format!("+{n}")).collect();
        eprintln!("usage: ghostty-build-data <{}>", names.join("|"));
        return ExitCode::FAILURE;
    };
    let out = mode.generate();
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(out.as_bytes())
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
