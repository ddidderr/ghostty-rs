//! Shared helpers: build the library and find it next to the Cargo
//! artifacts, parse the unchanged `include/ghostty.h`.
#![allow(dead_code, clippy::unwrap_used)]

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

pub fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn include_dir() -> PathBuf {
    manifest_dir().join("include")
}

/// `target/<profile>`, derived from the test executable's location.
pub fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    exe.parent() // deps
        .and_then(Path::parent) // <profile>
        .expect("profile dir")
        .to_path_buf()
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

static BUILT: OnceLock<PathBuf> = OnceLock::new();

/// Build the cdylib/staticlib for the host (`libghostty.{so,a}`) and return
/// the profile dir they are in.
pub fn build_lib() -> PathBuf {
    BUILT
        .get_or_init(|| {
            let dir = profile_dir();
            let profile = dir.file_name().and_then(|s| s.to_str()).unwrap_or("debug");
            let mut cmd = cargo();
            cmd.args(["build", "-p", "ghostty-lib"]);
            if profile != "debug" {
                cmd.args(["--profile", profile]);
            }
            let status = cmd.status().expect("run cargo build");
            assert!(status.success(), "cargo build -p ghostty-lib failed");
            dir
        })
        .clone()
}

/// Compile `sources` (C files) with `cc` against `libghostty`.
pub fn compile_c(lib_dir: &Path, sources: &[&Path], out: &Path, extra_cflags: &[&str]) {
    let mut cmd = Command::new("cc");
    cmd.arg("-std=gnu11")
        .arg("-I")
        .arg(include_dir())
        .args(extra_cflags)
        .args(sources)
        .arg("-o")
        .arg(out)
        .arg("-L")
        .arg(lib_dir)
        .arg("-lghostty")
        .arg(format!("-Wl,-rpath,{}", lib_dir.display()));
    let output = cmd.output().expect("run cc");
    assert!(
        output.status.success(),
        "cc failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn scratch_dir(name: &str) -> PathBuf {
    let d = profile_dir().join("ghostty-lib-tests").join(name);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// Header text with `/* */` and `//` comments removed.
pub fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"/*") {
            i += 2;
            while i < b.len() && !b[i..].starts_with(b"*/") {
                i += 1;
            }
            i += 2;
        } else if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else {
            let ch = src[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// The header with comments removed, and (on non-Apple targets) the
/// `#ifdef __APPLE__` blocks dropped.
pub fn header_text() -> String {
    let raw = std::fs::read_to_string(include_dir().join("ghostty.h")).unwrap();
    let text = strip_comments(&raw);
    let mut out = String::new();
    let mut skipping = false;
    for line in text.lines() {
        let t = line.trim();
        if t == "#ifdef __APPLE__" {
            skipping = !cfg!(target_vendor = "apple");
            continue;
        }
        if skipping && t.starts_with("#endif") {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// `ghostty_*` function names declared in the header (comments and, off
/// Apple, `__APPLE__` blocks removed).
pub fn declared_functions() -> Vec<String> {
    let text = header_text();
    let b = text.as_bytes();
    let mut names = Vec::new();
    let mut i = 0;
    while let Some(pos) = text[i..].find("ghostty_") {
        let start = i + pos;
        let mut end = start;
        while end < b.len()
            && (b[end].is_ascii_lowercase() || b[end].is_ascii_digit() || b[end] == b'_')
        {
            end += 1;
        }
        let mut j = end;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < b.len()
            && b[j] == b'('
            && !text[..start].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        {
            let n = text[start..end].to_string();
            if !names.contains(&n) {
                names.push(n);
            }
        }
        i = end.max(start + 1);
    }
    names.sort();
    names
}
