use ghostty_vt::modes::Mode;

use super::*;

/// Format the screen with the given extras, returning output and pin map.
fn screen_vt(t: &Terminal, mutate: impl FnOnce(&mut ScreenFormatter<'_>)) -> (String, Vec<Pin>) {
    let mut pin_map: Vec<Pin> = Vec::new();
    let out = {
        let mut f = ScreenFormatter::new(t.screen(), Options::vt());
        mutate(&mut f);
        f.pin_map = Some(&mut pin_map);
        f.format_string().unwrap()
    };
    (out, pin_map)
}

fn replay(output: &str) -> Terminal {
    let mut t2 = term(80, 24);
    feed(&mut t2, output.as_bytes());
    t2
}

fn first_node(t: &Terminal) -> crate::page_list::NodeId {
    t.screen().pages.first.unwrap()
}

fn page_fmt(t: &Terminal, opts: &Options) -> String {
    PageFormatter::new(last_page(t), opts)
        .format_string()
        .unwrap()
}

const HTML_OPEN: &str = "<div style=\"font-family: monospace; white-space: pre;\">";

#[test]
fn screen_plain_single_line() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello, world");

    let mut pin_map = Vec::new();
    let output = {
        let mut f = ScreenFormatter::new(t.screen(), Options::plain());
        f.pin_map = Some(&mut pin_map);
        f.format_string().unwrap()
    };
    assert_eq!(output, "hello, world");

    assert_eq!(output.len(), pin_map.len());
    let node = first_node(&t);
    for (i, p) in pin_map.iter().enumerate() {
        assert_eq!(
            *p,
            Pin {
                node,
                x: i as u16,
                y: 0,
                garbage: false
            }
        );
    }
}

#[test]
fn screen_plain_multiline() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello\r\nworld");

    let mut pin_map = Vec::new();
    let output = {
        let mut f = ScreenFormatter::new(t.screen(), Options::plain());
        f.pin_map = Some(&mut pin_map);
        f.format_string().unwrap()
    };
    assert_eq!(output, "hello\nworld");

    assert_eq!(output.len(), pin_map.len());
    let node = first_node(&t);
    for i in 0..5 {
        assert_eq!(pin_map[i].node, node);
        assert_eq!(pin_map[i].x, i as u16);
        assert_eq!(pin_map[i].y, 0);
    }
    assert_eq!(pin_map[5].node, node);
    for i in 0..5 {
        let idx = 6 + i;
        assert_eq!(pin_map[idx].node, node);
        assert_eq!(pin_map[idx].x, i as u16);
        assert_eq!(pin_map[idx].y, 1);
    }
}

#[test]
fn screen_plain_with_selection() {
    let mut t = term(80, 24);
    feed(&mut t, b"line1\r\nline2\r\nline3");

    let pages = &t.screen().pages;
    let a = pages.pin(Point::Active(Coordinate { x: 0, y: 1 })).unwrap();
    let b = pages.pin(Point::Active(Coordinate { x: 4, y: 1 })).unwrap();

    let mut pin_map = Vec::new();
    let output = {
        let mut f = ScreenFormatter::new(t.screen(), Options::plain());
        f.content = Content::Selection(Some(Selection::new(a, b, false)));
        f.pin_map = Some(&mut pin_map);
        f.format_string().unwrap()
    };
    assert_eq!(output, "line2");

    assert_eq!(output.len(), pin_map.len());
    let node = first_node(&t);
    for i in 0..5 {
        assert_eq!(pin_map[i].node, node);
        assert_eq!(pin_map[i].x, i as u16);
        assert_eq!(pin_map[i].y, 1);
    }
}

#[test]
fn screen_vt_with_cursor_position() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello\r\nworld");

    let (output, pin_map) = screen_vt(&t, |f| f.extra.cursor = true);
    let t2 = replay(&output);

    assert_eq!(t.screen().cursor.x, t2.screen().cursor.x);
    assert_eq!(t.screen().cursor.y, t2.screen().cursor.y);

    assert_eq!(output.len(), pin_map.len());
    let node = first_node(&t);
    for p in &pin_map {
        assert_eq!(p.node, node);
    }
}

