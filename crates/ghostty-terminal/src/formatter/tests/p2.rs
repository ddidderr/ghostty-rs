use super::*;

/// Format the last page of `t` with a page formatter and a point map.
fn page_fmt(t: &Terminal, opts: &Options) -> (String, Vec<Coordinate>) {
    let mut point_map = Vec::new();
    let mut f = PageFormatter::new(last_page(t), opts);
    f.point_map = Some(&mut point_map);
    let out = f.format_string().unwrap();
    (out, point_map)
}

fn trim_start<'a>(s: &'a str, chars: &[char]) -> &'a str {
    s.trim_start_matches(chars)
}

fn coord(x: u16, y: u32) -> Coordinate {
    Coordinate { x, y }
}

#[test]
fn page_vt_single_line_plain_text() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello");
    let (output, point_map) = page_fmt(&t, &Options::vt());
    assert_eq!(output, "hello");
    assert_eq!(point_map.len(), output.len());
    for i in 0..5 {
        assert_eq!(point_map[i], coord(i as u16, 0));
    }
}

#[test]
fn page_vt_single_line_with_bold() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[1mhello\x1b[0m");
    let (output, point_map) = page_fmt(&t, &Options::vt());
    assert_eq!(output, "\x1b[0m\x1b[1mhello\x1b[0m");
    assert_eq!(point_map.len(), output.len());
    for i in 0..8 {
        assert_eq!(point_map[i], coord(0, 0));
    }
    for i in 0..5 {
        assert_eq!(point_map[8 + i], coord(i as u16, 0));
    }
}

#[test]
fn page_vt_multiple_styles() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[1mhello \x1b[3mworld\x1b[0m");
    let (output, point_map) = page_fmt(&t, &Options::vt());
    assert_eq!(
        output,
        "\x1b[0m\x1b[1mhello \x1b[0m\x1b[1m\x1b[3mworld\x1b[0m"
    );
    assert_eq!(point_map.len(), output.len());
}

#[test]
fn page_vt_with_foreground_color() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[31mred\x1b[0m");
    let (output, point_map) = page_fmt(&t, &Options::vt());
    assert_eq!(output, "\x1b[0m\x1b[38;5;1mred\x1b[0m");
    assert_eq!(point_map.len(), output.len());
    for i in 0..13 {
        assert_eq!(point_map[i], coord(0, 0));
    }
    for i in 0..3 {
        assert_eq!(point_map[13 + i], coord(i as u16, 0));
    }
}

#[test]
fn page_vt_with_background_and_foreground_colors() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello");
    let mut opts = Options::vt();
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
    let output = fmt_str(PageFormatter::new(last_page(&t), &opts));
    assert_eq!(
        output,
        "\x1b]10;rgb:ab/cd/ef\x1b\\\x1b]11;rgb:12/34/56\x1b\\hello"
    );
}

#[test]
fn page_vt_multi_line_with_styles() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[1mfirst\x1b[0m\r\n\x1b[3msecond\x1b[0m");
    let (output, point_map) = page_fmt(&t, &Options::vt());
    // Note: style is reset before newline to prevent background colors from
    // bleeding to the next line's leading cells.
    assert_eq!(
        output,
        "\x1b[0m\x1b[1mfirst\x1b[0m\r\n\x1b[0m\x1b[3msecond\x1b[0m"
    );
    assert_eq!(point_map.len(), output.len());
}

#[test]
fn page_vt_duplicate_style_not_emitted_twice() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b[1mhel\x1b[1mlo\x1b[0m");
    let (output, point_map) = page_fmt(&t, &Options::vt());
    assert_eq!(output, "\x1b[0m\x1b[1mhello\x1b[0m");
    assert_eq!(point_map.len(), output.len());
}

fn node_of_first(t: &Terminal) -> crate::page_list::NodeId {
    t.screen().pages.first.unwrap()
}

fn node_of_last(t: &Terminal) -> crate::page_list::NodeId {
    t.screen().pages.last.unwrap()
}

fn list_fmt(t: &Terminal, opts: Options) -> (String, Vec<Pin>) {
    let mut pin_map = Vec::new();
    let mut f = PageListFormatter::new(&t.screen().pages, opts);
    f.pin_map = Some(&mut pin_map);
    let out = f.format_string().unwrap();
    (out, pin_map)
}

#[test]
fn pagelist_plain_single_line() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello, world");
    let (output, pin_map) = list_fmt(&t, Options::plain());
    assert_eq!(output, "hello, world");
    assert_eq!(pin_map.len(), output.len());
    let node = node_of_first(&t);
    for i in 0..output.len() {
        assert_eq!(
            pin_map[i],
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
fn pagelist_plain_spanning_two_pages() {
    let mut t = term(80, 24);
    let first_page_rows = first_page_rows(&t);

    // Fill the first page almost completely
    for _ in 0..first_page_rows - 1 {
        feed(&mut t, b"\r\n");
    }
    feed(&mut t, b"page one");

    // Verify we're still on one page
    assert_eq!(t.screen().pages.first, t.screen().pages.last);

    // Add one more newline to push content to a second page
    feed(&mut t, b"\r\n");
    assert_ne!(t.screen().pages.first, t.screen().pages.last);

    // Write content on the second page
    feed(&mut t, b"page two");

    let (full_output, pin_map) = list_fmt(&t, Options::plain());
    let output = trim_start(&full_output, &['\n']);
    assert_eq!(output, "page one\npage two");

    assert_eq!(pin_map.len(), full_output.len());
    let first_node = node_of_first(&t);
    let last_node = node_of_last(&t);
    let trimmed_count = full_output.len() - output.len();

    // First part (trimmed blank lines) maps to first node
    for p in &pin_map[..trimmed_count] {
        assert_eq!(p.node, first_node);
    }

    // "page one" (8 chars) maps to first node
    for i in 0..8 {
        let idx = trimmed_count + i;
        assert_eq!(pin_map[idx].node, first_node);
        assert_eq!(pin_map[idx].x, i as u16);
    }

    // \n - maps to last node as it represents the transition to new page
    assert_eq!(pin_map[trimmed_count + 8].node, last_node);

    // "page two" (8 chars) maps to last node
    for i in 0..8 {
        let idx = trimmed_count + 9 + i;
        assert_eq!(pin_map[idx].node, last_node);
        assert_eq!(pin_map[idx].x, i as u16);
    }
}

#[test]
fn pagelist_soft_wrapped_line_spanning_two_pages_without_unwrap() {
    let mut t = term(10, 3);
    let first_page_rows = first_page_rows(&t);

    // Fill the first page with soft-wrapped content
    for _ in 0..first_page_rows - 1 {
        feed(&mut t, b"\r\n");
    }
    feed(&mut t, b"hello world test");

    // Verify we're on two pages due to wrapping
    assert_ne!(t.screen().pages.first, t.screen().pages.last);

    let (full_output, pin_map) = list_fmt(&t, Options::plain());
    let output = trim_start(&full_output, &['\n']);
    assert_eq!(output, "hello worl\nd test");

    assert_eq!(pin_map.len(), full_output.len());
    let first_node = node_of_first(&t);
    let last_node = node_of_last(&t);
    let trimmed_count = full_output.len() - output.len();

    for p in &pin_map[..trimmed_count] {
        assert_eq!(p.node, first_node);
    }
    for i in 0..10 {
        assert_eq!(pin_map[trimmed_count + i].node, first_node);
    }

    // \n - maps to last node as it represents the transition to new page
    assert_eq!(pin_map[trimmed_count + 10].node, last_node);

    // "d test" (6 chars) maps to last node
    for i in 0..6 {
        assert_eq!(pin_map[trimmed_count + 11 + i].node, last_node);
    }
}

#[test]
fn pagelist_soft_wrapped_line_spanning_two_pages_with_unwrap() {
    let mut t = term(10, 3);
    let first_page_rows = first_page_rows(&t);

    for _ in 0..first_page_rows - 1 {
        feed(&mut t, b"\r\n");
    }
    feed(&mut t, b"hello world test");

    assert_ne!(t.screen().pages.first, t.screen().pages.last);

    let mut opts = Options::plain();
    opts.unwrap = true;
    let (full_output, pin_map) = list_fmt(&t, opts);
    let output = trim_start(&full_output, &['\r', '\n']);
    assert_eq!(output, "hello world test");

    assert_eq!(pin_map.len(), full_output.len());
    let first_node = node_of_first(&t);
    let last_node = node_of_last(&t);
    let trimmed_count = full_output.len() - output.len();

    for p in &pin_map[..trimmed_count] {
        assert_eq!(p.node, first_node);
    }
    for i in 0..10 {
        assert_eq!(pin_map[trimmed_count + i].node, first_node);
    }
    for i in 0..6 {
        assert_eq!(pin_map[trimmed_count + 10 + i].node, last_node);
    }
}

#[test]
fn pagelist_vt_spanning_two_pages() {
    let mut t = term(80, 24);
    let first_page_rows = first_page_rows(&t);

    // Fill the first page almost completely
    for _ in 0..first_page_rows - 1 {
        feed(&mut t, b"\r\n");
    }
    feed(&mut t, b"\x1b[1mpage one");

    assert_eq!(t.screen().pages.first, t.screen().pages.last);

    feed(&mut t, b"\r\n");
    assert_ne!(t.screen().pages.first, t.screen().pages.last);

    // New content is still styled
    feed(&mut t, b"page two");

    let (full_output, pin_map) = list_fmt(&t, Options::vt());
    let output = trim_start(&full_output, &['\r', '\n']);
    assert_eq!(
        output,
        "\x1b[0m\x1b[1mpage one\x1b[0m\r\n\x1b[0m\x1b[1mpage two\x1b[0m"
    );

    assert_eq!(pin_map.len(), full_output.len());
    let first_node = node_of_first(&t);
    let last_node = node_of_last(&t);

    let first_count = pin_map.iter().filter(|p| p.node == first_node).count();
    let last_count = pin_map.iter().filter(|p| p.node == last_node).count();
    assert!(first_count > 0);
    assert!(last_count > 0);
}

#[test]
fn pagelist_plain_with_x_offset_on_single_page() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello world\r\ntest case\r\nfoo bar");

    let node = node_of_first(&t);
    let mut pin_map = Vec::new();
    let mut f = PageListFormatter::new(&t.screen().pages, Options::plain());
    f.top_left = Some(Pin {
        node,
        y: 0,
        x: 6,
        garbage: false,
    });
    f.bottom_right = Some(Pin {
        node,
        y: 2,
        x: 2,
        garbage: false,
    });
    f.pin_map = Some(&mut pin_map);
    let output = f.format_string().unwrap();
    assert_eq!(output, "world\ntest case\nfoo");

    assert_eq!(pin_map.len(), output.len());
    for p in &pin_map {
        assert_eq!(p.node, node);
    }

    // "world" starts at x=6, y=0
    for i in 0..5 {
        assert_eq!(pin_map[i].x, 6 + i as u16);
        assert_eq!(pin_map[i].y, 0);
    }
}

#[test]
fn pagelist_plain_with_x_offset_spanning_two_pages() {
    let mut t = term(80, 24);
    let first_page_rows = first_page_rows(&t);

    for _ in 0..first_page_rows - 1 {
        feed(&mut t, b"\r\n");
    }
    feed(&mut t, b"hello world");

    assert_eq!(t.screen().pages.first, t.screen().pages.last);

    feed(&mut t, b"\r\n");
    assert_ne!(t.screen().pages.first, t.screen().pages.last);

    feed(&mut t, b"foo bar test");

    let first_node = node_of_first(&t);
    let last_node = node_of_last(&t);
    let first_rows = t.screen().pages.node(first_node).unwrap().data.size.rows;

    let mut pin_map = Vec::new();
    let mut f = PageListFormatter::new(&t.screen().pages, Options::plain());
    f.top_left = Some(Pin {
        node: first_node,
        y: first_rows - 1,
        x: 6,
        garbage: false,
    });
    f.bottom_right = Some(Pin {
        node: last_node,
        y: 1,
        x: 2,
        garbage: false,
    });
    f.pin_map = Some(&mut pin_map);
    let full_output = f.format_string().unwrap();
    let output = trim_start(&full_output, &['\n']);
    assert_eq!(output, "world\nfoo");

    assert_eq!(pin_map.len(), full_output.len());
    let trimmed_count = full_output.len() - output.len();

    // "world" (5 chars) from first page
    for i in 0..5 {
        let idx = trimmed_count + i;
        assert_eq!(pin_map[idx].node, first_node);
        assert_eq!(pin_map[idx].x, 6 + i as u16);
    }

    // \n - maps to last node as it represents the transition to new page
    assert_eq!(pin_map[trimmed_count + 5].node, last_node);

    // "foo" (3 chars) from last page
    for i in 0..3 {
        let idx = trimmed_count + 6 + i;
        assert_eq!(pin_map[idx].node, last_node);
        assert_eq!(pin_map[idx].x, i as u16);
    }
}

#[test]
fn pagelist_plain_with_start_x_only() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello world");

    let node = node_of_first(&t);
    let mut pin_map = Vec::new();
    let mut f = PageListFormatter::new(&t.screen().pages, Options::plain());
    f.top_left = Some(Pin {
        node,
        y: 0,
        x: 6,
        garbage: false,
    });
    f.pin_map = Some(&mut pin_map);
    let output = f.format_string().unwrap();
    assert_eq!(output, "world");

    assert_eq!(pin_map.len(), output.len());
    for i in 0..5 {
        assert_eq!(pin_map[i].node, node);
        assert_eq!(pin_map[i].x, 6 + i as u16);
        assert_eq!(pin_map[i].y, 0);
    }
}

#[test]
fn pagelist_plain_with_end_x_only() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello world\r\ntest");

    let node = node_of_first(&t);
    let mut pin_map = Vec::new();
    let mut f = PageListFormatter::new(&t.screen().pages, Options::plain());
    f.bottom_right = Some(Pin {
        node,
        y: 1,
        x: 2,
        garbage: false,
    });
    f.pin_map = Some(&mut pin_map);
    let output = f.format_string().unwrap();
    assert_eq!(output, "hello world\ntes");

    assert_eq!(pin_map.len(), output.len());

    for i in 0..11 {
        assert_eq!(pin_map[i].node, node);
        assert_eq!(pin_map[i].x, i as u16);
        assert_eq!(pin_map[i].y, 0);
    }

    // \n
    assert_eq!(pin_map[11].node, node);

    for i in 0..3 {
        assert_eq!(pin_map[12 + i].node, node);
        assert_eq!(pin_map[12 + i].x, i as u16);
        assert_eq!(pin_map[12 + i].y, 1);
    }
}

fn rect_format(t: &Terminal, tl: (u16, u32), br: (u16, u32)) -> String {
    let pages = &t.screen().pages;
    let mut f = PageListFormatter::new(pages, Options::plain());
    f.top_left = Some(
        pages
            .pin(Point::Screen(Coordinate { x: tl.0, y: tl.1 }))
            .unwrap(),
    );
    f.bottom_right = Some(
        pages
            .pin(Point::Screen(Coordinate { x: br.0, y: br.1 }))
            .unwrap(),
    );
    f.rectangle = true;
    f.format_string().unwrap()
}

#[test]
fn pagelist_plain_rectangle_basic() {
    let mut t = term(30, 5);
    feed(&mut t, b"Lorem ipsum dolor\r\n");
    feed(&mut t, b"sit amet, consectetur\r\n");
    feed(&mut t, b"adipiscing elit, sed do\r\n");
    feed(&mut t, b"eiusmod tempor incididunt\r\n");
    feed(&mut t, b"ut labore et dolore");

    let output = rect_format(&t, (2, 1), (6, 3));
    assert_eq!(output, "t ame\nipisc\nusmod");
}

#[test]
fn pagelist_plain_rectangle_with_eol() {
    let mut t = term(30, 5);
    feed(&mut t, b"Lorem ipsum dolor\r\n");
    feed(&mut t, b"sit amet, consectetur\r\n");
    feed(&mut t, b"adipiscing elit, sed do\r\n");
    feed(&mut t, b"eiusmod tempor incididunt\r\n");
    feed(&mut t, b"ut labore et dolore");

    let output = rect_format(&t, (12, 0), (26, 4));
    assert_eq!(
        output,
        "dolor\nnsectetur\nlit, sed do\nor incididunt\n dolore"
    );
}

#[test]
fn pagelist_plain_rectangle_more_complex_with_breaks() {
    let mut t = term(30, 8);
    feed(&mut t, b"Lorem ipsum dolor\r\n");
    feed(&mut t, b"sit amet, consectetur\r\n");
    feed(&mut t, b"adipiscing elit, sed do\r\n");
    feed(&mut t, b"eiusmod tempor incididunt\r\n");
    feed(&mut t, b"ut labore et dolore\r\n");
    feed(&mut t, b"\r\n");
    feed(&mut t, b"magna aliqua. Ut enim\r\n");
    feed(&mut t, b"ad minim veniam, quis");

    let output = rect_format(&t, (11, 2), (26, 7));
    assert_eq!(
        output,
        "elit, sed do\npor incididunt\nt dolore\n\na. Ut enim\nniam, quis"
    );
}

#[test]
fn terminal_formatter_plain_no_selection() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello\r\nworld");
    let f = TerminalFormatter::new(&t, Options::plain());
    assert_eq!(f.format_string().unwrap(), "hello\nworld");
}

#[test]
fn terminal_formatter_vt_with_palette() {
    let mut t = term(80, 24);

    // Modify some palette colors using VT sequences
    feed(&mut t, b"\x1b]4;0;rgb:12/34/56\x1b\\");
    feed(&mut t, b"\x1b]4;1;rgb:ab/cd/ef\x1b\\");
    feed(&mut t, b"\x1b]4;255;rgb:ff/00/ff\x1b\\");
    feed(&mut t, b"test");

    let output = TerminalFormatter::new(&t, Options::vt())
        .format_string()
        .unwrap();

    // Create a second terminal and apply the output
    let mut t2 = term(80, 24);
    feed(&mut t2, output.as_bytes());

    // Verify the palettes match
    for i in [0, 1, 255] {
        assert_eq!(t.colors.palette.current[i], t2.colors.palette.current[i]);
    }
}

fn line2_selection(t: &Terminal) -> Selection {
    let pages = &t.screen().pages;
    Selection::new(
        pages.pin(Point::Active(Coordinate { x: 0, y: 1 })).unwrap(),
        pages.pin(Point::Active(Coordinate { x: 4, y: 1 })).unwrap(),
        false,
    )
}

#[test]
fn terminal_formatter_with_selection() {
    let mut t = term(80, 24);
    feed(&mut t, b"line1\r\nline2\r\nline3");
    let sel = line2_selection(&t);
    let mut f = TerminalFormatter::new(&t, Options::plain());
    f.content = Content::Selection(Some(sel));
    assert_eq!(f.format_string().unwrap(), "line2");
}

#[test]
fn terminal_formatter_plain_with_pin_map() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello, world");

    let mut pin_map = Vec::new();
    let mut f = TerminalFormatter::new(&t, Options::plain());
    f.pin_map = Some(&mut pin_map);
    let output = f.format_string().unwrap();
    assert_eq!(output, "hello, world");

    assert_eq!(pin_map.len(), output.len());
    let node = node_of_first(&t);
    for i in 0..output.len() {
        assert_eq!(
            pin_map[i],
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
fn terminal_formatter_plain_multiline_with_pin_map() {
    let mut t = term(80, 24);
    feed(&mut t, b"hello\r\nworld");

    let mut pin_map = Vec::new();
    let mut f = TerminalFormatter::new(&t, Options::plain());
    f.pin_map = Some(&mut pin_map);
    let output = f.format_string().unwrap();
    assert_eq!(output, "hello\nworld");

    assert_eq!(pin_map.len(), output.len());
    let node = node_of_first(&t);
    for i in 0..5 {
        assert_eq!(pin_map[i].node, node);
        assert_eq!(pin_map[i].x, i as u16);
        assert_eq!(pin_map[i].y, 0);
    }
    // "\n" maps to end of first line
    assert_eq!(pin_map[5].node, node);
    for i in 0..5 {
        let idx = 6 + i;
        assert_eq!(pin_map[idx].node, node);
        assert_eq!(pin_map[idx].x, i as u16);
        assert_eq!(pin_map[idx].y, 1);
    }
}

#[test]
fn terminal_formatter_vt_with_palette_and_pin_map() {
    let mut t = term(80, 24);
    feed(&mut t, b"\x1b]4;0;rgb:12/34/56\x1b\\");
    feed(&mut t, b"test");

    let mut pin_map = Vec::new();
    let mut f = TerminalFormatter::new(&t, Options::vt());
    f.pin_map = Some(&mut pin_map);
    let output = f.format_string().unwrap();

    // Palette bytes should be mapped to top left
    assert_eq!(pin_map.len(), output.len());
    let node = node_of_first(&t);
    for p in &pin_map {
        assert_eq!(p.node, node);
    }
}

#[test]
fn terminal_formatter_with_selection_and_pin_map() {
    let mut t = term(80, 24);
    feed(&mut t, b"line1\r\nline2\r\nline3");
    let sel = line2_selection(&t);

    let mut pin_map = Vec::new();
    let mut f = TerminalFormatter::new(&t, Options::plain());
    f.content = Content::Selection(Some(sel));
    f.pin_map = Some(&mut pin_map);
    let output = f.format_string().unwrap();
    assert_eq!(output, "line2");

    assert_eq!(pin_map.len(), output.len());
    let node = node_of_first(&t);
    for i in 0..5 {
        assert_eq!(pin_map[i].node, node);
        assert_eq!(pin_map[i].x, i as u16);
        assert_eq!(pin_map[i].y, 1);
    }
}
