//! End-to-end runner for the CLI actions:
//! `cargo run -p ghostty-cli --example cli -- +list-actions --docs`
//!
//! Behaves like `ghostty` when an action is given (including `--version`,
//! `--help` and `-h`), and otherwise prints a note.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = ghostty_cli::run_from_args(&args) {
        std::process::exit(i32::from(code));
    }
    eprintln!("no +action given (this example only runs CLI actions)");
    std::process::exit(2);
}
