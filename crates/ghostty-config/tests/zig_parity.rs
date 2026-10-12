//! Parity regressions found by differential runs against Ghostty 1.3.1:
//! numeric grammars, field-specific error kinds for non-UTF-8 input and
//! float formatting.
#![allow(clippy::unwrap_used)]
use ghostty_config::{cli_args::ArgError, config::Config};

fn parse(key: &str, value: &[u8]) -> Result<Config, ArgError> {
    let mut config = Config::default();
    config.parse_field_bytes(key, Some(value))?;
    Ok(config)
}

fn kind(key: &str, value: &[u8]) -> String {
    parse(key, value).map_or_else(|e| e.to_string(), |_| "ok".to_string())
}

#[test]
fn numeric_grammars_accept_zig_spellings() {
    assert_eq!(parse("window-height", b"0X10").unwrap().window_height, 16);
    assert_eq!(parse("window-height", b"1__0").unwrap().window_height, 10);
    assert_eq!(parse("window-height", b"0b1_0").unwrap().window_height, 2);
    assert_eq!(parse("font-size", b"1_4").unwrap().font_size, 14.0);
    assert_eq!(parse("font-size", b"0x1.cp+3").unwrap().font_size, 14.0);
    assert_eq!(
        parse("window-padding-x", b"1_0")
            .unwrap()
            .window_padding_x
            .top_left,
        10
    );
    assert_eq!(kind("window-padding-x", b"0x10"), "InvalidValue");
    assert_eq!(kind("window-height", b"1_"), "InvalidValue");
    assert_eq!(kind("font-size", b"1__0"), "InvalidValue");
    // Decimal-only places keep rejecting prefixes.
    assert_eq!(kind("adjust-cell-width", b"0x10"), "InvalidFormat");
    assert_eq!(kind("adjust-cell-width", b"1_0"), "ok");
    assert_eq!(kind("adjust-cell-width", b"0x1p3%"), "ok");
    assert_eq!(kind("quick-terminal-size", b"1_0px"), "ok");
    assert_eq!(kind("quick-terminal-size", b"0x1p3%,1_0px"), "ok");
    assert_eq!(kind("quick-terminal-size", b"0x10px"), "InvalidValue");
    assert_eq!(kind("font-variation", b"wght=1_0.5"), "ok");
    assert_eq!(kind("font-variation", b"wght=0x1p4"), "ok");
    assert_eq!(kind("mouse-scroll-multiplier", b"1_0"), "ok");
    assert_eq!(kind("palette", b"0x10=#fff"), "ok");
    assert_eq!(kind("palette", b"256=#fff"), "Overflow");
    assert_eq!(kind("palette", b"-1=#fff"), "Overflow");
    assert_eq!(kind("palette", b"-0=#fff"), "ok");
    assert_eq!(kind("palette", b"1x=#fff"), "InvalidCharacter");
    assert_eq!(kind("background-blur", b"0b11"), "ok");
    assert_eq!(kind("keybind", b"a=scroll_page_lines:1_0"), "ok");
    assert_eq!(
        kind("keybind", b"a=scroll_page_lines:0x10"),
        "InvalidFormat"
    );
    assert_eq!(kind("keybind", b"a=set_font_size:1_4"), "ok");
    assert_eq!(kind("keybind", b"a=set_font_size:0x1.cp+3"), "ok");
    assert_eq!(kind("keybind", b"a=resize_split:up,1_0"), "ok");
    assert_eq!(kind("keybind", b"a=goto_tab:1__0"), "ok");
    assert_eq!(kind("keybind", b"a=goto_tab:_1"), "InvalidFormat");
}

#[test]
fn invalid_bytes_keep_field_specific_error_kinds() {
    for key in [
        "adjust-cell-width",
        "adjust-cursor-height",
        "adjust-icon-height",
    ] {
        assert_eq!(kind(key, b"\xff"), "InvalidFormat", "{key}");
        assert_eq!(kind(key, b"10px\xff"), "InvalidFormat", "{key}");
        assert_eq!(kind(key, b"\xff%"), "InvalidFormat", "{key}");
    }
    assert_eq!(kind("quick-terminal-size", b"\xff"), "MissingUnit");
    assert_eq!(kind("quick-terminal-size", b"a\xffb"), "MissingUnit");
    assert_eq!(kind("quick-terminal-size", b"\xff%"), "InvalidValue");
    assert_eq!(kind("quick-terminal-size", b"1px,\xff"), "MissingUnit");
    assert_eq!(kind("link", b"\xff"), "NotImplemented");
    assert_eq!(kind("link", b"x"), "NotImplemented");
    for key in [
        "background",
        "foreground",
        "cursor-color",
        "bold-color",
        "split-divider-color",
    ] {
        assert_eq!(kind(key, b"a\xffb"), "InvalidCharacter", "{key}");
        assert_eq!(kind(key, b"\xff"), "InvalidValue", "{key}");
    }
    assert_eq!(kind("palette", b"1=a\xffb"), "InvalidCharacter");
    assert_eq!(kind("palette", b"1=\xff"), "InvalidValue");
    assert_eq!(kind("keybind", b"\xff=a"), "InvalidAction");
    assert_eq!(kind("keybind", b"\xff=ignore"), "InvalidFormat");
    assert_eq!(kind("keybind", b"ctrl+a=goto_tab:\xff"), "InvalidFormat");
    assert_eq!(kind("keybind", b"ctrl+a=nope:\xff"), "InvalidAction");
    assert_eq!(kind("keybind", b"ctrl+a=text:\xff"), "ok");
}

#[test]
fn failed_background_blur_parse_leaves_a_zero_radius_like_zig() {
    use ghostty_config::types_b::BackgroundBlur;
    let mut config = Config::default();
    assert!(
        config
            .parse_field_bytes("background-blur", Some(b"bad"))
            .is_err()
    );
    assert_eq!(config.background_blur, BackgroundBlur::Radius(0));
    config
        .parse_field_bytes("background-blur", Some(b"7"))
        .unwrap();
    assert!(
        config
            .parse_field_bytes("background-blur", Some(b"\xff"))
            .is_err()
    );
    assert_eq!(config.background_blur, BackgroundBlur::Radius(7));
}

#[test]
fn float_nan_formats_like_zig_and_finalize_clamps_like_zig() {
    let mut config = parse("font-size", b"nan").unwrap();
    let mut out = String::new();
    ghostty_config::formatter::format_entry("font-size", &config.font_size, &mut out).unwrap();
    assert_eq!(out, "font-size = nan\n");

    config
        .parse_field_bytes("faint-opacity", Some(b"nan"))
        .unwrap();
    config
        .parse_field_bytes("unfocused-split-opacity", Some(b"nan"))
        .unwrap();
    config
        .parse_field_bytes("minimum-contrast", Some(b"nan"))
        .unwrap();
    config
        .parse_field_bytes("mouse-scroll-multiplier", Some(b"nan"))
        .unwrap();
    config.finalize().unwrap();
    assert_eq!(config.faint_opacity, 1.0);
    assert_eq!(config.unfocused_split_opacity, 0.15);
    assert_eq!(config.minimum_contrast, 1.0);
    assert_eq!(config.mouse_scroll_multiplier.precision, 0.01);

    let mut config = parse("faint-opacity", b"-0").unwrap();
    config.finalize().unwrap();
    assert!(config.faint_opacity.is_sign_positive());
}