#[test]
fn screen_vt_with_style() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[1;31mhello");

    let (output, pin_map) = screen_vt(&t, |f| f.extra.style = true);
    let t2 = replay(&output);

    assert!(t.screen().cursor.style.eql(t2.screen().cursor.style));

    assert_eq!(output.len(), pin_map.len());
    let node = first_node(&t);
    for p in &pin_map {
        assert_eq!(p.node, node);
    }
}

#[test]
fn screen_vt_with_hyperlink() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b]8;;http://example.com\x1b\\hello");

    let (output, pin_map) = screen_vt(&t, |f| f.extra.hyperlink = true);
    let t2 = replay(&output);

    let l1 = &t.screen().cursor.hyperlink;
    let l2 = &t2.screen().cursor.hyperlink;
    assert_eq!(l1.is_some(), l2.is_some());
    if let (Some(a), Some(b)) = (l1, l2) {
        assert_eq!(a.uri, b.uri);
    }

    assert_eq!(output.len(), pin_map.len());
    let node = first_node(&t);
    for p in &pin_map {
        assert_eq!(p.node, node);
    }
}

#[test]
fn screen_vt_with_protection() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[1\"qhello");

    let (output, pin_map) = screen_vt(&t, |f| f.extra.protection = true);
    let t2 = replay(&output);

    assert_eq!(t.screen().cursor.protected, t2.screen().cursor.protected);

    assert_eq!(output.len(), pin_map.len());
    let node = first_node(&t);
    for p in &pin_map {
        assert_eq!(p.node, node);
    }
}

#[test]
fn screen_vt_with_kitty_keyboard() {
    let mut t = term(80, 24);
    // Set kitty keyboard flags (disambiguate + report_events = 3)
    feed(&mut t, b"\x1b[=3;1uhello");

    let (output, pin_map) = screen_vt(&t, |f| f.extra.kitty_keyboard = true);
    let t2 = replay(&output);

    let flags1 = t.screen().kitty_keyboard.current().int();
    let flags2 = t2.screen().kitty_keyboard.current().int();
    assert_eq!(flags1, flags2);

    assert_eq!(output.len(), pin_map.len());
    let node = first_node(&t);
    for p in &pin_map {
        assert_eq!(p.node, node);
    }
}

#[test]
fn screen_vt_with_charsets() {
    use ghostty_vt::charsets::Slots;
    let mut t = term(80, 24);
    // Set G0 to DEC special and shift to G1
    feed(&mut t, b"\x1b(0\x0ehello");

    let (output, pin_map) = screen_vt(&t, |f| f.extra.charsets = true);
    let t2 = replay(&output);

    let (c1, c2) = (&t.screen().charset, &t2.screen().charset);
    assert_eq!(c1.gl, c2.gl);
    assert_eq!(c1.gr, c2.gr);
    assert_eq!(c1.charsets.get(Slots::G0), c2.charsets.get(Slots::G0));

    assert_eq!(output.len(), pin_map.len());
    let node = first_node(&t);
    for p in &pin_map {
        assert_eq!(p.node, node);
    }
}

