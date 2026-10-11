use super::*;

fn map_entry(range: (char, char), replacement: Replacement) -> CodepointMap {
    CodepointMap {
        range: [range.0 as u32, range.1 as u32],
        replacement,
    }
}

/// Format the last page of `t` with `opts`, optionally collecting a point map.
fn run(t: &Terminal, opts: &Options, point_map: Option<&mut Vec<Coordinate>>) -> String {
    let mut f = PageFormatter::new(last_page(t), opts);
    f.point_map = point_map;
    f.format_string().unwrap()
}

fn plain_with_map(entries: Vec<CodepointMap>) -> Options {
    let mut opts = Options::plain();
    opts.codepoint_map = Some(entries);
    opts
}

fn c(x: u16, y: u32) -> Coordinate {
    Coordinate { x, y }
}

#[test]
fn page_codepoint_map_single_replacement() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello world");

    // Replace 'o' with 'x'
    let opts = plain_with_map(vec![map_entry(
        ('o', 'o'),
        Replacement::Codepoint('x' as u32),
    )]);
    let mut point_map = Vec::new();
    let output = run(&t, &opts, Some(&mut point_map));
    assert_eq!(output, "hellx wxrld");

    // Verify point map - each output byte should map to original cell position
    assert_eq!(output.len(), point_map.len());
    for (i, coord) in point_map.iter().enumerate() {
        assert_eq!(*coord, c(i as u16, 0));
    }
}

#[test]
fn page_codepoint_map_conflicting_replacement_prefers_last() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello");

    // Replace 'o' with 'x', then with 'y' - should prefer last
    let opts = plain_with_map(vec![
        map_entry(('o', 'o'), Replacement::Codepoint('x' as u32)),
        map_entry(('o', 'o'), Replacement::Codepoint('y' as u32)),
    ]);
    assert_eq!(run(&t, &opts, None), "helly");
}

#[test]
fn page_codepoint_map_replace_with_string() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello");

    // Replace 'o' with a multi-byte string
    let opts = plain_with_map(vec![map_entry(
        ('o', 'o'),
        Replacement::String("XYZ".into()),
    )]);
    let mut point_map = Vec::new();
    let output = run(&t, &opts, Some(&mut point_map));
    assert_eq!(output, "hellXYZ");

    // Verify point map - string replacements should all map to the original cell
    assert_eq!(output.len(), point_map.len());
    for i in 0..4 {
        assert_eq!(point_map[i], c(i as u16, 0));
    }
    // All bytes of the replacement string "XYZ" should point to position 4 (where 'o' was)
    assert_eq!(point_map[4], c(4, 0)); // X
    assert_eq!(point_map[5], c(4, 0)); // Y
    assert_eq!(point_map[6], c(4, 0)); // Z
}

#[test]
fn page_codepoint_map_range_replacement() {
    let mut t = term(80, 24);
    feed(&mut t, b"abcdefg");

    // Replace 'b' through 'e' with 'X'
    let opts = plain_with_map(vec![map_entry(
        ('b', 'e'),
        Replacement::Codepoint('X' as u32),
    )]);
    assert_eq!(run(&t, &opts, None), "aXXXXfg");
}

#[test]
fn page_codepoint_map_multiple_ranges() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello world");

    // Replace 'a'-'m' with 'A' and 'n'-'z' with 'Z'
    let opts = plain_with_map(vec![
        map_entry(('a', 'm'), Replacement::Codepoint('A' as u32)),
        map_entry(('n', 'z'), Replacement::Codepoint('Z' as u32)),
    ]);
    // h e l l o   w o r l d
    // A A A A Z   Z Z Z A A
    assert_eq!(run(&t, &opts, None), "AAAAZ ZZZAA");
}

#[test]
fn page_codepoint_map_unicode_replacement() {
    let mut t = term(80, 24);
    feed(&mut t, "hello \u{26A1} world".as_bytes());

    // Replace lightning bolt with fire emoji
    let opts = plain_with_map(vec![map_entry(
        ('\u{26A1}', '\u{26A1}'),
        Replacement::String("\u{1F525}".into()),
    )]);
    let mut point_map = Vec::new();
    let output = run(&t, &opts, Some(&mut point_map));
    assert_eq!(output, "hello \u{1F525} world");

    // Verify point map
    assert_eq!(output.len(), point_map.len());
    // Note: the bolt is a wide character occupying cells 6-7
    for i in 0..6 {
        assert_eq!(point_map[i], c(i as u16, 0));
    }
    // The fire emoji is 4 UTF-8 bytes, all should map to cell 6 (where the bolt was)
    let fire_start = 6; // "hello " is 6 bytes
    for i in 0..4 {
        assert_eq!(point_map[fire_start + i], c(6, 0));
    }
    // " world" follows
    let world_start = fire_start + 4;
    for i in 0..6 {
        assert_eq!(point_map[world_start + i], c(8 + i as u16, 0));
    }
}

#[test]
fn page_codepoint_map_with_styled_formats() {
    let mut t = term(10, 24);
    feed(&mut t, b"\x1b[31mred text\x1b[0m");

    // Replace 'e' with 'X' in styled text
    let mut opts = Options::vt();
    opts.codepoint_map = Some(vec![map_entry(
        ('e', 'e'),
        Replacement::Codepoint('X' as u32),
    )]);
    // Should preserve styles while replacing text
    // "red text" becomes "rXd tXxt"
    // VT format uses \x1b[38;5;1m for palette color 1
    assert_eq!(run(&t, &opts, None), "\x1b[0m\x1b[38;5;1mrXd tXxt\x1b[0m");
}

