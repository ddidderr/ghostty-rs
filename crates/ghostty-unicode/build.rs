//! Generates the Unicode lookup tables (Ghostty's `props_uucode.zig` /
//! `symbols_uucode.zig` generator `main`s) from the vendored UCD files in
//! `ucd/`.
#![allow(dead_code)]
#![allow(
    clippy::unwrap_used,
    clippy::single_match_else,
    clippy::checked_conversions,
    clippy::trivially_copy_pass_by_ref,
    clippy::expect_used
)]

#[path = "src/lut.rs"]
mod lut;
#[path = "src/props.rs"]
mod props;
#[path = "build_support/ucd.rs"]
mod ucd;

use std::{env, fmt::Write as _, fs, path::PathBuf};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let ucd_dir = manifest.join("ucd");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_support/ucd.rs");
    println!("cargo:rerun-if-changed=src/lut.rs");
    println!("cargo:rerun-if-changed=src/props.rs");
    println!("cargo:rerun-if-changed=ucd");
    println!("cargo:rerun-if-changed=ucd/CaseFolding.txt");

    let ucd = ucd::Ucd::load(&ucd_dir);

    let props = lut::generate(|cp| ucd.get(cp)).expect("generate props tables");
    fs::write(
        out.join("props_tables.rs"),
        props.write_rust("Properties", props::Properties::format_rust),
    )
    .unwrap();

    let symbols = lut::generate(|cp| ucd.is_symbol(cp)).expect("generate symbols tables");
    fs::write(
        out.join("symbols_tables.rs"),
        symbols.write_rust("bool", ToString::to_string),
    )
    .unwrap();

    let emoji = lut::generate(|cp| ucd.is_emoji_presentation(cp)).expect("generate emoji tables");
    fs::write(
        out.join("emoji_tables.rs"),
        emoji.write_rust("bool", ToString::to_string),
    )
    .unwrap();

    generate_case_folding(&ucd_dir, &out);
}

/// Generates `case_folding_full.rs` from `CaseFolding.txt`: the `C` (common)
/// and `F` (full) statuses, which is uucode's `case_folding_full`.
fn generate_case_folding(ucd_dir: &std::path::Path, out: &std::path::Path) {
    let text = fs::read_to_string(ucd_dir.join("CaseFolding.txt")).unwrap();
    let mut entries: Vec<(u32, Vec<u32>)> = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(';').map(str::trim).collect();
        if fields.len() < 3 || (fields[1] != "C" && fields[1] != "F") {
            continue;
        }
        let cp = u32::from_str_radix(fields[0], 16).unwrap();
        let mapping: Vec<u32> = fields[2]
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        assert!(!mapping.is_empty() && mapping.len() <= 3);
        entries.push((cp, mapping));
    }
    entries.sort_by_key(|e| e.0);
    entries.dedup_by_key(|e| e.0);

    let mut src = String::new();
    writeln!(
        src,
        "static CASE_FOLDING_FULL: [(u32, [u32; 3]); {}] = [",
        entries.len()
    )
    .unwrap();
    for (cp, m) in &entries {
        let mut a = [0u32; 3];
        a[..m.len()].copy_from_slice(m);
        writeln!(
            src,
            "    ({cp:#x}, [{:#x}, {:#x}, {:#x}]),",
            a[0], a[1], a[2]
        )
        .unwrap();
    }
    src.push_str("];\n");
    fs::write(out.join("case_folding_full.rs"), src).unwrap();
}
