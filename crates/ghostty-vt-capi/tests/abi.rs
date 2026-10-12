//! ABI verification: a C program generated from the unchanged headers and
//! compiled with `cc` prints `sizeof`/`alignof`/`offsetof` of every struct and
//! union, `sizeof` of every enum/typedef, the value of every enumerator and
//! `#define`d constant, and takes the address of every declared function (so
//! linking proves the symbol is exported). The values are compared against the
//! Rust definitions (`size_of`/`align_of`/`offset_of!`, enum discriminants).
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    mem::{align_of, offset_of, size_of},
    process::Command,
};

use ghostty_input::{
    Mods,
    key::Key,
    key_mods::{ModSide, Side},
};
use ghostty_vt::{color::Name, kitty::key::Flags as KittyFlags};
use ghostty_vt_capi::{
    alloc::{Allocator, AllocatorVtable},
    color::GhosttyColorRgb,
    key_encode::KeyEncoderOption,
    key_event::KeyAction,
    osc::OscCommandData,
    result::GhosttyResult,
    sgr::{SgrAttribute, SgrAttributeTag, SgrAttributeValue, SgrUnknown},
};

/// `GHOSTTY_FOO_BAR` / `FooBar` / `foo_bar` -> `foobar`.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '_')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Parsed `typedef enum { ... } Name;` blocks: typedef name -> enumerators.
fn header_enums() -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    for h in common::headers() {
        let text = common::strip_comments(&std::fs::read_to_string(h).unwrap());
        let mut rest = text.as_str();
        while let Some(start) = rest.find("typedef enum") {
            let after = &rest[start..];
            let open = after.find('{').unwrap();
            let close = after.find('}').unwrap();
            let name_end = after[close..].find(';').unwrap() + close;
            let name = after[close + 1..name_end].trim().to_string();
            let names = after[open + 1..close]
                .split(',')
                .map(|e| e.split('=').next().unwrap().trim().to_string())
                .filter(|e| !e.is_empty())
                .collect();
            map.insert(name, names);
            rest = &after[name_end..];
        }
    }
    map
}

/// `#define GHOSTTY_X ...` constants that are not include guards.
fn header_defines() -> Vec<String> {
    let mut v = Vec::new();
    for h in common::headers() {
        for line in std::fs::read_to_string(h).unwrap().lines() {
            if let Some(rest) = line.strip_prefix("#define GHOSTTY_") {
                let name = rest.split_whitespace().next().unwrap();
                if !name.ends_with("_H") && rest.split_whitespace().nth(1).is_some() {
                    v.push(format!("GHOSTTY_{name}"));
                }
            }
        }
    }
    v
}

/// Names and values of a Rust-side enum, keyed by normalized name.
fn vals<T: std::fmt::Debug + Copy>(items: &[T], f: impl Fn(T) -> i64) -> BTreeMap<String, i64> {
    items
        .iter()
        .map(|i| (norm(&format!("{i:?}")), f(*i)))
        .collect()
}

struct Probe {
    c: String,
    expected: BTreeMap<String, i64>,
}

impl Probe {
    fn emit(&mut self, key: &str, c_expr: &str, expected: i64) {
        let _ = writeln!(
            self.c,
            "  printf(\"%s=%lld\\n\", \"{key}\", (long long)({c_expr}));"
        );
        self.expected.insert(key.to_string(), expected);
    }

    fn layout(&mut self, ty: &str, size: usize, align: usize, fields: &[(&str, usize)]) {
        self.emit(
            &format!("sizeof({ty})"),
            &format!("sizeof({ty})"),
            size as i64,
        );
        self.emit(
            &format!("alignof({ty})"),
            &format!("_Alignof({ty})"),
            align as i64,
        );
        for (f, off) in fields {
            self.emit(
                &format!("offsetof({ty},{f})"),
                &format!("offsetof({ty}, {f})"),
                *off as i64,
            );
        }
    }

    /// Check every enumerator of `typedef` against `rust` (names after
    /// `prefix`); the C compiler provides the values. `exact` also requires
    /// that Rust has no extra members.
    fn enumerators(
        &mut self,
        enums: &BTreeMap<String, Vec<String>>,
        typedef: &str,
        prefix: &str,
        rust: &BTreeMap<String, i64>,
        exact: bool,
    ) {
        let names = enums
            .get(typedef)
            .unwrap_or_else(|| panic!("no enum {typedef}"));
        // All GHOSTTY enums are `int` sized.
        self.emit(
            &format!("sizeof({typedef})"),
            &format!("sizeof({typedef})"),
            4,
        );
        for n in names {
            let short = n
                .strip_prefix(prefix)
                .unwrap_or_else(|| panic!("{n} lacks {prefix}"));
            let want = *rust
                .get(&norm(short))
                .unwrap_or_else(|| panic!("{typedef}: Rust has no member for {n}"));
            self.emit(n, n, want);
        }
        if exact {
            assert_eq!(names.len(), rust.len(), "{typedef}: member count differs");
        }
    }
}

