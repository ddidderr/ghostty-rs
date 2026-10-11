//! "Page plain ..." tests (formatter.zig lines 1580-3287).
use super::*;

struct Out {
    s: String,
    st: TrailingState,
    map: Vec<Coordinate>,
}

/// Runs a PageFormatter over the last page with a point map attached.
fn run(t: &Terminal, opts: &Options, cfg: impl FnOnce(&mut PageFormatter<'_>)) -> Out {
    let page = last_page(t);
    let mut map = Vec::new();
    let mut buf = Vec::new();
    let st = {
        let mut f = PageFormatter::new(page, opts);
        f.point_map = Some(&mut map);
        cfg(&mut f);
        f.format_with_state(&mut buf).unwrap()
    };
    Out {
        s: String::from_utf8(buf).unwrap(),
        st,
        map,
    }
}

fn c(x: u16, y: u32) -> Coordinate {
    Coordinate { x, y }
}

/// `n` consecutive cells on row `y` starting at column `x0`.
fn span(x0: u16, n: u16, y: u32) -> Vec<Coordinate> {
    (0..n).map(|i| c(x0 + i, y)).collect()
}

fn setup(cols: u16, rows: u16, input: &str) -> Terminal {
    let mut t = term(cols, rows);
    feed(&mut t, input.as_bytes());
    // Verify we have only a single page
    let pages = &t.screen().pages;
    assert!(pages.first.is_some());
    assert_eq!(pages.first, pages.last);
    t
}

fn unwrap_opts() -> Options {
    Options {
        unwrap: true,
        ..Options::plain()
    }
}

#[test]
fn page_plain_single_line() {
    let t = setup(80, 24, "hello, world");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |_| {});
    assert_eq!(o.s, "hello, world");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 12));
    assert_eq!(o.map, span(0, 12, 0));
}

#[test]
fn page_plain_single_line_soft_wrapped_unwrapped() {
    let t = setup(3, 5, "hello!");
    let o = run(&t, &unwrap_opts(), |_| {});
    assert_eq!(o.s, "hello!");
    let mut e = span(0, 3, 0);
    e.extend(span(0, 3, 1));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_single_wide_char() {
    let t = setup(80, 24, "1A⚡");
    let page = last_page(&t);
    let opts = Options::plain();

    // Full string
    let o = run(&t, &opts, |_| {});
    assert_eq!(o.s, "1A⚡");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 4));
    assert_eq!(o.map.len(), o.s.len());
    for i in 2..o.s.len() {
        assert_eq!(o.map[i], c(2, 0));
    }

    // Wide only (from start)
    let o = run(&t, &opts, |f| f.start_x = 2);
    assert_eq!(o.s, "⚡");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 4));
    assert_eq!(o.map, vec![c(2, 0); o.s.len()]);

    // Wide only (from tail)
    let o = run(&t, &opts, |f| f.start_x = 3);
    assert_eq!(o.s, "⚡");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 4));
    assert_eq!(o.map, vec![c(2, 0); o.s.len()]);
}

#[test]
fn page_plain_single_wide_char_soft_wrapped_unwrapped() {
    let t = setup(3, 24, "1A⚡");
    let page = last_page(&t);
    let opts = unwrap_opts();

    // Full string
    let o = run(&t, &opts, |_| {});
    assert_eq!(o.s, "1A⚡");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 2));
    assert_eq!(o.map.len(), o.s.len());
    for i in 2..o.s.len() {
        assert_eq!(o.map[i], c(0, 1));
    }

    // Full string (ending on spacer head)
    let o = run(&t, &opts, |f| {
        f.end_x = Some(2);
        f.end_y = Some(0);
    });
    assert_eq!(o.s, "1A⚡");
    assert_eq!(o.map.len(), o.s.len());
    for i in 2..o.s.len() {
        assert_eq!(o.map[i], c(0, 1));
    }

    // Wide only (from start)
    let o = run(&t, &opts, |f| f.start_x = 2);
    assert_eq!(o.s, "⚡");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 2));
    assert_eq!(o.map, vec![c(0, 1); o.s.len()]);

    // Wide only (from tail). In Zig the formatter carried start_x = 2 over
    // from the previous block; start_y = 1, start_x = 1.
    let o = run(&t, &opts, |f| {
        f.start_y = 1;
        f.start_x = 1;
    });
    assert_eq!(o.s, "⚡");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 2));
    assert_eq!(o.map, vec![c(0, 1); o.s.len()]);
}

