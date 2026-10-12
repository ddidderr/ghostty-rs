#![allow(clippy::unwrap_used, clippy::assert_is_empty)]
use ghostty_config::{
    cli_args::{ArgError, ArgsIterator, LineIterator},
    config::Config,
};
use ghostty_input::binding::{Action, Trigger};

fn load(bytes: &[u8], cli: bool) -> Config {
    let mut config = Config::default();
    if cli {
        let arg = [b"--".as_slice(), bytes].concat();
        config
            .load_iter(&mut ArgsIterator::new([arg].into_iter()))
            .unwrap();
    } else {
        config.load_iter(&mut LineIterator::new(bytes)).unwrap();
    }
    config
}

#[test]
fn literal_fdd0_is_a_single_unicode_trigger() {
    for cli in [false, true] {
        let config = load("keybind=\u{fdd0}=text:ok".as_bytes(), cli);
        assert!(config.diagnostics.items().is_empty());
        let trigger = Trigger::parse("\u{fdd0}").unwrap();
        assert_eq!(
            config
                .keybind
                .set
                .get(trigger)
                .unwrap()
                .as_leaf()
                .unwrap()
                .action,
            Action::Text("ok".into())
        );
    }
}

#[test]
fn selection_word_chars_reject_invalid_utf8() {
    for cli in [false, true] {
        let config = load(b"selection-word-chars=\xff", cli);
        assert_eq!(config.diagnostics.items().len(), 1);
        assert!(
            config.diagnostics.items()[0]
                .message
                .starts_with("invalid value")
        );
    }
    let mut config = Config::default();
    assert_eq!(
        config.parse_field_bytes("selection-word-chars", Some(b"\xed\xa0\x80")),
        Err(ArgError::InvalidValue)
    );
    config
        .parse_field("selection-word-chars", Some("\u{fdd0}é"))
        .unwrap();
    assert_eq!(config.selection_word_chars.codepoints, [0, 0xfdd0, 0xe9]);
}

#[test]
fn clipboard_replacements_require_utf8_and_keep_literal_fdd0() {
    for cli in [false, true] {
        let config = load(b"clipboard-codepoint-map=U+0041=\xff", cli);
        assert_eq!(config.diagnostics.items().len(), 1);
        assert!(config.clipboard_codepoint_map.map.list.is_empty());
    }
    let config = load("clipboard-codepoint-map=U+0041=\u{fdd0}é".as_bytes(), false);
    assert_eq!(
        config.clipboard_codepoint_map.map.list[0].replacement,
        ghostty_terminal::formatter::Replacement::String("\u{fdd0}é".into())
    );
}

#[test]
fn font_style_and_codepoint_family_keep_original_bytes() {
    let config = load(
        "font-style=Name\u{fdd0}é\nfont-codepoint-map=U+0041=Family\u{fdd0}é".as_bytes(),
        false,
    );
    assert_eq!(
        config.font_style.name_value().unwrap().to_bytes(),
        "Name\u{fdd0}é".as_bytes()
    );
    assert_eq!(
        config.font_codepoint_map.map.list[0]
            .descriptor
            .family
            .as_ref()
            .unwrap()
            .to_bytes(),
        "Family\u{fdd0}é".as_bytes()
    );
    let config = load(b"font-codepoint-map=U+0041=Family\xff", true);
    assert!(config.diagnostics.items().is_empty());
    assert_eq!(
        config.font_codepoint_map.map.list[0]
            .descriptor
            .family
            .as_ref()
            .unwrap()
            .to_bytes(),
        b"Family\xff"
    );
}

#[test]
fn byte_fields_and_auto_struct_values_never_parse_escape_encoding() {
    let raw = "x\u{fdd0}é".as_bytes();
    for cli in [false, true] {
        for key in ["title", "term", "enquiry-response", "language"] {
            let config = load(&[key.as_bytes(), b"=", raw, b"\xff"].concat(), cli);
            assert!(config.diagnostics.items().is_empty(), "{key}");
            let value = match key {
                "title" => config.title.unwrap(),
                "term" => config.term,
                "enquiry-response" => config.enquiry_response,
                _ => config.language.unwrap(),
            };
            assert_eq!(value.to_bytes(), [raw, b"\xff"].concat());
        }
    }
    let config = load(b"env=K\xff=V\xfe\ncommand=direct:exe arg\xff\ninput=raw:in\xff\\n\ncommand-palette-entry=title:\"name\\xff\",action:\"text:payload\\xfe\"", false);
    assert!(config.diagnostics.items().is_empty());
    let (key, value) = config.env.iter().next().unwrap();
    assert_eq!(key.to_bytes(), b"K\xff");
    assert_eq!(value.to_bytes(), b"V\xfe");
    assert_eq!(
        config
            .command
            .unwrap()
            .arg_iterator()
            .last()
            .unwrap()
            .to_bytes(),
        b"arg\xff"
    );
    let ghostty_config::io::ReadableIO::Raw(input) = config.input.list[0].clone_parsed() else {
        panic!("raw input")
    };
    assert_eq!(input.to_bytes(), b"in\xff\n");
    let entry = config.command_palette_entry.value.last().unwrap();
    assert_eq!(entry.title.to_bytes(), b"name\xff");
    assert_eq!(
        entry.action,
        Action::Text(ghostty_os::osbytes::EncodedOs::from_bytes(b"payload\xfe"))
    );
}

#[test]
fn byte_fields_trim_zig_ascii_whitespace() {
    for cli in [false, true] {
        let config = load(b"env=\x0bKEY\x0b=\x0bVALUE\x0b", cli);
        assert_eq!(config.env.get("KEY").unwrap().to_bytes(), b"VALUE");
        let config = load(b"working-directory=\x0binherit\x0b", cli);
        assert_eq!(
            config.working_directory,
            Some(ghostty_config::types_a::WorkingDirectory::Inherit)
        );
    }
}

#[test]
fn font_variation_tags_are_four_original_bytes() {
    for cli in [false, true] {
        for tag in [b"a\xffcd".as_slice(), "\u{fdd0}x".as_bytes()] {
            let config = load(&[b"font-variation=", tag, b"=12"].concat(), cli);
            assert!(config.diagnostics.items().is_empty());
            assert_eq!(config.font_variation.list[0].id.str(), tag);
            assert_eq!(config.font_variation.list[0].value, 12.0);
            let mut formatted = String::new();
            ghostty_config::formatter::format_entry(
                "font-variation",
                &config.font_variation,
                &mut formatted,
            )
            .unwrap();
            assert_eq!(
                ghostty_os::osbytes::config_str_to_bytes(&formatted),
                [b"font-variation = ", tag, b"=12\n"].concat()
            );
        }
    }
}