fn build_probe() -> Probe {
    let enums = header_enums();
    let mut p = Probe {
        c: String::new(),
        expected: BTreeMap::new(),
    };

    // ---- structs and unions ----
    p.layout(
        "GhosttyColorRgb",
        size_of::<GhosttyColorRgb>(),
        align_of::<GhosttyColorRgb>(),
        &[
            ("r", offset_of!(GhosttyColorRgb, r)),
            ("g", offset_of!(GhosttyColorRgb, g)),
            ("b", offset_of!(GhosttyColorRgb, b)),
        ],
    );
    p.layout(
        "GhosttyAllocatorVtable",
        size_of::<AllocatorVtable>(),
        align_of::<AllocatorVtable>(),
        &[
            ("alloc", offset_of!(AllocatorVtable, alloc)),
            ("resize", offset_of!(AllocatorVtable, resize)),
            ("remap", offset_of!(AllocatorVtable, remap)),
            ("free", offset_of!(AllocatorVtable, free)),
        ],
    );
    p.layout(
        "GhosttyAllocator",
        size_of::<Allocator>(),
        align_of::<Allocator>(),
        &[
            ("ctx", offset_of!(Allocator, ctx)),
            ("vtable", offset_of!(Allocator, vtable)),
        ],
    );
    p.layout(
        "GhosttySgrUnknown",
        size_of::<SgrUnknown>(),
        align_of::<SgrUnknown>(),
        &[
            ("full_ptr", offset_of!(SgrUnknown, full_ptr)),
            ("full_len", offset_of!(SgrUnknown, full_len)),
            ("partial_ptr", offset_of!(SgrUnknown, partial_ptr)),
            ("partial_len", offset_of!(SgrUnknown, partial_len)),
        ],
    );
    p.layout(
        "GhosttySgrAttributeValue",
        size_of::<SgrAttributeValue>(),
        align_of::<SgrAttributeValue>(),
        &[
            ("unknown", offset_of!(SgrAttributeValue, unknown)),
            ("underline", offset_of!(SgrAttributeValue, underline)),
            (
                "underline_color",
                offset_of!(SgrAttributeValue, underline_color),
            ),
            (
                "underline_color_256",
                offset_of!(SgrAttributeValue, underline_color_256),
            ),
            (
                "direct_color_fg",
                offset_of!(SgrAttributeValue, direct_color_fg),
            ),
            (
                "direct_color_bg",
                offset_of!(SgrAttributeValue, direct_color_bg),
            ),
            ("bg_8", offset_of!(SgrAttributeValue, bg_8)),
            ("fg_8", offset_of!(SgrAttributeValue, fg_8)),
            ("bright_bg_8", offset_of!(SgrAttributeValue, bright_bg_8)),
            ("bright_fg_8", offset_of!(SgrAttributeValue, bright_fg_8)),
            ("bg_256", offset_of!(SgrAttributeValue, bg_256)),
            ("fg_256", offset_of!(SgrAttributeValue, fg_256)),
            ("_padding", offset_of!(SgrAttributeValue, _padding)),
        ],
    );
    p.layout(
        "GhosttySgrAttribute",
        size_of::<SgrAttribute>(),
        align_of::<SgrAttribute>(),
        &[
            ("tag", offset_of!(SgrAttribute, tag)),
            ("value", offset_of!(SgrAttribute, value)),
        ],
    );

    // ---- scalar typedefs and opaque handles ----
    for (ty, size) in [
        ("GhosttyMods", 2),
        ("GhosttyKittyKeyFlags", 1),
        ("GhosttyColorPaletteIndex", 1),
        ("GhosttyOscParser", size_of::<usize>()),
        ("GhosttyOscCommand", size_of::<usize>()),
        ("GhosttySgrParser", size_of::<usize>()),
        ("GhosttyKeyEvent", size_of::<usize>()),
        ("GhosttyKeyEncoder", size_of::<usize>()),
    ] {
        p.emit(
            &format!("sizeof({ty})"),
            &format!("sizeof({ty})"),
            size as i64,
        );
    }

    // ---- enums ----
    p.enumerators(
        &enums,
        "GhosttyResult",
        "GHOSTTY_",
        &vals(
            &[
                GhosttyResult::Success,
                GhosttyResult::OutOfMemory,
                GhosttyResult::InvalidValue,
            ],
            |v| v as i64,
        ),
        true,
    );
    // The Rust OSC parser knows two commands newer than the header.
    let osc = {
        use ghostty_vt::osc::Key::*;
        vals(
            &[
                Invalid,
                ChangeWindowTitle,
                ChangeWindowIcon,
                SemanticPrompt,
                ClipboardContents,
                ReportPwd,
                MouseShape,
                ColorOperation,
                KittyColorProtocol,
                ShowDesktopNotification,
                HyperlinkStart,
                HyperlinkEnd,
                ConemuSleep,
                ConemuShowMessageBox,
                ConemuChangeTabTitle,
                ConemuProgressReport,
                ConemuWaitInput,
                ConemuGuimacro,
                ConemuRunProcess,
                ConemuOutputEnvironmentVariable,
                ConemuXtermEmulation,
                ConemuComment,
                KittyTextSizing,
                KittyClipboardProtocol,
                ContextSignal,
            ],
            |v| v as i64,
        )
    };
    p.enumerators(
        &enums,
        "GhosttyOscCommandType",
        "GHOSTTY_OSC_COMMAND_",
        &osc,
        false,
    );
    p.enumerators(
        &enums,
        "GhosttyOscCommandData",
        "GHOSTTY_OSC_DATA_",
        &vals(
            &[
                OscCommandData::Invalid,
                OscCommandData::ChangeWindowTitleStr,
            ],
            |v| v as i64,
        ),
        true,
    );
    p.enumerators(
        &enums,
        "GhosttySgrAttributeTag",
        "GHOSTTY_SGR_ATTR_",
        &sgr_tags(),
        true,
    );
    p.enumerators(
        &enums,
        "GhosttySgrUnderline",
        "GHOSTTY_SGR_UNDERLINE_",
        &vals(
            &[
                ghostty_vt::sgr::Underline::None,
                ghostty_vt::sgr::Underline::Single,
                ghostty_vt::sgr::Underline::Double,
                ghostty_vt::sgr::Underline::Curly,
                ghostty_vt::sgr::Underline::Dotted,
                ghostty_vt::sgr::Underline::Dashed,
            ],
            |v| v as i64,
        ),
        true,
    );
    p.enumerators(
        &enums,
        "GhosttyKeyAction",
        "GHOSTTY_KEY_ACTION_",
        &vals(
            &[KeyAction::Release, KeyAction::Press, KeyAction::Repeat],
            |v| v as i64,
        ),
        true,
    );
    // ghostty_input::Action must agree with the header too.
    assert_eq!(ghostty_input::Action::Release as i32, 0);
    assert_eq!(ghostty_input::Action::Press as i32, 1);
    assert_eq!(ghostty_input::Action::Repeat as i32, 2);

    let keys: BTreeMap<String, i64> = Key::ALL
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let n = k.name();
            (norm(n.strip_prefix("key_").unwrap_or(n)), i as i64)
        })
        .collect();
    p.enumerators(&enums, "GhosttyKey", "GHOSTTY_KEY_", &keys, true);
    for (i, k) in Key::ALL.iter().enumerate() {
        assert_eq!(*k as usize, i, "Key discriminants are dense");
    }
    p.enumerators(
        &enums,
        "GhosttyKeyEncoderOption",
        "GHOSTTY_KEY_ENCODER_OPT_",
        &vals(
            &[
                KeyEncoderOption::CursorKeyApplication,
                KeyEncoderOption::KeypadKeyApplication,
                KeyEncoderOption::IgnoreKeypadWithNumlock,
                KeyEncoderOption::AltEscPrefix,
                KeyEncoderOption::ModifyOtherKeysState2,
                KeyEncoderOption::KittyFlags,
                KeyEncoderOption::MacosOptionAsAlt,
            ],
            |v| v as i64,
        ),
        true,
    );
    p.enumerators(
        &enums,
        "GhosttyOptionAsAlt",
        "GHOSTTY_OPTION_AS_ALT_",
        &vals(
            &[
                ghostty_input::OptionAsAlt::False,
                ghostty_input::OptionAsAlt::True,
                ghostty_input::OptionAsAlt::Left,
                ghostty_input::OptionAsAlt::Right,
            ],
            |v| v as i64,
        ),
        true,
    );

    // ---- #define constants ----
    let right = |f: fn(&mut Side)| {
        let mut s = Side::default();
        f(&mut s);
        Mods {
            sides: s,
            ..Mods::default()
        }
        .int() as i64
    };
    let _ = ModSide::Right;
    let mod_flag = |f: fn(&mut Mods)| {
        let mut m = Mods::default();
        f(&mut m);
        m.int() as i64
    };
    let kitty = |f: fn(&mut KittyFlags)| {
        let mut k = KittyFlags::DISABLED;
        f(&mut k);
        k.int() as i64
    };
    let named = |n: Name| i64::from(n.0);
    let defines: BTreeMap<&str, i64> = BTreeMap::from([
        ("GHOSTTY_MODS_SHIFT", mod_flag(|m| m.shift = true)),
        ("GHOSTTY_MODS_CTRL", mod_flag(|m| m.ctrl = true)),
        ("GHOSTTY_MODS_ALT", mod_flag(|m| m.alt = true)),
        ("GHOSTTY_MODS_SUPER", mod_flag(|m| m.super_ = true)),
        ("GHOSTTY_MODS_CAPS_LOCK", mod_flag(|m| m.caps_lock = true)),
        ("GHOSTTY_MODS_NUM_LOCK", mod_flag(|m| m.num_lock = true)),
        (
            "GHOSTTY_MODS_SHIFT_SIDE",
            right(|s| s.shift = ModSide::Right),
        ),
        ("GHOSTTY_MODS_CTRL_SIDE", right(|s| s.ctrl = ModSide::Right)),
        ("GHOSTTY_MODS_ALT_SIDE", right(|s| s.alt = ModSide::Right)),
        (
            "GHOSTTY_MODS_SUPER_SIDE",
            right(|s| s.super_ = ModSide::Right),
        ),
        ("GHOSTTY_KITTY_KEY_DISABLED", kitty(|_| {})),
        (
            "GHOSTTY_KITTY_KEY_DISAMBIGUATE",
            kitty(|k| k.disambiguate = true),
        ),
        (
            "GHOSTTY_KITTY_KEY_REPORT_EVENTS",
            kitty(|k| k.report_events = true),
        ),
        (
            "GHOSTTY_KITTY_KEY_REPORT_ALTERNATES",
            kitty(|k| k.report_alternates = true),
        ),
        (
            "GHOSTTY_KITTY_KEY_REPORT_ALL",
            kitty(|k| k.report_all = true),
        ),
        (
            "GHOSTTY_KITTY_KEY_REPORT_ASSOCIATED",
            kitty(|k| k.report_associated = true),
        ),
        ("GHOSTTY_KITTY_KEY_ALL", KittyFlags::TRUE.int() as i64),
        ("GHOSTTY_COLOR_NAMED_BLACK", named(Name::BLACK)),
        ("GHOSTTY_COLOR_NAMED_RED", named(Name::RED)),
        ("GHOSTTY_COLOR_NAMED_GREEN", named(Name::GREEN)),
        ("GHOSTTY_COLOR_NAMED_YELLOW", named(Name::YELLOW)),
        ("GHOSTTY_COLOR_NAMED_BLUE", named(Name::BLUE)),
        ("GHOSTTY_COLOR_NAMED_MAGENTA", named(Name::MAGENTA)),
        ("GHOSTTY_COLOR_NAMED_CYAN", named(Name::CYAN)),
        ("GHOSTTY_COLOR_NAMED_WHITE", named(Name::WHITE)),
        (
            "GHOSTTY_COLOR_NAMED_BRIGHT_BLACK",
            named(Name::BRIGHT_BLACK),
        ),
        ("GHOSTTY_COLOR_NAMED_BRIGHT_RED", named(Name::BRIGHT_RED)),
        (
            "GHOSTTY_COLOR_NAMED_BRIGHT_GREEN",
            named(Name::BRIGHT_GREEN),
        ),
        (
            "GHOSTTY_COLOR_NAMED_BRIGHT_YELLOW",
            named(Name::BRIGHT_YELLOW),
        ),
        ("GHOSTTY_COLOR_NAMED_BRIGHT_BLUE", named(Name::BRIGHT_BLUE)),
        (
            "GHOSTTY_COLOR_NAMED_BRIGHT_MAGENTA",
            named(Name::BRIGHT_MAGENTA),
        ),
        ("GHOSTTY_COLOR_NAMED_BRIGHT_CYAN", named(Name::BRIGHT_CYAN)),
        (
            "GHOSTTY_COLOR_NAMED_BRIGHT_WHITE",
            named(Name::BRIGHT_WHITE),
        ),
    ]);
    let declared = header_defines();
    for d in &declared {
        let want = defines
            .get(d.as_str())
            .unwrap_or_else(|| panic!("header #define {d} has no Rust expectation"));
        p.emit(d, d, *want);
    }
    assert_eq!(
        declared.len(),
        defines.len(),
        "stale expectation in the define table"
    );
    p
}

