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

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let ucd_dir = manifest.join("ucd");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_support/ucd.rs");
    println!("cargo:rerun-if-changed=src/lut.rs");
    println!("cargo:rerun-if-changed=src/props.rs");
    println!("cargo:rerun-if-changed=ucd");

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
}
