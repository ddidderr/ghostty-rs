//! Build metadata for `+version` and the generated docs.
//!
//! * the compiler version (Zig: `builtin.zig_version_string`)
//!
//! The app version, channel, commit and build mode live in
//! `ghostty-build-info` (single source of truth).

use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Generates the per-action help table (Zig: `helpgen.zig` `genActions`):
/// the doc comment of each action's `run` function becomes
/// `Action::help_text`.
fn gen_action_help(manifest_dir: &Path) {
    let ghostty_rs = manifest_dir.join("src/ghostty.rs");
    println!("cargo:rerun-if-changed={}", ghostty_rs.display());
    let src = std::fs::read_to_string(&ghostty_rs).expect("read ghostty.rs");
    let mut table = Vec::new();
    for (variant, _) in ghostty_helpgen::enum_variant_docs(&src, "Action") {
        // `ListFonts` -> `list_fonts` (file name, as Zig `Action.file`).
        let mut file = String::new();
        for (i, c) in variant.chars().enumerate() {
            if c.is_ascii_uppercase() && i > 0 {
                file.push('_');
            }
            file.push(c.to_ascii_lowercase());
        }
        let path = manifest_dir.join(format!("src/{file}.rs"));
        println!("cargo:rerun-if-changed={}", path.display());
        let text = std::fs::read_to_string(&path).expect("read action source");
        let help = ghostty_helpgen::run_fn_doc(&text).unwrap_or_else(|| {
            panic!("doc comment must be present on run function of the {file} action!")
        });
        table.push((variant, help));
    }
    assert!(!table.is_empty(), "no actions found");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(
        out.join("action_help.rs"),
        ghostty_helpgen::cli_help_match(&table),
    )
    .expect("write action_help.rs");
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    gen_action_help(&PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default(),
    ));
    println!("cargo:rerun-if-env-changed=RUSTC");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let version = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        // "rustc 1.99.0 (b940084d7 2026-09-28)" -> "1.99.0"
        .and_then(|s| s.split_whitespace().nth(1).map(str::to_owned))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=GHOSTTY_RUSTC_VERSION={version}");
}