fn term_vt(t: &Terminal, mutate: impl FnOnce(&mut TerminalFormatter<'_>)) -> String {
    let mut f = TerminalFormatter::new(t, Options::vt());
    mutate(&mut f);
    f.format_string().unwrap()
}

#[test]
fn terminal_vt_with_scrolling_region() {
    let mut t = term(80, 24);
    // Set scrolling region: top=5, bottom=20
    feed(&mut t, b"\x1b[6;21rhello");

    let output = term_vt(&t, |f| f.extra.scrolling_region = true);
    let t2 = replay(&output);

    assert_eq!(t.scrolling_region.top, t2.scrolling_region.top);
    assert_eq!(t.scrolling_region.bottom, t2.scrolling_region.bottom);
    assert_eq!(t.scrolling_region.left, t2.scrolling_region.left);
    assert_eq!(t.scrolling_region.right, t2.scrolling_region.right);
}

#[test]
fn terminal_vt_with_modes() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[?2004h"); // Bracketed paste
    feed(&mut t, b"\x1b[?1000h"); // Mouse event normal
    feed(&mut t, b"\x1b[?7l"); // Disable wraparound (default is true)
    feed(&mut t, b"hello");

    let output = term_vt(&t, |f| f.extra.modes = true);
    let t2 = replay(&output);

    for m in [
        Mode::BracketedPaste,
        Mode::MouseEventNormal,
        Mode::Wraparound,
    ] {
        assert_eq!(t.modes.get(m), t2.modes.get(m));
    }
}

#[test]
fn terminal_vt_with_tabstops() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[3g"); // Clear all tabs
    feed(&mut t, b"\x1b[5G\x1bH"); // Set tab at column 5
    feed(&mut t, b"\x1b[15G\x1bH"); // Set tab at column 15
    feed(&mut t, b"\x1b[30G\x1bH"); // Set tab at column 30
    feed(&mut t, b"hello");

    let output = term_vt(&t, |f| f.extra.tabstops = true);
    let t2 = replay(&output);

    for c in [4, 14, 29] {
        assert_eq!(t.tabstops.get(c), t2.tabstops.get(c));
        assert!(t2.tabstops.get(c));
    }
    assert!(!t2.tabstops.get(8)); // Not a tab
}

#[test]
fn terminal_vt_with_keyboard_modes() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[>4;2m");
    feed(&mut t, b"hello");

    let output = term_vt(&t, |f| f.extra.keyboard = true);
    let t2 = replay(&output);

    assert_eq!(t.flags.modify_other_keys_2, t2.flags.modify_other_keys_2);
    assert!(t2.flags.modify_other_keys_2);
}

#[test]
fn terminal_vt_with_pwd() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b]7;file://host/home/user\x1b\\hello");

    let output = term_vt(&t, |f| f.extra.pwd = true);
    let t2 = replay(&output);

    assert_eq!(t.pwd, t2.pwd);
}

#[test]
fn page_html_with_multiple_styles() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[1mbold\x1b[3mitalic\x1b[0mnormal");

    let output = page_fmt(&t, &Options::html());
    assert_eq!(
        output,
        "<div style=\"font-family: monospace; white-space: pre;\">\
         <div style=\"display: inline;font-weight: bold;\">bold</div>\
         <div style=\"display: inline;font-weight: bold;font-style: italic;\">italic</div>\
         normal\
         </div>"
    );
}

#[test]
fn page_html_plain_text() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello, world");

    let output = page_fmt(&t, &Options::html());
    assert_eq!(output, format!("{HTML_OPEN}hello, world</div>"));
}

#[test]
fn page_html_with_colors() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[31;44mcolored");

    let output = page_fmt(&t, &Options::html());
    assert_eq!(
        output,
        "<div style=\"font-family: monospace; white-space: pre;\">\
         <div style=\"display: inline;color: var(--vt-palette-1);background-color: var(--vt-palette-4);\">colored</div>\
         </div>"
    );
}

#[test]
fn terminal_formatter_html_with_palette() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b]4;0;rgb:12/34/56\x1b\\");
    feed(&mut t, b"\x1b]4;1;rgb:ab/cd/ef\x1b\\");
    feed(&mut t, b"\x1b]4;255;rgb:ff/00/ff\x1b\\");
    feed(&mut t, b"test");

    let mut f = TerminalFormatter::new(&t, Options::html());
    f.extra.palette = true;
    let output = f.format_string().unwrap();

    assert!(output.contains("<style>:root{"));
    assert!(output.contains("--vt-palette-0: #123456;"));
    assert!(output.contains("--vt-palette-1: #abcdef;"));
    assert!(output.contains("--vt-palette-255: #ff00ff;"));
    assert!(output.contains("}</style>"));
    assert!(output.contains("test"));
}

#[test]
fn page_html_with_background_and_foreground_colors() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello");

    let mut opts = Options::html();
    opts.background = Some(Rgb {
        r: 0x12,
        g: 0x34,
        b: 0x56,
    });
    opts.foreground = Some(Rgb {
        r: 0xab,
        g: 0xcd,
        b: 0xef,
    });
    let output = page_fmt(&t, &opts);
    assert_eq!(
        output,
        "<div style=\"font-family: monospace; white-space: pre;background-color: #123456;color: #abcdef;\">hello</div>"
    );
}