#[test]
fn page_plain_multiline() {
    let t = setup(80, 24, "hello\r\nworld");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |_| {});
    assert_eq!(o.s, "hello\nworld");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    let mut e = span(0, 5, 0);
    e.push(c(4, 0)); // \n
    e.extend(span(0, 5, 1));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_multiline_rectangle() {
    let t = setup(80, 24, "hello\r\nworld");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| {
        f.start_x = 1;
        f.end_x = Some(3);
        f.rectangle = true;
    });
    assert_eq!(o.s, "ell\norl");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, 0);
    let mut e = span(1, 3, 0);
    e.push(c(3, 0)); // \n
    e.extend(span(1, 3, 1));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_multi_blank_lines() {
    let t = setup(80, 24, "hello\r\n\r\n\r\nworld");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |_| {});
    assert_eq!(o.s, "hello\n\n\nworld");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 3));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    let mut e = span(0, 5, 0);
    e.push(c(4, 0)); // \n after row 0
    e.push(c(0, 1)); // \n after blank row 1
    e.push(c(0, 2)); // \n after blank row 2
    e.extend(span(0, 5, 3));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_trailing_blank_lines() {
    let t = setup(80, 24, "hello\r\nworld\r\n\r\n");
    let page = last_page(&t);
    // We expect there to be no trailing newlines because we can't
    // differentiate trailing blank lines as being meaningful because the
    // page formatter can't see the cursor position.
    let o = run(&t, &Options::plain(), |_| {});
    assert_eq!(o.s, "hello\nworld");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    let mut e = span(0, 5, 0);
    e.push(c(4, 0));
    e.extend(span(0, 5, 1));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_trailing_whitespace() {
    let t = setup(80, 24, "hello   \r\nworld   ");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |_| {});
    assert_eq!(o.s, "hello\nworld");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    let mut e = span(0, 5, 0);
    e.push(c(4, 0));
    e.extend(span(0, 5, 1));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_trailing_whitespace_no_trim() {
    let t = setup(80, 24, "hello   \r\nworld  ");
    let page = last_page(&t);
    let opts = Options {
        trim: false,
        ..Options::plain()
    };
    let o = run(&t, &opts, |_| {});
    assert_eq!(o.s, "hello   \nworld  ");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 7));
    let mut e = span(0, 8, 0);
    e.push(c(7, 0));
    e.extend(span(0, 7, 1));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_with_prior_trailing_state_rows() {
    let t = setup(80, 24, "hello");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| {
        f.trailing_state = Some(TrailingState { rows: 2, cells: 0 });
    });
    assert_eq!(o.s, "\n\nhello");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    let mut e = vec![c(0, 0), c(0, 1)];
    e.extend(span(0, 5, 0));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_with_prior_trailing_state_cells_no_wrapped_line() {
    let t = setup(80, 24, "hello");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| {
        f.trailing_state = Some(TrailingState { rows: 0, cells: 3 });
    });
    // Blank cells are reset when row is not a wrap continuation
    assert_eq!(o.s, "hello");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    assert_eq!(o.map, span(0, 5, 0));
}

