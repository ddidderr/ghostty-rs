//! ABI verification: a C program generated from the unchanged
//! `include/ghostty.h` prints `sizeof`/`alignof`/`offsetof` of every struct
//! and union, `sizeof` of every enum and the value of every enumerator, and
//! takes the address of every declared function (so linking proves the
//! symbol is exported). The values are compared against the Rust
//! definitions. A second test compares the `nm` exports with the header.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::items_after_statements)]

mod common;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    mem::{align_of, offset_of, size_of},
    process::Command,
};

use ghostty::types::*;
use ghostty_apprt::{
    action::{self, Key as ActionKey},
    structs::{Clipboard, ClipboardRequestType, ColorScheme},
    surface::NewSurfaceContext,
};
use ghostty_input::{
    Action as KeyAction,
    Key,
    Mods,
    MouseButton,
    MouseButtonState,
    binding::Flags,
    key_mods::{ModSide, Side},
    mouse::Momentum,
};
use ghostty_vt::mouse_shape::MouseShape;

/// `GHOSTTY_FOO_BAR` / `FooBar` / `foo_bar` -> `foobar`.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '_')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Parsed `typedef enum { ... } Name;` blocks: typedef name -> enumerators.
fn header_enums(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    let mut rest = text;
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
    map
}

/// Parsed `typedef struct|union { ... } name;` blocks: name -> depth-1 field
/// names.
fn header_records(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut map = BTreeMap::new();
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(pos) = text[i..].find("typedef ") {
        let start = i + pos;
        let after = &text[start + 8..];
        let kw = if after.starts_with("struct {") {
            Some("struct {")
        } else if after.starts_with("union {") {
            Some("union {")
        } else {
            None
        };
        i = start + 8;
        let Some(kw) = kw else { continue };
        let open = start + 8 + kw.len() - 1;
        // Match braces.
        let mut depth = 0usize;
        let mut j = open;
        let mut fields = Vec::new();
        let mut stmt = String::new();
        loop {
            match b[j] {
                b'{' => {
                    depth += 1;
                    if depth > 1 {
                        stmt.push('{');
                    }
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    stmt.push('}');
                }
                b';' if depth == 1 => {
                    // A field: the last identifier, minus array suffix.
                    let s = stmt.trim();
                    let s = if let Some(p) = s.rfind('}') {
                        &s[p + 1..]
                    } else {
                        s
                    };
                    let s = s.split('[').next().unwrap().trim();
                    let name = s
                        .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .next()
                        .unwrap();
                    if !name.is_empty() {
                        fields.push(name.to_string());
                    }
                    stmt.clear();
                }
                c => stmt.push(c as char),
            }
            j += 1;
        }
        let end = text[j..].find(';').unwrap() + j;
        let name = text[j + 1..end].trim().to_string();
        map.insert(name, fields);
        i = end;
    }
    map
}

/// Names and values of a Rust-side enum, keyed by normalized name.
fn vals<T: std::fmt::Debug + Copy>(items: &[T], f: impl Fn(T) -> i64) -> BTreeMap<String, i64> {
    items
        .iter()
        .map(|i| (norm(&format!("{i:?}")), f(*i)))
        .collect()
}

fn named(items: &[(&str, i64)]) -> BTreeMap<String, i64> {
    items.iter().map(|(n, v)| (norm(n), *v)).collect()
}

struct Probe {
    c: String,
    expected: BTreeMap<String, i64>,
    records: BTreeMap<String, Vec<String>>,
    covered_records: BTreeSet<String>,
    covered_enums: BTreeSet<String>,
}

impl Probe {
    fn emit(&mut self, key: &str, c_expr: &str, expected: i64) {
        let _ = writeln!(
            self.c,
            "  printf(\"%s=%lld\\n\", \"{key}\", (long long)({c_expr}));"
        );
        self.expected.insert(key.to_string(), expected);
    }

    /// `fields` are the Rust offsets in the header's field order; the field
    /// names are taken from the parsed header (so a missing or reordered
    /// field is caught).
    fn layout(&mut self, ty: &str, size: usize, align: usize, offsets: &[usize]) {
        let fields = self
            .records
            .get(ty)
            .unwrap_or_else(|| panic!("no record {ty} in the header"))
            .clone();
        assert_eq!(
            fields.len(),
            offsets.len(),
            "{ty}: the header has fields {fields:?}"
        );
        self.covered_records.insert(ty.to_string());
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
        for (f, off) in fields.iter().zip(offsets) {
            self.emit(
                &format!("offsetof({ty},{f})"),
                &format!("offsetof({ty}, {f})"),
                *off as i64,
            );
        }
    }