#[test]
fn page_codepoint_map_empty_map() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello world");

    // Empty map should not change anything
    let opts = plain_with_map(Vec::new());
    assert_eq!(run(&t, &opts, None), "hello world");

    // A None map is equivalent.
    let mut opts = Options::plain();
    opts.codepoint_map = None;
    assert_eq!(run(&t, &opts, None), "hello world");
}

#[test]
fn page_vt_background_color_on_trailing_blank_cells() {
    // This test reproduces a bug where trailing cells with background color
    // but no text are emitted as plain spaces without SGR sequences.
    // This causes TUIs like htop to lose background colors on rehydration.
    let mut t = term(20, 5);

    // Simulate a TUI row: "CPU:" with text, then trailing cells with red background
    // to end of line (no text after the colored region).
    // \x1b[41m sets red background, then EL fills rest of row with that bg.
    feed(&mut t, b"CPU:\x1b[41m\x1b[K");
    // Reset colors and move to next line with different content
    feed(&mut t, b"\x1b[0m\r\nline2");

    let mut opts = Options::vt();
    opts.trim = false; // Don't trim so we can see the trailing behavior
    let output = run(&t, &opts, None);

    // The output should preserve the red background SGR for trailing cells on line 1.
    // The red background should appear BEFORE the newline, not after.

    // Find position of CRLF
    let crlf_pos = output.find("\r\n").expect("crlf");

    // Check that red background (48;5;1) appears BEFORE the newline (on line 1)
    let line1 = &output[..crlf_pos];
    let has_red_bg_line1 = line1.contains("\x1b[41m") || line1.contains("\x1b[48;5;1m");
    assert!(has_red_bg_line1);
}

const HTML_OPEN: &str = "<div style=\"font-family: monospace; white-space: pre;\">";

fn html(t: &Terminal, point_map: Option<&mut Vec<Coordinate>>) -> String {
    run(t, &Options::html(), point_map)
}

#[test]
fn page_html_with_hyperlinks() {
    let mut t = term(80, 24);

    // Start a hyperlink, write some text, end it
    feed(
        &mut t,
        b"\x1b]8;;https://example.com\x1b\\link text\x1b]8;;\x1b\\ normal",
    );

    assert_eq!(
        html(&t, None),
        format!("{HTML_OPEN}<a href=\"https://example.com\">link text</a> normal</div>")
    );
}

#[test]
fn page_html_with_multiple_hyperlinks() {
    let mut t = term(80, 24);

    // Two different hyperlinks
    feed(
        &mut t,
        b"\x1b]8;;https://first.com\x1b\\first\x1b]8;;\x1b\\ ",
    );
    feed(
        &mut t,
        b"\x1b]8;;https://second.com\x1b\\second\x1b]8;;\x1b\\",
    );

    assert_eq!(
        html(&t, None),
        format!(
            "{HTML_OPEN}<a href=\"https://first.com\">first</a> <a href=\"https://second.com\">second</a></div>"
        )
    );
}

#[test]
fn page_html_with_hyperlink_escaping() {
    let mut t = term(80, 24);

    // URL with special characters that need escaping
    feed(
        &mut t,
        b"\x1b]8;;https://example.com?a=1&b=2\x1b\\link\x1b]8;;\x1b\\",
    );

    assert_eq!(
        html(&t, None),
        format!("{HTML_OPEN}<a href=\"https://example.com?a=1&amp;b=2\">link</a></div>")
    );
}

#[test]
fn page_html_with_styled_hyperlink() {
    let mut t = term(80, 24);

    // Bold hyperlink
    feed(
        &mut t,
        b"\x1b]8;;https://example.com\x1b\\\x1b[1mbold link\x1b[0m\x1b]8;;\x1b\\",
    );

    assert_eq!(
        html(&t, None),
        format!(
            "{HTML_OPEN}<div style=\"display: inline;font-weight: bold;\"><a href=\"https://example.com\">bold link</div></a></div>"
        )
    );
}

#[test]
fn page_html_hyperlink_closes_style_before_anchor() {
    let mut t = term(80, 24);

    // Styled hyperlink followed by plain text
    feed(
        &mut t,
        b"\x1b]8;;https://example.com\x1b\\\x1b[1mbold\x1b[0m plain",
    );

    assert_eq!(
        html(&t, None),
        format!(
            "{HTML_OPEN}<div style=\"display: inline;font-weight: bold;\"><a href=\"https://example.com\">bold</div> plain</a></div>"
        )
    );
}

#[test]
fn page_html_hyperlink_point_map_maps_closing_to_previous_cell() {
    let mut t = term(80, 24);

    feed(
        &mut t,
        b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ normal",
    );

    let mut point_map = Vec::new();
    let output = html(&t, Some(&mut point_map));

    let expected_output =
        format!("{HTML_OPEN}<a href=\"https://example.com\">link</a> normal</div>");
    assert_eq!(output, expected_output);
    assert_eq!(expected_output.len(), point_map.len());

    // The </a> closing tag bytes should all map to the last cell of the link
    let closing_idx = expected_output.find("</a>").unwrap();
    let expected_coord = point_map[closing_idx - 1];
    for i in closing_idx..closing_idx + "</a>".len() {
        assert_eq!(point_map[i], expected_coord);
    }
}