#[test]
fn page_plain_with_prior_trailing_state_cells_with_wrap_continuation() {
    let mut t = setup(80, 24, "world");
    // Surgically modify the first row to be a wrap continuation
    {
        let pages = &mut t.screen_mut().pages;
        let id = pages.last.unwrap();
        pages
            .node_mut(id)
            .unwrap()
            .data
            .get_row_mut(0)
            .unwrap()
            .set_wrap_continuation(true);
    }
    let page = last_page(&t);
    let o = run(&t, &unwrap_opts(), |f| {
        f.trailing_state = Some(TrailingState { rows: 0, cells: 3 });
    });
    // Blank cells are preserved when row is a wrap continuation with unwrap enabled
    assert_eq!(o.s, "   world");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    // 3 spaces from prior trailing state (can't go back beyond (0,0)) + "world"
    let mut e = vec![c(0, 0); 3];
    e.extend(span(0, 5, 0));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_soft_wrapped_without_unwrap() {
    let t = setup(10, 24, "hello world test");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |_| {});
    // Without unwrap, wrapped lines show as separate lines
    assert_eq!(o.s, "hello worl\nd test");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 6));
    let mut e = span(0, 10, 0);
    e.push(c(9, 0)); // \n
    e.extend(span(0, 6, 1));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_soft_wrapped_with_unwrap() {
    let t = setup(10, 24, "hello world test");
    let page = last_page(&t);
    let o = run(&t, &unwrap_opts(), |_| {});
    // With unwrap, wrapped lines are joined together
    assert_eq!(o.s, "hello world test");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 6));
    let mut e = span(0, 10, 0);
    e.extend(span(0, 6, 1));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_soft_wrapped_3_lines_without_unwrap() {
    let t = setup(10, 24, "hello world this is a test");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |_| {});
    assert_eq!(o.s, "hello worl\nd this is\na test");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 2));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 6));
    let mut e = span(0, 10, 0);
    e.push(c(9, 0)); // \n
    e.extend(span(0, 9, 1));
    e.push(c(8, 1)); // \n
    e.extend(span(0, 6, 2));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_soft_wrapped_3_lines_with_unwrap() {
    let t = setup(10, 24, "hello world this is a test");
    let page = last_page(&t);
    let o = run(&t, &unwrap_opts(), |_| {});
    assert_eq!(o.s, "hello world this is a test");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 2));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 6));
    let mut e = span(0, 10, 0);
    e.extend(span(0, 10, 1));
    e.extend(span(0, 6, 2));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_start_y_subset() {
    let t = setup(80, 24, "hello\r\nworld\r\ntest");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| f.start_y = 1);
    assert_eq!(o.s, "world\ntest");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 2));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 4));
    let mut e = span(0, 5, 1);
    e.push(c(4, 1)); // \n
    e.extend(span(0, 4, 2));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_end_y_subset() {
    let t = setup(80, 24, "hello\r\nworld\r\ntest");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| f.end_y = Some(1));
    assert_eq!(o.s, "hello\nworld");
    assert_eq!(o.st.rows, 1);
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    let mut e = span(0, 5, 0);
    e.push(c(4, 0)); // \n
    e.extend(span(0, 5, 1));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_start_y_and_end_y_range() {
    let t = setup(80, 24, "hello\r\nworld\r\ntest\r\nfoo");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| {
        f.start_y = 1;
        f.end_y = Some(2);
    });
    assert_eq!(o.s, "world\ntest");
    assert_eq!(o.st.rows, 1);
    assert_eq!(o.st.cells, usize::from(page.size.cols - 4));
    let mut e = span(0, 5, 1);
    e.push(c(4, 1)); // \n
    e.extend(span(0, 4, 2));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_start_y_out_of_bounds() {
    let t = setup(80, 24, "hello");
    let o = run(&t, &Options::plain(), |f| f.start_y = 30);
    assert_eq!(o.s, "");
    assert_eq!(o.st.rows, 0);
    assert_eq!(o.st.cells, 0);
    assert_eq!(o.map.len(), 0);
}

#[test]
fn page_plain_end_y_greater_than_rows() {
    let t = setup(80, 24, "hello");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| f.end_y = Some(30));
    // Should clamp to page.size.rows and work normally
    assert_eq!(o.s, "hello");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    assert_eq!(o.map, span(0, 5, 0));
}

#[test]
fn page_plain_end_y_less_than_start_y() {
    let t = setup(80, 24, "hello");
    let o = run(&t, &Options::plain(), |f| {
        f.start_y = 5;
        f.end_y = Some(2);
    });
    assert_eq!(o.s, "");
    assert_eq!(o.st.rows, 0);
    assert_eq!(o.st.cells, 0);
    assert_eq!(o.map.len(), 0);
}

#[test]
fn page_plain_start_x_on_first_row_only() {
    let t = setup(80, 24, "hello world");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| f.start_x = 6);
    assert_eq!(o.s, "world");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 11));
    assert_eq!(o.map, span(6, 5, 0));
}