    /// Check every enumerator of `typedef` against `rust` (names after
    /// `prefix`); the C compiler provides the values.
    fn enumerators(
        &mut self,
        enums: &BTreeMap<String, Vec<String>>,
        typedef: &str,
        prefix: &str,
        rust: &BTreeMap<String, i64>,
    ) {
        let names = enums
            .get(typedef)
            .unwrap_or_else(|| panic!("no enum {typedef}"));
        self.covered_enums.insert(typedef.to_string());
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
        assert_eq!(names.len(), rust.len(), "{typedef}: member count differs");
    }
}

/// `layout!(probe, c_name, RustType, field, ...)`
macro_rules! layout {
    ($p:expr, $ty:ty, $name:literal, $($f:ident),+ $(,)?) => {
        $p.layout(
            $name,
            size_of::<$ty>(),
            align_of::<$ty>(),
            &[$(offset_of!($ty, $f)),+],
        )
    };
}

fn build_probe() -> Probe {
    let text = common::header_text();
    let enums = header_enums(&text);
    let mut p = Probe {
        c: String::new(),
        expected: BTreeMap::new(),
        records: header_records(&text),
        covered_records: BTreeSet::new(),
        covered_enums: BTreeSet::new(),
    };

    // ---- structs and unions ----
    layout!(
        p,
        ghostty_clipboard_content_s,
        "ghostty_clipboard_content_s",
        mime,
        data
    );
    layout!(
        p,
        ghostty_input_key_s,
        "ghostty_input_key_s",
        action,
        mods,
        consumed_mods,
        keycode,
        text,
        unshifted_codepoint,
        composing
    );
    layout!(
        p,
        ghostty_input_trigger_key_u,
        "ghostty_input_trigger_key_u",
        translated,
        physical,
        unicode
    );
    layout!(
        p,
        ghostty_input_trigger_s,
        "ghostty_input_trigger_s",
        tag,
        key,
        mods
    );
    layout!(
        p,
        ghostty_command_s,
        "ghostty_command_s",
        action_key,
        action,
        title,
        description
    );
    layout!(
        p,
        ghostty_info_s,
        "ghostty_info_s",
        build_mode,
        version,
        version_len
    );
    layout!(p, ghostty_diagnostic_s, "ghostty_diagnostic_s", message);
    layout!(p, ghostty_string_s, "ghostty_string_s", ptr, len, sentinel);
    layout!(
        p,
        ghostty_text_s,
        "ghostty_text_s",
        tl_px_x,
        tl_px_y,
        offset_start,
        offset_len,
        text,
        text_len
    );
    layout!(p, ghostty_point_s, "ghostty_point_s", tag, coord, x, y);
    layout!(
        p,
        ghostty_selection_s,
        "ghostty_selection_s",
        top_left,
        bottom_right,
        rectangle
    );
    layout!(p, ghostty_env_var_s, "ghostty_env_var_s", key, value);
    layout!(
        p,
        ghostty_platform_macos_s,
        "ghostty_platform_macos_s",
        nsview
    );
    layout!(p, ghostty_platform_ios_s, "ghostty_platform_ios_s", uiview);
    layout!(p, ghostty_platform_u, "ghostty_platform_u", macos, ios);
    layout!(
        p,
        ghostty_surface_config_s,
        "ghostty_surface_config_s",
        platform_tag,
        platform,
        userdata,
        scale_factor,
        font_size,
        working_directory,
        command,
        env_vars,
        env_var_count,
        initial_input,
        wait_after_command,
        context
    );
    layout!(
        p,
        ghostty_surface_size_s,
        "ghostty_surface_size_s",
        columns,
        rows,
        width_px,
        height_px,
        cell_width_px,
        cell_height_px
    );
    layout!(
        p,
        ghostty_config_path_s,
        "ghostty_config_path_s",
        path,
        optional
    );
    layout!(p, ghostty_config_color_s, "ghostty_config_color_s", r, g, b);
    layout!(
        p,
        ghostty_config_color_list_s,
        "ghostty_config_color_list_s",
        colors,
        len
    );
    layout!(
        p,
        ghostty_config_command_list_s,
        "ghostty_config_command_list_s",
        commands,
        len
    );
    layout!(
        p,
        ghostty_config_palette_s,
        "ghostty_config_palette_s",
        colors
    );
    layout!(
        p,
        ghostty_quick_terminal_size_value_u,
        "ghostty_quick_terminal_size_value_u",
        percentage,
        pixels
    );
    layout!(
        p,
        ghostty_quick_terminal_size_s,
        "ghostty_quick_terminal_size_s",
        tag,
        value
    );
    layout!(
        p,
        ghostty_config_quick_terminal_size_s,
        "ghostty_config_quick_terminal_size_s",
        primary,
        secondary
    );
    layout!(p, ghostty_target_u, "ghostty_target_u", surface);
    layout!(p, ghostty_target_s, "ghostty_target_s", tag, target);
    layout!(
        p,
        ghostty_action_resize_split_s,
        "ghostty_action_resize_split_s",
        amount,
        direction
    );
    layout!(
        p,
        ghostty_action_move_tab_s,
        "ghostty_action_move_tab_s",
        amount
    );
    layout!(
        p,
        ghostty_action_desktop_notification_s,
        "ghostty_action_desktop_notification_s",
        title,
        body
    );
    layout!(
        p,
        ghostty_action_set_title_s,
        "ghostty_action_set_title_s",
        title
    );
    layout!(p, ghostty_action_pwd_s, "ghostty_action_pwd_s", pwd);
    layout!(
        p,
        ghostty_action_mouse_over_link_s,
        "ghostty_action_mouse_over_link_s",
        url,
        len
    );
    layout!(
        p,
        ghostty_action_size_limit_s,
        "ghostty_action_size_limit_s",
        min_width,
        min_height,
        max_width,
        max_height
    );
    layout!(
        p,
        ghostty_action_initial_size_s,
        "ghostty_action_initial_size_s",
        width,
        height
    );
    layout!(
        p,
        ghostty_action_cell_size_s,
        "ghostty_action_cell_size_s",
        width,
        height
    );
    layout!(
        p,
        ghostty_action_key_sequence_s,
        "ghostty_action_key_sequence_s",
        active,
        trigger
    );
    layout!(
        p,
        ghostty_action_key_table_u,
        "ghostty_action_key_table_u",
        activate
    );
    layout!(
        p,
        ghostty_action_key_table_s,
        "ghostty_action_key_table_s",
        tag,
        value
    );
    layout!(
        p,
        ghostty_action_color_change_s,
        "ghostty_action_color_change_s",
        kind,
        r,
        g,
        b
    );
    layout!(
        p,
        ghostty_action_config_change_s,
        "ghostty_action_config_change_s",
        config
    );
    layout!(
        p,
        ghostty_action_reload_config_s,
        "ghostty_action_reload_config_s",
        soft
    );
    layout!(
        p,
        ghostty_action_open_url_s,
        "ghostty_action_open_url_s",
        kind,
        url,
        len
    );
    layout!(
        p,
        ghostty_surface_message_childexited_s,
        "ghostty_surface_message_childexited_s",
        exit_code,
        timetime_ms
    );
    layout!(
        p,
        ghostty_action_progress_report_s,
        "ghostty_action_progress_report_s",
        state,
        progress
    );
    layout!(
        p,
        ghostty_action_command_finished_s,
        "ghostty_action_command_finished_s",
        exit_code,
        duration
    );
    layout!(
        p,
        ghostty_action_start_search_s,
        "ghostty_action_start_search_s",
        needle
    );
    layout!(
        p,
        ghostty_action_search_total_s,
        "ghostty_action_search_total_s",
        total
    );
    layout!(
        p,
        ghostty_action_search_selected_s,
        "ghostty_action_search_selected_s",
        selected
    );
    layout!(
        p,
        ghostty_action_scrollbar_s,
        "ghostty_action_scrollbar_s",
        total,
        offset,
        len
    );
    layout!(
        p,
        ghostty_action_u,
        "ghostty_action_u",
        new_split,
        toggle_fullscreen,
        move_tab,
        goto_tab,
        goto_split,
        goto_window,
        resize_split,
        size_limit,
        initial_size,
        cell_size,
        scrollbar,
        inspector,
        desktop_notification,
        set_title,
        set_tab_title,
        prompt_title,
        pwd,
        mouse_shape,
        mouse_visibility,
        mouse_over_link,
        renderer_health,
        quit_timer,
        float_window,
        secure_input,
        key_sequence,
        key_table,
        color_change,
        reload_config,
        config_change,
        open_url,
        close_tab_mode,
        child_exited,
        progress_report,
        command_finished,
        start_search,
        search_total,
        search_selected,
        readonly
    );
    layout!(p, ghostty_action_s, "ghostty_action_s", tag, action);
    layout!(
        p,
        ghostty_runtime_config_s,
        "ghostty_runtime_config_s",
        userdata,
        supports_selection_clipboard,
        wakeup_cb,
        action_cb,
        read_clipboard_cb,
        confirm_read_clipboard_cb,
        write_clipboard_cb,
        close_surface_cb
    );
    layout!(p, ghostty_ipc_target_u, "ghostty_ipc_target_u", klass);
    layout!(p, chostty_ipc_target_s, "chostty_ipc_target_s", tag, target);
    layout!(
        p,
        ghostty_ipc_action_new_window_s,
        "ghostty_ipc_action_new_window_s",
        arguments
    );
    layout!(p, ghostty_ipc_action_u, "ghostty_ipc_action_u", new_window);

    // ---- scalar typedefs and opaque handles ----
    for (ty, size) in [
        ("ghostty_app_t", size_of::<usize>()),
        ("ghostty_config_t", size_of::<usize>()),
        ("ghostty_surface_t", size_of::<usize>()),
        ("ghostty_inspector_t", size_of::<usize>()),
        ("ghostty_input_scroll_mods_t", 4),
        ("ghostty_runtime_wakeup_cb", size_of::<usize>()),
        ("ghostty_runtime_action_cb", size_of::<usize>()),
    ] {
        p.emit(
            &format!("sizeof({ty})"),
            &format!("sizeof({ty})"),
            size as i64,
        );
    }
    p.emit(
        "GHOSTTY_SUCCESS",
        "GHOSTTY_SUCCESS",
        i64::from(GHOSTTY_SUCCESS),
    );

    // ---- enums ----
    p.enumerators(
        &enums,
        "ghostty_platform_e",
        "GHOSTTY_PLATFORM_",
        &named(&[
            ("invalid", i64::from(platform::INVALID)),
            ("macos", i64::from(platform::MACOS)),
            ("ios", i64::from(platform::IOS)),
        ]),
    );
    p.enumerators(
        &enums,
        "ghostty_clipboard_e",
        "GHOSTTY_CLIPBOARD_",
        &vals(&[Clipboard::Standard, Clipboard::Selection], |v| v as i64),
    );
    p.enumerators(
        &enums,
        "ghostty_clipboard_request_e",
        "GHOSTTY_CLIPBOARD_REQUEST_",
        &named(&[
            ("paste", ClipboardRequestType::Paste as i64),
            ("osc_52_read", ClipboardRequestType::Osc52Read as i64),
            ("osc_52_write", ClipboardRequestType::Osc52Write as i64),
        ]),
    );
    p.enumerators(
        &enums,
        "ghostty_input_mouse_state_e",
        "GHOSTTY_MOUSE_",
        &vals(&[MouseButtonState::Release, MouseButtonState::Press], |v| {
            v as i64
        }),
    );
    p.enumerators(
        &enums,
        "ghostty_input_mouse_button_e",
        "GHOSTTY_MOUSE_",
        &vals(
            &[
                MouseButton::Unknown,
                MouseButton::Left,
                MouseButton::Right,
                MouseButton::Middle,
                MouseButton::Four,
                MouseButton::Five,
                MouseButton::Six,
                MouseButton::Seven,
                MouseButton::Eight,
                MouseButton::Nine,
                MouseButton::Ten,
                MouseButton::Eleven,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_input_mouse_momentum_e",
        "GHOSTTY_MOUSE_MOMENTUM_",
        &vals(
            &[
                Momentum::None,
                Momentum::Began,
                Momentum::Stationary,
                Momentum::Changed,
                Momentum::Ended,
                Momentum::Cancelled,
                Momentum::MayBegin,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_color_scheme_e",
        "GHOSTTY_COLOR_SCHEME_",
        &vals(&[ColorScheme::Light, ColorScheme::Dark], |v| v as i64),
    );
    let side = |f: fn(&mut Side)| {
        let mut s = Side::default();
        f(&mut s);
        Mods {
            sides: s,
            ..Mods::default()
        }
        .int() as i64
    };
    p.enumerators(
        &enums,
        "ghostty_input_mods_e",
        "GHOSTTY_MODS_",
        &named(&[
            ("none", 0),
            (
                "shift",
                Mods {
                    shift: true,
                    ..Mods::default()
                }
                .int()
                .into(),
            ),
            (
                "ctrl",
                Mods {
                    ctrl: true,
                    ..Mods::default()
                }
                .int()
                .into(),
            ),
            (
                "alt",
                Mods {
                    alt: true,
                    ..Mods::default()
                }
                .int()
                .into(),
            ),
            (
                "super",
                Mods {
                    super_: true,
                    ..Mods::default()
                }
                .int()
                .into(),
            ),
            (
                "caps",
                Mods {
                    caps_lock: true,
                    ..Mods::default()
                }
                .int()
                .into(),
            ),
            (
                "num",
                Mods {
                    num_lock: true,
                    ..Mods::default()
                }
                .int()
                .into(),
            ),
            ("shift_right", side(|s| s.shift = ModSide::Right)),
            ("ctrl_right", side(|s| s.ctrl = ModSide::Right)),
            ("alt_right", side(|s| s.alt = ModSide::Right)),
            ("super_right", side(|s| s.super_ = ModSide::Right)),
        ]),
    );
    let flag = |f: fn(&mut Flags)| {
        let mut fl = Flags {
            consumed: false,
            all: false,
            global: false,
            performable: false,
        };
        f(&mut fl);
        i64::from(fl.cval())
    };
    p.enumerators(
        &enums,
        "ghostty_binding_flags_e",
        "GHOSTTY_BINDING_FLAGS_",
        &named(&[
            ("consumed", flag(|f| f.consumed = true)),
            ("all", flag(|f| f.all = true)),
            ("global", flag(|f| f.global = true)),
            ("performable", flag(|f| f.performable = true)),
        ]),
    );
    p.enumerators(
        &enums,
        "ghostty_input_action_e",
        "GHOSTTY_ACTION_",
        &vals(
            &[KeyAction::Release, KeyAction::Press, KeyAction::Repeat],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_input_key_e",
        "GHOSTTY_KEY_",
        &Key::ALL
            .iter()
            .enumerate()
            .map(|(i, k)| {
                // The header names the letter keys `A`..`Z` (`KeyA` in the
                // W3C code table).
                let n = k.name();
                let n = if n.len() == 5 {
                    n.strip_prefix("key_").unwrap_or(n)
                } else {
                    n
                };
                (norm(n), i as i64)
            })
            .collect(),
    );
    p.enumerators(
        &enums,
        "ghostty_input_trigger_tag_e",
        "GHOSTTY_TRIGGER_",
        &named(&[
            ("physical", i64::from(trigger_tag::PHYSICAL)),
            ("unicode", i64::from(trigger_tag::UNICODE)),
            ("catch_all", i64::from(trigger_tag::CATCH_ALL)),
        ]),
    );
    p.enumerators(
        &enums,
        "ghostty_build_mode_e",
        "GHOSTTY_BUILD_MODE_",
        &named(&[
            ("debug", i64::from(build_mode::DEBUG)),
            ("release_safe", i64::from(build_mode::RELEASE_SAFE)),
            ("release_fast", i64::from(build_mode::RELEASE_FAST)),
            ("release_small", i64::from(build_mode::RELEASE_SMALL)),
        ]),
    );
    p.enumerators(
        &enums,
        "ghostty_point_tag_e",
        "GHOSTTY_POINT_",
        // The header calls the fourth tag `SURFACE`; the Zig `Point.Tag`
        // calls it `history` (value 3).
        &named(&[
            ("active", 0),
            ("viewport", 1),
            ("screen", 2),
            ("surface", 3),
        ]),
    );
    p.enumerators(
        &enums,
        "ghostty_point_coord_e",
        "GHOSTTY_POINT_COORD_",
        &named(&[("exact", 0), ("top_left", 1), ("bottom_right", 2)]),
    );
    p.enumerators(
        &enums,
        "ghostty_surface_context_e",
        "GHOSTTY_SURFACE_CONTEXT_",
        &vals(
            &[
                NewSurfaceContext::Window,
                NewSurfaceContext::Tab,
                NewSurfaceContext::Split,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_quick_terminal_size_tag_e",
        "GHOSTTY_QUICK_TERMINAL_SIZE_",
        &named(&[
            ("none", i64::from(quick_terminal_size_tag::NONE)),
            ("percentage", i64::from(quick_terminal_size_tag::PERCENTAGE)),
            ("pixels", i64::from(quick_terminal_size_tag::PIXELS)),
        ]),
    );
    p.enumerators(
        &enums,
        "ghostty_config_fullscreen_e",
        "GHOSTTY_CONFIG_FULLSCREEN_",
        &ghostty_config::types_a::Fullscreen::ALL
            .iter()
            .enumerate()
            .map(|(i, v)| (norm(&format!("{v:?}")), i as i64))
            .collect(),
    );
    p.enumerators(
        &enums,
        "ghostty_target_tag_e",
        "GHOSTTY_TARGET_",
        &named(&[
            ("app", i64::from(target_tag::APP)),
            ("surface", i64::from(target_tag::SURFACE)),
        ]),
    );
    use action as a;
    p.enumerators(
        &enums,
        "ghostty_action_split_direction_e",
        "GHOSTTY_SPLIT_DIRECTION_",
        &vals(
            &[
                a::SplitDirection::Right,
                a::SplitDirection::Down,
                a::SplitDirection::Left,
                a::SplitDirection::Up,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_goto_split_e",
        "GHOSTTY_GOTO_SPLIT_",
        &vals(
            &[
                a::GotoSplit::Previous,
                a::GotoSplit::Next,
                a::GotoSplit::Up,
                a::GotoSplit::Left,
                a::GotoSplit::Down,
                a::GotoSplit::Right,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_goto_window_e",
        "GHOSTTY_GOTO_WINDOW_",
        &vals(&[a::GotoWindow::Previous, a::GotoWindow::Next], |v| {
            v as i64
        }),
    );
    p.enumerators(
        &enums,
        "ghostty_action_resize_split_direction_e",
        "GHOSTTY_RESIZE_SPLIT_",
        &vals(
            &[
                a::ResizeSplitDirection::Up,
                a::ResizeSplitDirection::Down,
                a::ResizeSplitDirection::Left,
                a::ResizeSplitDirection::Right,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_goto_tab_e",
        "GHOSTTY_GOTO_TAB_",
        &named(&[
            ("previous", i64::from(a::GotoTab::Previous.int())),
            ("next", i64::from(a::GotoTab::Next.int())),
            ("last", i64::from(a::GotoTab::Last.int())),
        ]),
    );
    p.enumerators(
        &enums,
        "ghostty_action_fullscreen_e",
        "GHOSTTY_FULLSCREEN_",
        &vals(
            &[
                a::Fullscreen::Native,
                a::Fullscreen::MacosNonNative,
                a::Fullscreen::MacosNonNativeVisibleMenu,
                a::Fullscreen::MacosNonNativePaddedNotch,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_float_window_e",
        "GHOSTTY_FLOAT_WINDOW_",
        &vals(
            &[
                a::FloatWindow::On,
                a::FloatWindow::Off,
                a::FloatWindow::Toggle,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_secure_input_e",
        "GHOSTTY_SECURE_INPUT_",
        &vals(
            &[
                a::SecureInput::On,
                a::SecureInput::Off,
                a::SecureInput::Toggle,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_inspector_e",
        "GHOSTTY_INSPECTOR_",
        &vals(
            &[a::Inspector::Toggle, a::Inspector::Show, a::Inspector::Hide],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_quit_timer_e",
        "GHOSTTY_QUIT_TIMER_",
        &vals(&[a::QuitTimer::Start, a::QuitTimer::Stop], |v| v as i64),
    );
    p.enumerators(
        &enums,
        "ghostty_action_readonly_e",
        "GHOSTTY_READONLY_",
        &vals(&[a::Readonly::Off, a::Readonly::On], |v| v as i64),
    );
    p.enumerators(
        &enums,
        "ghostty_action_prompt_title_e",
        "GHOSTTY_PROMPT_TITLE_",
        &vals(&[a::PromptTitle::Surface, a::PromptTitle::Tab], |v| {
            v as i64
        }),
    );
    use MouseShape as M;
    p.enumerators(
        &enums,
        "ghostty_action_mouse_shape_e",
        "GHOSTTY_MOUSE_SHAPE_",
        &vals(
            &[
                M::Default,
                M::ContextMenu,
                M::Help,
                M::Pointer,
                M::Progress,
                M::Wait,
                M::Cell,
                M::Crosshair,
                M::Text,
                M::VerticalText,
                M::Alias,
                M::Copy,
                M::Move,
                M::NoDrop,
                M::NotAllowed,
                M::Grab,
                M::Grabbing,
                M::AllScroll,
                M::ColResize,
                M::RowResize,
                M::NResize,
                M::EResize,
                M::SResize,
                M::WResize,
                M::NeResize,
                M::NwResize,
                M::SeResize,
                M::SwResize,
                M::EwResize,
                M::NsResize,
                M::NeswResize,
                M::NwseResize,
                M::ZoomIn,
                M::ZoomOut,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_mouse_visibility_e",
        "GHOSTTY_MOUSE_",
        &vals(
            &[a::MouseVisibility::Visible, a::MouseVisibility::Hidden],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_renderer_health_e",
        "GHOSTTY_RENDERER_HEALTH_",
        &named(&[("healthy", 0), ("unhealthy", 1)]),
    );
    p.enumerators(
        &enums,
        "ghostty_action_key_table_tag_e",
        "GHOSTTY_KEY_TABLE_",
        &named(&[("activate", 0), ("deactivate", 1), ("deactivate_all", 2)]),
    );
    p.enumerators(
        &enums,
        "ghostty_action_color_kind_e",
        "GHOSTTY_ACTION_COLOR_KIND_",
        &named(&[
            ("foreground", i64::from(a::ColorKind::Foreground.int())),
            ("background", i64::from(a::ColorKind::Background.int())),
            ("cursor", i64::from(a::ColorKind::Cursor.int())),
        ]),
    );
    p.enumerators(
        &enums,
        "ghostty_action_open_url_kind_e",
        "GHOSTTY_ACTION_OPEN_URL_KIND_",
        &vals(
            &[
                a::OpenUrlKind::Unknown,
                a::OpenUrlKind::Text,
                a::OpenUrlKind::Html,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_close_tab_mode_e",
        "GHOSTTY_ACTION_CLOSE_TAB_MODE_",
        &vals(
            &[
                a::CloseTabMode::This,
                a::CloseTabMode::Other,
                a::CloseTabMode::Right,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_action_progress_report_state_e",
        "GHOSTTY_PROGRESS_STATE_",
        &named(&[
            ("remove", 0),
            ("set", 1),
            ("error", 2),
            ("indeterminate", 3),
            ("pause", 4),
        ]),
    );
    use ActionKey as K;
    p.enumerators(
        &enums,
        "ghostty_action_tag_e",
        "GHOSTTY_ACTION_",
        &vals(
            &[
                K::Quit,
                K::NewWindow,
                K::NewTab,
                K::CloseTab,
                K::NewSplit,
                K::CloseAllWindows,
                K::ToggleMaximize,
                K::ToggleFullscreen,
                K::ToggleTabOverview,
                K::ToggleWindowDecorations,
                K::ToggleQuickTerminal,
                K::ToggleCommandPalette,
                K::ToggleVisibility,
                K::ToggleBackgroundOpacity,
                K::MoveTab,
                K::GotoTab,
                K::GotoSplit,
                K::GotoWindow,
                K::ResizeSplit,
                K::EqualizeSplits,
                K::ToggleSplitZoom,
                K::PresentTerminal,
                K::SizeLimit,
                K::ResetWindowSize,
                K::InitialSize,
                K::CellSize,
                K::Scrollbar,
                K::Render,
                K::Inspector,
                K::ShowGtkInspector,
                K::RenderInspector,
                K::DesktopNotification,
                K::SetTitle,
                K::SetTabTitle,
                K::PromptTitle,
                K::Pwd,
                K::MouseShape,
                K::MouseVisibility,
                K::MouseOverLink,
                K::RendererHealth,
                K::OpenConfig,
                K::QuitTimer,
                K::FloatWindow,
                K::SecureInput,
                K::KeySequence,
                K::KeyTable,
                K::ColorChange,
                K::ReloadConfig,
                K::ConfigChange,
                K::CloseWindow,
                K::RingBell,
                K::Undo,
                K::Redo,
                K::CheckForUpdates,
                K::OpenUrl,
                K::ShowChildExited,
                K::ProgressReport,
                K::ShowOnScreenKeyboard,
                K::CommandFinished,
                K::StartSearch,
                K::EndSearch,
                K::SearchTotal,
                K::SearchSelected,
                K::Readonly,
                K::CopyTitleToClipboard,
            ],
            |v| v as i64,
        ),
    );
    p.enumerators(
        &enums,
        "ghostty_ipc_target_tag_e",
        "GHOSTTY_IPC_TARGET_",
        &named(&[("class", 0), ("detect", 1)]),
    );
    p.enumerators(
        &enums,
        "ghostty_ipc_action_tag_e",
        "GHOSTTY_IPC_ACTION_",
        &named(&[("new_window", 0)]),
    );

    // Every enum and record of the header must be covered.
    let all_enums: BTreeSet<_> = enums.keys().cloned().collect();
    assert_eq!(all_enums, p.covered_enums, "uncovered enums");
    let all_records: BTreeSet<_> = p.records.keys().cloned().collect();
    assert_eq!(all_records, p.covered_records, "uncovered structs/unions");
    p
}

/// The enum `ghostty_action_tag_e` has the same size as the Rust key.
#[test]
fn action_tags_fit_the_union() {
    assert_eq!(size_of::<ghostty_action_u>(), 24);
    assert_eq!(size_of::<ghostty_action_s>(), 32);
    assert_eq!(size_of::<ghostty_target_s>(), 16);
}

#[test]
fn abi_matches_header() {
    let probe = build_probe();
    let lib_dir = common::build_lib();
    let dir = common::scratch_dir("abi");

    // Take the address of every declared function so linking proves the
    // library exports it.
    let funcs = common::declared_functions();
    assert!(funcs.len() > 80, "parsed only {} functions", funcs.len());
    let mut c = String::from(
        "#include <stdio.h>\n#include <stddef.h>\n#include <stdint.h>\n#include <ghostty.h>\n\n",
    );
    c.push_str("static void *const funcs[] = {\n");
    for f in &funcs {
        let _ = writeln!(c, "  (void *)&{f},");
    }
    c.push_str("};\n\nint main(void) {\n");
    c.push_str("  if (sizeof(funcs) == 0) return 2;\n");
    c.push_str(&probe.c);
    c.push_str("  return 0;\n}\n");
    let src = dir.join("probe.c");
    std::fs::write(&src, c).unwrap();
    let exe = dir.join("probe");
    common::compile_c(&lib_dir, &[&src], &exe, &["-Wno-deprecated-declarations"]);

    let out = Command::new(&exe).output().unwrap();
    assert!(out.status.success(), "probe failed");
    let got: BTreeMap<String, i64> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            let (k, v) = l.rsplit_once('=').unwrap();
            (k.to_string(), v.parse().unwrap())
        })
        .collect();

    let mut bad = Vec::new();
    for (k, want) in &probe.expected {
        match got.get(k) {
            Some(g) if g == want => {}
            Some(g) => bad.push(format!("{k}: C={g} Rust={want}")),
            None => bad.push(format!("{k}: missing from C output")),
        }
    }
    assert!(bad.is_empty(), "ABI mismatches:\n{}", bad.join("\n"));
    assert!(got.len() > 700, "only {} values checked", got.len());
}

#[test]
fn exports_match_header() {
    let lib_dir = common::build_lib();
    let so = ["libghostty.so", "libghostty.dylib"]
        .iter()
        .map(|n| lib_dir.join(n))
        .find(|p| p.exists())
        .expect("shared library");
    let out = Command::new("nm").arg("-D").arg(&so).output().unwrap();
    assert!(out.status.success());
    let exported: BTreeSet<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (_addr, kind, name) = (it.next()?, it.next()?, it.next()?);
            (kind == "T" && name.starts_with("ghostty_")).then(|| name.to_string())
        })
        .collect();
    let declared: BTreeSet<String> = common::declared_functions().into_iter().collect();
    let missing: Vec<_> = declared.difference(&exported).collect();
    let extra: Vec<_> = exported.difference(&declared).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "missing exports: {missing:?}\nextra exports: {extra:?}"
    );
}

#[test]
fn header_files_are_unchanged_copies() {
    let zig = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ghostty/include");
    if !zig.exists() {
        eprintln!("skipping: Zig checkout not found");
        return;
    }
    for f in ["ghostty.h", "module.modulemap"] {
        let a = std::fs::read(zig.join(f)).unwrap();
        let b = std::fs::read(common::include_dir().join(f)).unwrap();
        assert_eq!(a, b, "{f} differs from upstream");
    }
}
