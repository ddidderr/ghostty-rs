//! Builds Ghostty's unchanged C examples (examples/c/*.c, from
//! ZIG/example/c-vt*) against the Rust `libghostty-vt` and checks their output.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::process::Command;

fn run_example(name: &str) -> String {
    let lib = common::install_native_lib();
    let scratch = common::scratch_dir("c_examples");
    let src = common::manifest_dir()
        .join("examples/c")
        .join(format!("{name}.c"));
    let exe = scratch.join(name);
    common::compile_c(&lib, &[&src], &exe, &["-Wall"]);
    let out = Command::new(&exe).output().expect("run example");
    assert!(out.status.success(), "{name} exited with {:?}", out.status);
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn c_vt_osc() {
    assert_eq!(
        run_example("c-vt"),
        "Command type: 1\nExtracted title: hello\n"
    );
}

#[test]
fn c_vt_key_encode() {
    assert_eq!(
        run_example("c-vt-key-encode"),
        "Encoding event: left ctrl release with all Kitty flags enabled\n\
         Required buffer size: 12 bytes\n\
         Encoded 12 bytes\n\
         Hex: 1b 5b 35 37 34 34 32 3b 35 3a 33 75 \n\
         String: \\x1b[57442;5:3u\n"
    );
}

#[test]
fn c_vt_paste() {
    assert_eq!(
        run_example("c-vt-paste"),
        "'hello world' is safe to paste\n\
         'rm -rf /\n' is UNSAFE - contains newline\n\
         Data with escape sequence is UNSAFE\n\
         Empty data is safe\n"
    );
}

#[test]
fn c_vt_sgr() {
    assert_eq!(
        run_example("c-vt-sgr"),
        "Parsing Kakoune SGR sequence:\n\
         ESC[4:3;38;2;51;51;51;48;2;170;170;170;58;2;255;97;136m\n\
         \n\
         Attribute 1: Underline style = curly\n\
         Attribute 2: Foreground RGB = (51, 51, 51)\n\
         Attribute 3: Background RGB = (170, 170, 170)\n\
         Attribute 4: Underline color RGB = (255, 97, 136)\n\
         \n\
         Total attributes parsed: 4\n"
    );
}

/// The static archive links too (what embedders of `libghostty-vt.a` use).
#[test]
fn c_vt_links_statically() {
    let lib = common::install_native_lib();
    let scratch = common::scratch_dir("c_examples_static");
    let src = common::manifest_dir().join("examples/c/c-vt-paste.c");
    let exe = scratch.join("paste-static");
    let out = Command::new("cc")
        .arg("-I")
        .arg(common::include_dir())
        .arg(&src)
        .arg(lib.join("libghostty-vt.a"))
        .args(["-lpthread", "-ldl", "-lm"])
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(String::from_utf8_lossy(&run.stdout).contains("Empty data is safe"));
}