#[test]
fn page_plain_end_x_on_last_row_only() {
    let t = setup(80, 24, "first line\r\nsecond line\r\nthird line");
    let o = run(&t, &Options::plain(), |f| {
        f.end_y = Some(2);
        f.end_x = Some(4);
    });
    assert_eq!(o.s, "first line\nsecond line\nthird");
    assert_eq!(o.st.rows, 1);
    assert_eq!(o.st.cells, 0);
    let mut e = span(0, 10, 0);
    e.push(c(9, 0)); // \n
    e.extend(span(0, 11, 1));
    e.push(c(10, 1)); // \n
    e.extend(span(0, 5, 2));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_start_x_and_end_x_multiline() {
    let t = setup(80, 24, "hello world\r\ntest case\r\nfoo bar");
    let o = run(&t, &Options::plain(), |f| {
        f.start_x = 6;
        f.end_y = Some(2);
        f.end_x = Some(2);
    });
    // First row: "world" (start_x=6 to end of row)
    // Second row: "test case" (full row)
    // Third row: "foo" (start to end_x=2, inclusive)
    assert_eq!(o.s, "world\ntest case\nfoo");
    assert_eq!(o.st.rows, 1);
    assert_eq!(o.st.cells, 0);
    let mut e = span(6, 5, 0);
    e.push(c(10, 0)); // \n
    e.extend(span(0, 9, 1));
    e.push(c(8, 1)); // \n
    e.extend(span(0, 3, 2));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_start_x_out_of_bounds() {
    let t = setup(80, 24, "hello");
    let o = run(&t, &Options::plain(), |f| f.start_x = 100);
    assert_eq!(o.s, "");
    assert_eq!(o.st.rows, 0);
    assert_eq!(o.st.cells, 0);
    assert_eq!(o.map.len(), 0);
}

#[test]
fn page_plain_end_x_greater_than_cols() {
    let t = setup(80, 24, "hello");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| f.end_x = Some(100));
    assert_eq!(o.s, "hello");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    assert_eq!(o.map, span(0, 5, 0));
}

#[test]
fn page_plain_end_x_less_than_start_x_single_row() {
    let t = setup(80, 24, "hello");
    let o = run(&t, &Options::plain(), |f| {
        f.start_x = 10;
        f.end_y = Some(0);
        f.end_x = Some(5);
    });
    assert_eq!(o.s, "");
    assert_eq!(o.st.rows, 0);
    assert_eq!(o.st.cells, 0);
    assert_eq!(o.map.len(), 0);
}

#[test]
fn page_plain_start_y_non_zero_ignores_trailing_state() {
    let t = setup(80, 24, "hello\r\nworld");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| {
        f.start_y = 1;
        f.trailing_state = Some(TrailingState { rows: 5, cells: 10 });
    });
    // Should NOT output the 5 newlines from trailing_state because start_y is non-zero
    assert_eq!(o.s, "world");
    assert_eq!(o.st.rows, usize::from(page.size.rows - 1));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    assert_eq!(o.map, span(0, 5, 1));
}

#[test]
fn page_plain_start_x_non_zero_ignores_trailing_state() {
    let t = setup(80, 24, "hello world");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| {
        f.start_x = 6;
        f.trailing_state = Some(TrailingState { rows: 2, cells: 8 });
    });
    // Should NOT output the 2 newlines or 8 spaces from trailing_state
    assert_eq!(o.s, "world");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 11));
    assert_eq!(o.map, span(6, 5, 0));
}

#[test]
fn page_plain_start_y_and_start_x_zero_uses_trailing_state() {
    let t = setup(80, 24, "hello");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |f| {
        f.start_y = 0;
        f.start_x = 0;
        f.trailing_state = Some(TrailingState { rows: 2, cells: 0 });
    });
    // SHOULD output the 2 newlines from trailing_state
    assert_eq!(o.s, "\n\nhello");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 5));
    let mut e = vec![c(0, 0), c(0, 1)];
    e.extend(span(0, 5, 0));
    assert_eq!(o.map, e);
}

#[test]
fn page_plain_single_line_with_styling() {
    let t = setup(80, 24, "hello, \x1b[1mworld\x1b[0m");
    let page = last_page(&t);
    let o = run(&t, &Options::plain(), |_| {});
    assert_eq!(o.s, "hello, world");
    assert_eq!(o.st.rows, usize::from(page.size.rows));
    assert_eq!(o.st.cells, usize::from(page.size.cols - 12));
    assert_eq!(o.map, span(0, 12, 0));
}
