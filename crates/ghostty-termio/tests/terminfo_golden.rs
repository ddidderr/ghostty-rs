//! The terminfo source emitted by the Rust port must be byte-identical to the

#![allow(clippy::unwrap_used)]
//! output of the Zig `terminfo/ghostty.zig` `encode` (golden file generated
//! with the Zig reference implementation).

use ghostty_termio::terminfo;

#[test]
fn terminfo_source_matches_zig_output() {
    let expected = include_str!("data/xterm-ghostty.terminfo");
    assert_eq!(expected, terminfo::ghostty.encode_to_string());
}

#[test]
fn terminfo_compiles_with_tic_if_available() {
    let Ok(dir) = std::env::var("CARGO_TARGET_TMPDIR") else {
        return;
    };
    let src = std::path::Path::new(&dir).join("xterm-ghostty.src");
    std::fs::write(&src, terminfo::ghostty.encode_to_string()).unwrap();
    let out = std::path::Path::new(&dir).join("terminfo-out");
    let _ = std::fs::create_dir_all(&out);
    let Ok(status) = std::process::Command::new("tic")
        .arg("-x")
        .arg("-o")
        .arg(&out)
        .arg(&src)
        .status()
    else {
        return; // tic not installed
    };
    assert!(status.success());
}
