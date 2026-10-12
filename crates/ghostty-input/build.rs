//! Generates the keybind action docs table (Zig: `helpgen.zig`
//! `genKeybindActions`) from the `///` docs of `enum Action` in
//! `src/binding/action.rs`.

use std::{env, fs, path::PathBuf};

fn main() {
    let src_path = "src/binding/action.rs";
    println!("cargo:rerun-if-changed={src_path}");
    println!("cargo:rerun-if-changed=build.rs");
    let src = fs::read_to_string(src_path).expect("read action.rs");

    let variants: Vec<(String, String)> = ghostty_helpgen::enum_variant_docs(&src, "Action")
        .into_iter()
        .map(|(name, doc)| {
            let doc = doc
                .unwrap_or_else(|| panic!("keybind action `{name}` must have a `///` doc comment"));
            (name, doc)
        })
        .collect();
    assert!(!variants.is_empty(), "no variants found in `enum Action`");

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    fs::write(
        out.join("keybind_action_docs.rs"),
        ghostty_helpgen::keybind_docs_match(&variants),
    )
    .expect("write keybind_action_docs.rs");
}
