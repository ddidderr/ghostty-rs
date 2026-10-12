//! Compiles `tests/c/alloc_test.c` (a counting / failing / limiting C
//! allocator driving the OSC parser) against the Rust `libghostty-vt`.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::process::Command;

#[test]
fn c_allocator_governs_osc_parser_storage() {
    let lib = common::install_native_lib();
    let scratch = common::scratch_dir("c_allocator");
    let src = common::manifest_dir().join("tests/c/alloc_test.c");
    let exe = scratch.join("alloc_test");
    // No -DNDEBUG: the test relies on assert().
    common::compile_c(&lib, &[&src], &exe, &["-Wall", "-O0"]);
    let out = Command::new(&exe).output().expect("run alloc_test");
    assert!(
        out.status.success(),
        "alloc_test failed: {:?}\n{}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "ok\n");
}
