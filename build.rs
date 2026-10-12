//! Link the final binary with lazy symbol binding.
//!
//! Rust links executables with `-z now` (eager binding) by default. The GTK
//! apprt compiles in code for newer GTK/libadwaita API (e.g. the GTK 4.16 CSS
//! diagnostics) that is only ever called after a runtime `at_least` check,
//! exactly like the Zig build, which links GTK dynamically with lazy binding.
//! With eager binding the dynamic loader would refuse to start the program on
//! an older library that merely lacks such a symbol; with lazy binding the
//! symbol is only resolved if the guarded code actually runs.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,-z,lazy");
    }
}
