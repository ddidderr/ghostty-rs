//! Builds the crate for `wasm32-unknown-unknown` and runs Ghostty's wasm
//! examples (examples/wasm/*.html, unmodified) under node, checking that the
//! exported symbols match the headers and the examples produce the right
//! output.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::{path::PathBuf, process::Command, sync::OnceLock};

fn tool_available(cmd: &str, arg: &str) -> bool {
    Command::new(cmd)
        .arg(arg)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn wasm_target_installed() -> bool {
    Command::new("rustc")
        .args([
            "--print",
            "target-libdir",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .is_some_and(|p| {
            p.join("libstd.rlib").exists()
                || std::fs::read_dir(p).is_ok_and(|mut d| {
                    d.any(|e| {
                        e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with("libstd-"))
                    })
                })
        })
}

/// Path of the built wasm module, or `None` (with a note) if the toolchain
/// pieces are missing.
fn wasm_module() -> Option<PathBuf> {
    static WASM: OnceLock<Option<PathBuf>> = OnceLock::new();
    WASM.get_or_init(|| {
        if !tool_available("node", "--version") {
            eprintln!("node not installed; skipping wasm tests");
            return None;
        }
        if !wasm_target_installed() {
            eprintln!("wasm32-unknown-unknown target not installed; skipping wasm tests");
            return None;
        }
        let profile = common::profile_dir();
        let profile = profile
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("debug")
            .to_string();
        let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        cmd.args([
            "build",
            "-p",
            "ghostty-vt-capi",
            "--target",
            "wasm32-unknown-unknown",
        ]);
        if profile != "debug" {
            cmd.args(["--profile", &profile]);
        }
        assert!(cmd.status().unwrap().success(), "wasm build failed");
        let target = common::profile_dir()
            .parent()
            .unwrap()
            .join("wasm32-unknown-unknown");
        let wasm = target.join(&profile).join("ghostty_vt_capi.wasm");
        assert!(wasm.exists(), "{} missing", wasm.display());
        // Install under the upstream name too (`ghostty-vt.wasm`).
        let named = wasm.with_file_name("ghostty-vt.wasm");
        std::fs::copy(&wasm, &named).unwrap();
        Some(named)
    })
    .clone()
}

fn node(args: &[&str]) -> String {
    let script = common::manifest_dir().join("tests/wasm/run.mjs");
    let out = Command::new("node")
        .arg(&script)
        .args(args)
        .output()
        .expect("run node");
    assert!(
        out.status.success(),
        "node failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn example(name: &str) -> String {
    common::manifest_dir()
        .join("examples/wasm")
        .join(name)
        .display()
        .to_string()
}

#[test]
fn wasm_exports_match_headers() {
    let Some(wasm) = wasm_module() else { return };
    // Every function in every header, including the `__wasm__`-only ones.
    let names = common::declared_functions().join("\n");
    assert!(
        names.contains("ghostty_wasm_alloc_sgr_attribute")
            && names.contains("ghostty_wasm_alloc_u8_array")
    );
    let names_file = common::scratch_dir("wasm").join("names.txt");
    std::fs::write(&names_file, names).unwrap();
    let out = node(&[
        "exports",
        &wasm.display().to_string(),
        &names_file.display().to_string(),
    ]);
    assert!(!out.contains("MISSING"), "{out}");
    assert!(out.contains("has_memory: true"), "{out}");
    // No host imports are needed (upstream imports `env.log`; ours logs nowhere).
    assert!(out.contains("imports: \n"), "{out}");
}

fn sections(out: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for line in out.lines() {
        if let Some(name) = line.strip_prefix("=== ") {
            v.push((name.to_string(), String::new()));
        } else if let Some((_, body)) = v.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    v
}

#[test]
fn wasm_sgr_example() {
    let Some(wasm) = wasm_module() else { return };
    let html = example("sgr.html");
    let wasm = wasm.display().to_string();
    let header = common::include_dir()
        .join("ghostty/vt/sgr.h")
        .display()
        .to_string();

    // As shipped upstream, the page's tag table is stale relative to sgr.h
    // (upstream commit a384af5e2 changed the header only), so only the
    // attributes whose tags still agree (underline = 7) are labelled right.
    let stock = node(&["sgr", &html, &wasm, &header]);
    let (_, text) = &sections(&stock)[0];
    assert!(
        text.contains("Attribute 1: Underline style = curly"),
        "{text}"
    );
    assert!(text.contains("Total attributes parsed: 4"), "{text}");

    // With the tag table regenerated from sgr.h the page output is exact.
    let out = node(&["sgr", &html, &wasm, &header, "fix-tags"]);
    let s = sections(&out);
    let get = |n: &str| {
        s.iter()
            .find(|(k, _)| k == n)
            .unwrap_or_else(|| panic!("{n}"))
            .1
            .clone()
    };
    let head = "Parsing SGR sequence:\n";
    assert_eq!(
        get("default"),
        format!(
            "{head}ESC[4:3;38;2;51;51;51;48;2;170;170;170;58;2;255;97;136m\n\n\
             Attribute 1: Underline style = curly\n\
             Attribute 2: Foreground RGB = (51, 51, 51)\n\
             Attribute 3: Background RGB = (170, 170, 170)\n\
             Attribute 4: Underline color RGB = (255, 97, 136)\n\
             \nTotal attributes parsed: 4\n"
        )
    );
    assert_eq!(
        get("1;3;31;42"),
        format!(
            "{head}ESC[1;3;31;42m\n\n\
             Attribute 1: Bold\n\
             Attribute 2: Italic\n\
             Attribute 3: Foreground 8-color = 1\n\
             Attribute 4: Background 8-color = 2\n\
             \nTotal attributes parsed: 4\n"
        )
    );
    assert_eq!(
        get("38:5:123;48;5;200"),
        format!(
            "{head}ESC[38:5:123;48;5;200m\n\n\
             Attribute 1: Foreground 256-color = 123\n\
             Attribute 2: Background 256-color = 200\n\
             \nTotal attributes parsed: 2\n"
        )
    );
    assert_eq!(
        get("0"),
        format!(
            "{head}ESC[0m\n\nAttribute 1: Reset all attributes\n\nTotal attributes parsed: 1\n"
        )
    );
    assert_eq!(
        get("9999"),
        format!(
            "{head}ESC[9999m\n\nAttribute 1: Unknown attribute\n\nTotal attributes parsed: 1\n"
        )
    );
    let underlines = get("4:0;4:1;4:2;4:3;4:4;4:5");
    for style in ["none", "single", "double", "curly", "dotted", "dashed"] {
        assert!(
            underlines.contains(&format!("Underline style = {style}\n")),
            "{underlines}"
        );
    }
    let bright = get("91;102;53");
    assert!(
        bright.contains("Other attribute (tag=BRIGHT_FG_8)"),
        "{bright}"
    );
    assert!(
        bright.contains("Other attribute (tag=BRIGHT_BG_8)"),
        "{bright}"
    );
    assert!(
        bright.contains("Other attribute (tag=OVERLINE)"),
        "{bright}"
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn wasm_key_encode_example() {
    use ghostty_input::{
        Action,
        Key,
        KeyEvent,
        Mods,
        key_encode::{Options, encode},
    };
    use ghostty_vt::kitty::key::Flags;

    let Some(wasm) = wasm_module() else { return };
    let out = node(&[
        "key",
        &example("key-encode.html"),
        &wasm.display().to_string(),
    ]);
    let idx = out.lines().position(|l| l == "flags: none").unwrap();
    let (with_flags, without): (Vec<_>, Vec<_>) = {
        let lines: Vec<&str> = out.lines().collect();
        (lines[..idx].to_vec(), lines[idx..].to_vec())
    };
    let parse = |lines: &[&str]| -> Vec<(String, String)> {
        lines
            .iter()
            .filter_map(|l| l.split_once('\t'))
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    };
    let (with_flags, without) = (parse(&with_flags), parse(&without));

    let enc = |opts: Options, action: Action, key: Key, mods: Mods, utf8: &str, unshifted: char| {
        let ev = KeyEvent {
            action,
            key,
            mods,
            utf8: utf8.as_bytes(),
            unshifted_codepoint: unshifted as u32,
            ..KeyEvent::default()
        };
        let mut buf = Vec::new();
        encode(&mut buf, &ev, &opts).unwrap();
        hex(&buf)
    };
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    let none = Mods::default();
    let all = Options {
        kitty_flags: Flags::TRUE,
        ..Options::default()
    };
    let legacy = Options::default();

    let want_all = [
        ("ctrl+a", enc(all, Action::Press, Key::KeyA, ctrl, "a", 'a')),
        ("a", enc(all, Action::Press, Key::KeyA, none, "a", 'a')),
        (
            "shift+Enter",
            enc(all, Action::Press, Key::Enter, shift, "", '\0'),
        ),
        (
            "ArrowUp",
            enc(all, Action::Press, Key::ArrowUp, none, "", '\0'),
        ),
        ("alt+x", enc(all, Action::Press, Key::KeyX, alt, "x", 'x')),
        (
            "release a",
            enc(all, Action::Release, Key::KeyA, none, "a", 'a'),
        ),
    ];
    assert_eq!(with_flags.len(), want_all.len(), "{with_flags:?}");
    for ((name, got), (wname, want)) in with_flags.iter().zip(want_all) {
        assert_eq!(name, wname);
        assert_eq!(*got, want, "kitty-all: {name}");
    }
    // Spot-check against the protocol, so a shared bug can't hide.
    assert_eq!(with_flags[0].1, hex(b"\x1b[97;5u"));
    assert_eq!(with_flags[2].1, hex(b"\x1b[13;2u"));
    assert_eq!(with_flags[5].1, hex(b"\x1b[97;1:3u"));

    let want_none = [
        (
            "ctrl+a",
            enc(legacy, Action::Press, Key::KeyA, ctrl, "a", 'a'),
        ),
        ("a", enc(legacy, Action::Press, Key::KeyA, none, "a", 'a')),
        (
            "shift+Enter",
            enc(legacy, Action::Press, Key::Enter, shift, "", '\0'),
        ),
        (
            "ArrowUp",
            enc(legacy, Action::Press, Key::ArrowUp, none, "", '\0'),
        ),
        (
            "alt+x",
            enc(legacy, Action::Press, Key::KeyX, alt, "x", 'x'),
        ),
        (
            "Escape",
            enc(legacy, Action::Press, Key::Escape, none, "", '\0'),
        ),
        ("F5", enc(legacy, Action::Press, Key::F5, none, "", '\0')),
        (
            "unidentified",
            enc(legacy, Action::Press, Key::Unidentified, none, "", '\0'),
        ),
    ];
    assert_eq!(without.len(), want_none.len(), "{without:?}");
    for ((name, got), (wname, want)) in without.iter().zip(want_none) {
        assert_eq!(name, wname);
        assert_eq!(*got, want, "legacy: {name}");
    }
    assert_eq!(without[0].1, "01");
    assert_eq!(without[1].1, "61");
    assert_eq!(without[3].1, hex(b"\x1b[A"));
    // alt_esc_prefix is off by default in the C encoder, so alt+x is plain "x".
    assert_eq!(without[4].1, hex(b"x"));
}
