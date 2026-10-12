//! Shared helpers: build the library and install it under Ghostty's file
//! names (`libghostty-vt.{so,a}`) next to the Cargo artifacts.
#![allow(dead_code)]

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

/// Major version of libghostty-vt (the soname suffix set by build.rs).
pub const VT_MAJOR: u32 = 0;

static INSTALLED: OnceLock<PathBuf> = OnceLock::new();

/// Build the cdylib/staticlib for the host and install them as
/// `libghostty-vt.so` / `libghostty-vt.a` in the profile dir. Returns that dir.
pub fn install_native_lib() -> PathBuf {
    INSTALLED.get_or_init(install_native_lib_once).clone()
}

fn install_native_lib_once() -> PathBuf {
    let dir = profile_dir();
    let profile = dir.file_name().and_then(|s| s.to_str()).unwrap_or("debug");
    let mut cmd = cargo();
    cmd.args(["build", "-p", "ghostty-vt-capi"]);
    if profile != "debug" {
        cmd.args(["--profile", profile]);
    }
    let status = cmd.status().expect("run cargo build");
    assert!(status.success(), "cargo build -p ghostty-vt-capi failed");
    for (from, to) in [
        ("libghostty_vt_capi.so", "libghostty-vt.so"),
        ("libghostty_vt_capi.a", "libghostty-vt.a"),
        ("libghostty_vt_capi.dylib", "libghostty-vt.dylib"),
    ] {
        let src = dir.join(from);
        if src.exists() {
            let dst = dir.join(to);
            // Copy + rename so a concurrent reader never sees a partial file.
            let tmp = dir.join(format!("{to}.{}.tmp", std::process::id()));
            std::fs::copy(&src, &tmp).expect("install library");
            std::fs::rename(&tmp, &dst).expect("install library");
        }
    }
    // The cdylib's soname is versioned (`libghostty-vt.so.<major>`, see
    // build.rs), so executables linked against it need that name at run time
    // (otherwise the loader would fall back to an unrelated system copy).
    let so = dir.join("libghostty-vt.so");
    if so.exists() {
        let versioned = dir.join(format!("libghostty-vt.so.{VT_MAJOR}"));
        let tmp = dir.join(format!(
            "libghostty-vt.so.{VT_MAJOR}.{}.tmp",
            std::process::id()
        ));
        std::fs::copy(&so, &tmp).expect("install versioned library");
        std::fs::rename(&tmp, &versioned).expect("install versioned library");
    }
    dir
}

/// Compile `sources` (C files) with `cc` against the installed library.
pub fn compile_c(lib_dir: &Path, sources: &[&Path], out: &Path, extra_cflags: &[&str]) {
    let mut cmd = Command::new("cc");
    cmd.arg("-I")
        .arg(include_dir())
        .args(extra_cflags)
        .args(sources)
        .arg("-o")
        .arg(out)
        .arg("-L")
        .arg(lib_dir)
        .arg("-lghostty-vt")
        .arg(format!("-Wl,-rpath,{}", lib_dir.display()));
    let output = cmd.output().expect("run cc");
    assert!(
        output.status.success(),
        "cc failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn scratch_dir(name: &str) -> PathBuf {
    let d = profile_dir().join("vt-capi-tests").join(name);
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

/// All headers (relative to `include/`), in a stable order.
pub fn headers() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "h") {
                out.push(p);
            }
        }
    }
    let mut v = Vec::new();
    walk(&include_dir(), &mut v);
    v.sort();
    v
}

/// `ghostty_*` function names declared in the headers (comments removed).
pub fn declared_functions() -> Vec<String> {
    let mut names = Vec::new();
    for h in headers() {
        let text = strip_comments(&std::fs::read_to_string(h).unwrap());
        let b = text.as_bytes();
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
    }
    names.sort();
    names
}