fn sgr_tags() -> BTreeMap<String, i64> {
    use SgrAttributeTag::*;
    vals(
        &[
            Unset,
            Unknown,
            Bold,
            ResetBold,
            Italic,
            ResetItalic,
            Faint,
            Underline,
            UnderlineColor,
            UnderlineColor256,
            ResetUnderlineColor,
            Overline,
            ResetOverline,
            Blink,
            ResetBlink,
            Inverse,
            ResetInverse,
            Invisible,
            ResetInvisible,
            Strikethrough,
            ResetStrikethrough,
            DirectColorFg,
            DirectColorBg,
            Bg8,
            Fg8,
            ResetFg,
            ResetBg,
            BrightBg8,
            BrightFg8,
            Bg256,
            Fg256,
        ],
        |v| v as i64,
    )
}

#[test]
fn abi_matches_headers() {
    let lib = common::install_native_lib();
    let scratch = common::scratch_dir("abi");
    let mut probe = build_probe();

    let fns: Vec<String> = common::declared_functions()
        .into_iter()
        .filter(|f| !f.starts_with("ghostty_wasm_"))
        .collect();
    assert!(fns.len() == 38, "expected the full API, got {fns:?}");
    let mut src = String::from(
        "#include <stdio.h>\n#include <stddef.h>\n#include <stdalign.h>\n#include <ghostty/vt.h>\n",
    );
    src.push_str("static void *const fns[] = {\n");
    for f in &fns {
        let _ = writeln!(src, "  (void *){f},");
    }
    src.push_str("};\nint main(void) {\n  if (sizeof(fns) == 0) return 2;\n");
    src.push_str(&probe.c);
    src.push_str("  return 0;\n}\n");

    let c_file = scratch.join("probe.c");
    std::fs::write(&c_file, src).unwrap();
    let exe = scratch.join("probe");
    common::compile_c(&lib, &[&c_file], &exe, &["-Wall", "-Werror", "-std=c11"]);
    let out = Command::new(&exe).output().unwrap();
    assert!(out.status.success());

    let got: BTreeMap<String, i64> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            let (k, v) = l.rsplit_once('=').unwrap();
            (k.to_string(), v.parse().unwrap())
        })
        .collect();
    probe.expected.insert("_".into(), 0);
    let mut bad = Vec::new();
    for (k, want) in &probe.expected {
        if k == "_" {
            continue;
        }
        match got.get(k) {
            Some(g) if g == want => {}
            g => bad.push(format!("{k}: C={g:?} Rust={want}")),
        }
    }
    assert!(bad.is_empty(), "ABI mismatches:\n{}", bad.join("\n"));
    assert!(got.len() > 300, "suspiciously few probes: {}", got.len());
}

/// Every exported function in the shared object is declared in the headers
/// (no accidental extra `ghostty_*` symbols on native targets).
#[test]
fn exported_symbols_match_headers() {
    let lib = common::install_native_lib();
    let out = Command::new("nm")
        .args(["-D", "--defined-only"])
        .arg(lib.join("libghostty-vt.so"))
        .output();
    let Ok(out) = out else {
        eprintln!("nm not available; skipping");
        return;
    };
    let text = String::from_utf8(out.stdout).unwrap();
    let exported: Vec<&str> = text
        .lines()
        .filter_map(|l| l.split_whitespace().last())
        .filter(|s| s.starts_with("ghostty_"))
        .collect();
    let declared: Vec<String> = common::declared_functions()
        .into_iter()
        .filter(|f| !f.starts_with("ghostty_wasm_"))
        .collect();
    for f in &declared {
        assert!(
            exported.contains(&f.as_str()),
            "{f} declared but not exported"
        );
    }
    for e in &exported {
        assert!(
            declared.iter().any(|d| d == e),
            "{e} exported but not declared"
        );
    }
}