#[test]
fn page_html_with_escaping() {
    let mut t = term(80, 24);
    feed(&mut t, b"<tag>&\"'text");

    let opts = Options::html();
    let mut point_map: Vec<Coordinate> = Vec::new();
    let output = {
        let mut f = PageFormatter::new(last_page(&t), &opts);
        f.point_map = Some(&mut point_map);
        f.format_string().unwrap()
    };
    assert_eq!(
        output,
        format!("{HTML_OPEN}&lt;tag&gt;&amp;&quot;&#39;text</div>")
    );

    assert_eq!(output.len(), point_map.len());

    let offset = HTML_OPEN.len();
    for c in &point_map[..offset] {
        assert_eq!(*c, Coordinate { x: 0, y: 0 });
    }

    let at = |i: usize, x: u16| {
        assert_eq!(point_map[offset + i], Coordinate { x, y: 0 }, "index {i}");
    };
    for i in 0..4 {
        at(i, 0);
    }
    at(4, 1);
    at(5, 2);
    at(6, 3);
    for i in 0..4 {
        at(7 + i, 4);
    }
    for i in 0..5 {
        at(11 + i, 5);
    }
    for i in 0..6 {
        at(16 + i, 6);
    }
    for i in 0..5 {
        at(22 + i, 7);
    }
    at(27, 8);
    at(28, 9);
    at(29, 10);
    at(30, 11);
}

#[test]
fn page_html_with_unicode_as_numeric_entities() {
    let mut t = term(80, 24);
    feed(&mut t, "╰─ ❯".as_bytes());

    let output = page_fmt(&t, &Options::html());
    assert_eq!(output, format!("{HTML_OPEN}&#9584;&#9472; &#10095;</div>"));
}

#[test]
fn page_html_ascii_characters_unchanged() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello world");

    let output = page_fmt(&t, &Options::html());
    assert_eq!(output, format!("{HTML_OPEN}hello world</div>"));
}

#[test]
fn page_html_mixed_ascii_and_unicode() {
    let mut t = term(80, 24);
    feed(&mut t, "test ╰─❯ ok".as_bytes());

    let output = page_fmt(&t, &Options::html());
    assert_eq!(
        output,
        format!("{HTML_OPEN}test &#9584;&#9472;&#10095; ok</div>")
    );
}

#[test]
fn page_vt_with_palette_option_emits_rgb() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b]4;1;rgb:ab/cd/ef\x1b\\");
    feed(&mut t, b"\x1b[31mred");

    // Without palette option - should emit palette index
    let output = page_fmt(&t, &Options::vt());
    assert_eq!(output, "\x1b[0m\x1b[38;5;1mred\x1b[0m");

    // With palette option - should emit RGB directly
    let mut opts = Options::vt();
    opts.palette = Some(t.colors.palette.current);
    let output = page_fmt(&t, &opts);
    assert_eq!(output, "\x1b[0m\x1b[38;2;171;205;239mred\x1b[0m");
}

#[test]
fn page_html_with_palette_option_emits_rgb() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b]4;1;rgb:ab/cd/ef\x1b\\");
    feed(&mut t, b"\x1b[31mred");

    let output = page_fmt(&t, &Options::html());
    assert_eq!(
        output,
        "<div style=\"font-family: monospace; white-space: pre;\">\
         <div style=\"display: inline;color: var(--vt-palette-1);\">red</div>\
         </div>"
    );

    let mut opts = Options::html();
    opts.palette = Some(t.colors.palette.current);
    let output = page_fmt(&t, &opts);
    assert_eq!(
        output,
        "<div style=\"font-family: monospace; white-space: pre;\">\
         <div style=\"display: inline;color: rgb(171, 205, 239);\">red</div>\
         </div>"
    );
}

#[test]
fn page_vt_style_reset_properly_closes_styles() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[1mbold\x1b[0mnormal");

    let output = page_fmt(&t, &Options::vt());
    assert_eq!(output, "\x1b[0m\x1b[1mbold\x1b[0mnormal");
}
