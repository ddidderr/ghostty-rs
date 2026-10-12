use super::*;
use crate::{
    page::Wide,
    screen::{PromptKind, SemanticContentSet},
};

fn cell_at(s: &Screen, pt: Point) -> Cell {
    s.pages.get_cell(pt).unwrap()
}

fn row_at(s: &Screen, pt: Point) -> Row {
    *pin(s, pt).row_and_cell(&s.pages).unwrap().0
}

fn resize(s: &mut Screen, cols: u16, rows: u16) {
    s.resize(Resize::new(cols, rows)).unwrap();
}

fn resize_no_reflow(s: &mut Screen, cols: u16, rows: u16) {
    s.resize(Resize {
        reflow: false,
        ..Resize::new(cols, rows)
    })
    .unwrap();
}

fn cursor_cell_cp(s: &Screen) -> u32 {
    cell_at(s, active(s.cursor.x, u32::from(s.cursor.y))).codepoint()
}

#[test]
fn screen_clone_empty_viewport() {
    let s = init(10, 3, 0);

    {
        let s2 = s
            .clone_screen(viewport(0, 0), Some(viewport(0, 0)))
            .unwrap();

        // Test our contents rotated
        assert_eq!("", dump(&s2, viewport(0, 0)));
    }
}

#[test]
fn screen_clone_one_line_viewport() {
    let mut s = init(10, 3, 0);
    s.test_write_string("1ABC").unwrap();

    {
        let s2 = s
            .clone_screen(viewport(0, 0), Some(viewport(0, 0)))
            .unwrap();

        // Test our contents
        assert_eq!("1ABC", dump(&s2, viewport(0, 0)));
    }
}

#[test]
fn screen_clone_empty_active() {
    let s = init(10, 3, 0);

    {
        let s2 = s.clone_screen(active(0, 0), Some(active(0, 0))).unwrap();

        // Test our contents rotated
        assert_eq!("", dump(&s2, active(0, 0)));
    }
}

#[test]
fn screen_clone_one_line_active_with_extra_space() {
    let mut s = init(10, 3, 0);
    s.test_write_string("1ABC").unwrap();

    {
        let s2 = s.clone_screen(active(0, 0), None).unwrap();

        // Test our contents rotated
        assert_eq!("1ABC", dump(&s2, active(0, 0)));
    }
}

#[test]
fn screen_clear_history_with_no_history() {
    let mut s = init(10, 3, 3);
    s.test_write_string("4ABCD\n5EFGH\n6IJKL").unwrap();
    assert!(s.pages.viewport == Viewport::Active);
    s.erase_rows(history(0, 0), None);
    assert!(s.pages.viewport == Viewport::Active);
    {
        // Test our contents rotated
        assert_eq!("4ABCD\n5EFGH\n6IJKL", dump(&s, viewport(0, 0)));
    }
    {
        // Test our contents rotated
        assert_eq!("4ABCD\n5EFGH\n6IJKL", dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_clear_history() {
    let mut s = init(10, 3, 3);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL\n4ABCD\n5EFGH\n6IJKL")
        .unwrap();
    assert!(s.pages.viewport == Viewport::Active);

    // Scroll to top
    s.scroll(Scroll::Top);
    {
        // Test our contents rotated
        assert_eq!("1ABCD\n2EFGH\n3IJKL", dump(&s, viewport(0, 0)));
    }

    s.erase_rows(history(0, 0), None);
    assert!(s.pages.viewport == Viewport::Active);
    {
        // Test our contents rotated
        assert_eq!("4ABCD\n5EFGH\n6IJKL", dump(&s, viewport(0, 0)));
    }
    {
        // Test our contents rotated
        assert_eq!("4ABCD\n5EFGH\n6IJKL", dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_clear_above_cursor() {
    let mut s = init(10, 10, 3);
    s.test_write_string("4ABCD\n5EFGH\n6IJKL").unwrap();
    s.clear_rows(
        active(0, 0),
        Some(active(0, u32::from(s.cursor.y) - 1)),
        false,
    );
    {
        assert_eq!("\n\n6IJKL", dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!("\n\n6IJKL", dump(&s, screen_pt(0, 0)));
    }

    assert_eq!(5, s.cursor.x);
    assert_eq!(2, s.cursor.y);
}

#[test]
fn screen_clear_above_cursor_with_history() {
    let mut s = init(10, 3, 3);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL\n").unwrap();
    s.test_write_string("4ABCD\n5EFGH\n6IJKL").unwrap();
    s.clear_rows(
        active(0, 0),
        Some(active(0, u32::from(s.cursor.y) - 1)),
        false,
    );
    {
        assert_eq!("\n\n6IJKL", dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!("1ABCD\n2EFGH\n3IJKL\n\n\n6IJKL", dump(&s, screen_pt(0, 0)));
    }

    assert_eq!(5, s.cursor.x);
    assert_eq!(2, s.cursor.y);
}

#[test]
fn screen_resize_no_reflow_more_rows() {
    let mut s = init(10, 3, 0);
    let str = "1ABCD\n2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();

    // Resize
    resize_no_reflow(&mut s, 10, 10);
    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_no_reflow_less_rows() {
    let mut s = init(10, 3, 0);
    let str = "1ABCD\n2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();
    assert_eq!(5, s.cursor.x);
    assert_eq!(2, s.cursor.y);
    resize_no_reflow(&mut s, 10, 2);

    // Since we shrunk, we should adjust our cursor
    assert_eq!(5, s.cursor.x);
    assert_eq!(1, s.cursor.y);

    {
        assert_eq!("2EFGH\n3IJKL", dump(&s, viewport(0, 0)));
    }
}

fn bg_fill_rows(s: &mut Screen) {
    // Write only a background color into the remaining rows
    for y in 1..s.pages.rows {
        let c = s.pages.get_cell_mut(active(0, u32::from(y))).unwrap();
        let mut cell = Cell::default();
        cell.set_content_tag(ContentTag::BgColorRgb);
        cell.set_color_rgb(ghostty_vt::color::Rgb {
            r: 0xFF,
            g: 0,
            b: 0,
        });
        *c = cell;
    }
}

#[test]
fn screen_resize_no_reflow_less_rows_trims_blank_lines() {
    let mut s = init(10, 3, 0);
    let str = "1ABCD";
    s.test_write_string(str).unwrap();

    bg_fill_rows(&mut s);

    let (cx, cy) = (s.cursor.x, s.cursor.y);
    resize_no_reflow(&mut s, 6, 2);

    // Cursor should not move
    assert_eq!(cx, s.cursor.x);
    assert_eq!(cy, s.cursor.y);

    {
        assert_eq!("1ABCD", dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_no_reflow_more_rows_trims_blank_lines() {
    let mut s = init(10, 3, 0);
    let str = "1ABCD";
    s.test_write_string(str).unwrap();

    bg_fill_rows(&mut s);

    let (cx, cy) = (s.cursor.x, s.cursor.y);
    resize_no_reflow(&mut s, 10, 7);

    // Cursor should not move
    assert_eq!(cx, s.cursor.x);
    assert_eq!(cy, s.cursor.y);

    {
        assert_eq!("1ABCD", dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_no_reflow_more_cols() {
    let mut s = init(10, 3, 0);
    let str = "1ABCD\n2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();
    resize_no_reflow(&mut s, 20, 3);

    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_no_reflow_less_cols() {
    let mut s = init(10, 3, 0);
    let str = "1ABCD\n2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();
    resize_no_reflow(&mut s, 4, 3);

    {
        let expected = "1ABC\n2EFG\n3IJK";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_no_reflow_more_rows_with_scrollback_cursor_end() {
    let mut s = init(7, 3, 2);
    let str = "1ABCD\n2EFGH\n3IJKL\n4ABCD\n5EFGH";
    s.test_write_string(str).unwrap();
    resize_no_reflow(&mut s, 7, 10);

    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_no_reflow_less_rows_with_scrollback() {
    let mut s = init(7, 3, 2);
    let str = "1ABCD\n2EFGH\n3IJKL\n4ABCD\n5EFGH";
    s.test_write_string(str).unwrap();
    resize_no_reflow(&mut s, 7, 2);

    {
        let expected = "4ABCD\n5EFGH";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }
}

// https://github.com/mitchellh/ghostty/issues/1030
#[test]
fn screen_resize_no_reflow_less_rows_with_empty_trailing() {
    let mut s = init(5, 3, 5);
    let str = "1\n2\n3\n4\n5\n6\n7\n8";
    s.test_write_string(str).unwrap();
    s.scroll_clear().unwrap();
    s.cursor_absolute(0, 0);
    s.test_write_string("A\nB").unwrap();

    let (cx, cy) = (s.cursor.x, s.cursor.y);
    resize_no_reflow(&mut s, 5, 2);
    assert_eq!(cx, s.cursor.x);
    assert_eq!(cy, s.cursor.y);

    {
        assert_eq!("A\nB", dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_no_reflow_more_rows_with_soft_wrapping() {
    let mut s = init(2, 3, 3);
    let str = "1A2B\n3C4E\n5F6G";
    s.test_write_string(str).unwrap();

    // Every second row should be wrapped
    for y in 0..6u32 {
        let row = row_at(&s, screen_pt(0, y));
        let wrapped = y % 2 == 0;
        assert_eq!(wrapped, row.wrap());
    }

    // Resize
    resize_no_reflow(&mut s, 2, 10);
    {
        let expected = "1A\n2B\n3C\n4E\n5F\n6G";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Every second row should be wrapped
    for y in 0..6u32 {
        let row = row_at(&s, screen_pt(0, y));
        let wrapped = y % 2 == 0;
        assert_eq!(wrapped, row.wrap());
    }
}

#[test]
fn screen_resize_more_rows_no_scrollback() {
    let mut s = init(5, 3, 0);
    let str = "1ABCD\n2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();
    let (cx, cy) = (s.cursor.x, s.cursor.y);
    resize(&mut s, 5, 10);

    // Cursor should not move
    assert_eq!(cx, s.cursor.x);
    assert_eq!(cy, s.cursor.y);

    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_resize_more_rows_with_empty_scrollback() {
    let mut s = init(5, 3, 10);
    let str = "1ABCD\n2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();
    let (cx, cy) = (s.cursor.x, s.cursor.y);
    resize(&mut s, 5, 10);

    // Cursor should not move
    assert_eq!(cx, s.cursor.x);
    assert_eq!(cy, s.cursor.y);

    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_resize_more_rows_with_populated_scrollback() {
    let mut s = init(5, 3, 5);
    let str = "1ABCD\n2EFGH\n3IJKL\n4ABCD\n5EFGH";
    s.test_write_string(str).unwrap();
    {
        let expected = "3IJKL\n4ABCD\n5EFGH";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Set our cursor to be on the "4"
    s.cursor_absolute(0, 1);
    {
        assert_eq!(u32::from('4'), cursor_cell_cp(&s));
    }

    // Resize
    resize(&mut s, 5, 10);

    // Cursor should still be on the "4"
    {
        assert_eq!(u32::from('4'), cursor_cell_cp(&s));
    }

    {
        let expected = "3IJKL\n4ABCD\n5EFGH";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_more_cols_no_reflow() {
    let mut s = init(5, 3, 0);
    let str = "1ABCD\n2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();

    let (cx, cy) = (s.cursor.x, s.cursor.y);
    resize(&mut s, 10, 3);

    // Cursor should not move
    assert_eq!(cx, s.cursor.x);
    assert_eq!(cy, s.cursor.y);

    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
}

// https://github.com/mitchellh/ghostty/issues/272#issuecomment-1676038963
#[test]
fn screen_resize_more_cols_perfect_split() {
    let mut s = init(5, 3, 0);
    let str = "1ABCD2EFGH3IJKL";
    s.test_write_string(str).unwrap();
    resize(&mut s, 10, 3);

    {
        assert_eq!("1ABCD2EFGH\n3IJKL", dump(&s, screen_pt(0, 0)));
    }
}

// https://github.com/mitchellh/ghostty/issues/1159
#[test]
fn screen_resize_no_reflow_more_cols_with_scrollback_scrolled_up() {
    let mut s = init(5, 3, 5);
    let str = "1\n2\n3\n4\n5\n6\n7\n8";
    s.test_write_string(str).unwrap();

    // Cursor at bottom
    assert_eq!(1, s.cursor.x);
    assert_eq!(2, s.cursor.y);

    s.scroll(Scroll::DeltaRow(-4));
    {
        assert_eq!("2\n3\n4", dump(&s, viewport(0, 0)));
    }

    resize(&mut s, 8, 3);
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }

    // Cursor remains at bottom
    assert_eq!(1, s.cursor.x);
    assert_eq!(2, s.cursor.y);
}

// https://github.com/mitchellh/ghostty/issues/1159
#[test]
fn screen_resize_no_reflow_less_cols_with_scrollback_scrolled_up() {
    let mut s = init(5, 3, 5);
    let str = "1\n2\n3\n4\n5\n6\n7\n8";
    s.test_write_string(str).unwrap();

    // Cursor at bottom
    assert_eq!(1, s.cursor.x);
    assert_eq!(2, s.cursor.y);

    s.scroll(Scroll::DeltaRow(-4));
    {
        assert_eq!("2\n3\n4", dump(&s, viewport(0, 0)));
    }

    resize(&mut s, 4, 3);
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
    {
        assert_eq!("6\n7\n8", dump(&s, active(0, 0)));
    }

    // Cursor remains at bottom
    assert_eq!(1, s.cursor.x);
    assert_eq!(2, s.cursor.y);

    // Old implementation doesn't do this but it makes sense to me:
    // {
    //     assert_eq!("2\n3\n4", dump(&s, viewport(0, 0)));
    // }
}

#[test]
fn screen_resize_more_cols_no_reflow_preserves_semantic_prompt() {
    let mut s = init(5, 3, 0);

    // Set one of the rows to be a prompt
    s.cursor_set_semantic_content(SemanticContentSet::Output);
    s.test_write_string("1ABCD\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("2EFGH").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Output);
    s.test_write_string("\n3IJKL").unwrap();

    s.resize(Resize {
        reflow: false,
        ..Resize::new(10, 3)
    })
    .unwrap();

    let expected = "1ABCD\n2EFGH\n3IJKL";
    {
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!(expected, dump(&s, screen_pt(0, 0)));
    }

    // Our one row should still be a semantic prompt, the others should not.
    assert!(row_at(&s, active(0, 0)).semantic_prompt() == RowPrompt::None);
    assert!(row_at(&s, active(0, 1)).semantic_prompt() == RowPrompt::Prompt);
    assert!(row_at(&s, active(0, 2)).semantic_prompt() == RowPrompt::None);
}

#[test]
fn screen_resize_more_cols_with_reflow_that_fits_full_width() {
    let mut s = init(5, 3, 0);
    let str = "1ABCD2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();

    // Verify we soft wrapped
    {
        let expected = "1ABCD\n2EFGH\n3IJKL";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Let's put our cursor on row 2, where the soft wrap is
    s.cursor_absolute(0, 1);
    {
        assert_eq!(u32::from('2'), cursor_cell_cp(&s));
    }

    // Resize and verify we undid the soft wrap because we have space now
    resize(&mut s, 10, 3);
    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }

    // Our cursor should've moved
    assert_eq!(5, s.cursor.x);
    assert_eq!(0, s.cursor.y);
}

#[test]
fn screen_resize_more_cols_with_reflow_that_ends_in_newline() {
    let mut s = init(6, 3, 0);
    let str = "1ABCD2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();

    // Verify we soft wrapped
    {
        let expected = "1ABCD2\nEFGH\n3IJKL";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Let's put our cursor on the last row
    s.cursor_absolute(0, 2);
    {
        assert_eq!(u32::from('3'), cursor_cell_cp(&s));
    }

    // Resize and verify we undid the soft wrap because we have space now
    resize(&mut s, 10, 3);
    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }

    // Our cursor should still be on the 3
    {
        assert_eq!(u32::from('3'), cursor_cell_cp(&s));
    }
}

#[test]
fn screen_resize_more_cols_with_reflow_that_forces_more_wrapping() {
    let mut s = init(5, 3, 0);
    let str = "1ABCD2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();

    // Let's put our cursor on row 2, where the soft wrap is
    s.cursor_absolute(0, 1);
    {
        assert_eq!(u32::from('2'), cursor_cell_cp(&s));
    }

    // Verify we soft wrapped
    {
        let expected = "1ABCD\n2EFGH\n3IJKL";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Resize and verify we undid the soft wrap because we have space now
    resize(&mut s, 7, 3);
    {
        let expected = "1ABCD2E\nFGH\n3IJKL";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Our cursor should've moved
    assert_eq!(5, s.cursor.x);
    assert_eq!(0, s.cursor.y);
}

#[test]
fn screen_resize_more_cols_with_reflow_that_unwraps_multiple_times() {
    let mut s = init(5, 3, 0);
    let str = "1ABCD2EFGH3IJKL";
    s.test_write_string(str).unwrap();

    // Let's put our cursor on row 2, where the soft wrap is
    s.cursor_absolute(0, 2);
    {
        assert_eq!(u32::from('3'), cursor_cell_cp(&s));
    }

    // Verify we soft wrapped
    {
        let expected = "1ABCD\n2EFGH\n3IJKL";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Resize and verify we undid the soft wrap because we have space now
    resize(&mut s, 15, 3);
    {
        let expected = "1ABCD2EFGH3IJKL";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Our cursor should've moved
    assert_eq!(10, s.cursor.x);
    assert_eq!(0, s.cursor.y);
}

#[test]
fn screen_resize_more_cols_with_populated_scrollback() {
    let mut s = init(5, 3, 5);
    let str = "1ABCD\n2EFGH\n3IJKL\n4ABCD5EFGH";
    s.test_write_string(str).unwrap();
    {
        let expected = "3IJKL\n4ABCD\n5EFGH";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // // Set our cursor to be on the "5"
    s.cursor_absolute(0, 2);
    {
        assert_eq!(u32::from('5'), cursor_cell_cp(&s));
    }

    // Resize
    resize(&mut s, 10, 3);
    {
        let expected = "2EFGH\n3IJKL\n4ABCD5EFGH";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Cursor should still be on the "5"
    {
        assert_eq!(u32::from('5'), cursor_cell_cp(&s));
    }
}

#[test]
fn screen_resize_more_cols_with_reflow() {
    let mut s = init(2, 3, 5);
    let str = "1ABC\n2DEF\n3ABC\n4DEF";
    s.test_write_string(str).unwrap();

    // Let's put our cursor on row 2, where the soft wrap is
    s.cursor_absolute(0, 2);
    {
        assert_eq!(u32::from('E'), cursor_cell_cp(&s));
    }

    // Verify we soft wrapped
    {
        let expected = "BC\n4D\nEF";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Resize and verify we undid the soft wrap because we have space now
    resize(&mut s, 7, 3);
    {
        let expected = "1ABC\n2DEF\n3ABC\n4DEF";
        assert_eq!(expected, dump(&s, screen_pt(0, 0)));
    }

    // Our cursor should've moved
    assert_eq!(2, s.cursor.x);
    assert_eq!(2, s.cursor.y);
}

#[test]
fn screen_resize_more_rows_and_cols_with_wrapping() {
    let mut s = init(2, 4, 0);
    let str = "1A2B\n3C4D";
    s.test_write_string(str).unwrap();
    {
        let expected = "1A\n2B\n3C\n4D";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    resize(&mut s, 5, 10);

    // Cursor should move due to wrapping
    assert_eq!(3, s.cursor.x);
    assert_eq!(1, s.cursor.y);

    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_resize_less_rows_no_scrollback() {
    let mut s = init(5, 3, 0);
    let str = "1ABCD\n2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();

    s.cursor_absolute(0, 0);
    let (cx, cy) = (s.cursor.x, s.cursor.y);
    resize(&mut s, 5, 1);

    // Cursor should not move
    assert_eq!(cx, s.cursor.x);
    assert_eq!(cy, s.cursor.y);

    {
        assert_eq!("3IJKL", dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!("3IJKL", dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_resize_less_rows_moving_cursor() {
    let mut s = init(5, 3, 0);
    let str = "1ABCD\n2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();

    // Put our cursor on the last line
    s.cursor_absolute(1, 2);
    {
        assert_eq!(u32::from('I'), cursor_cell_cp(&s));
    }

    // Resize
    resize(&mut s, 5, 1);

    {
        assert_eq!("3IJKL", dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!("3IJKL", dump(&s, screen_pt(0, 0)));
    }

    // Cursor should be on the last line
    assert_eq!(1, s.cursor.x);
    assert_eq!(0, s.cursor.y);
}

#[test]
fn screen_resize_less_rows_with_empty_scrollback() {
    let mut s = init(5, 3, 10);
    let str = "1ABCD\n2EFGH\n3IJKL";
    s.test_write_string(str).unwrap();
    resize(&mut s, 5, 1);

    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
    {
        assert_eq!("3IJKL", dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_less_rows_with_populated_scrollback() {
    let mut s = init(5, 3, 5);
    let str = "1ABCD\n2EFGH\n3IJKL\n4ABCD\n5EFGH";
    s.test_write_string(str).unwrap();
    {
        let expected = "3IJKL\n4ABCD\n5EFGH";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Resize
    resize(&mut s, 5, 1);

    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
    {
        assert_eq!("5EFGH", dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_less_rows_with_full_scrollback() {
    let mut s = init(5, 3, 3);
    let str = "00000\n1ABCD\n2EFGH\n3IJKL\n4ABCD\n5EFGH";
    s.test_write_string(str).unwrap();
    {
        let expected = "3IJKL\n4ABCD\n5EFGH";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    assert_eq!(4, s.cursor.x);
    assert_eq!(2, s.cursor.y);

    // Resize
    resize(&mut s, 5, 2);

    // Cursor should stay in the same relative place (bottom of the
    // screen, same character).
    assert_eq!(4, s.cursor.x);
    assert_eq!(1, s.cursor.y);

    {
        let expected = "00000\n1ABCD\n2EFGH\n3IJKL\n4ABCD\n5EFGH";
        assert_eq!(expected, dump(&s, screen_pt(0, 0)));
    }
    {
        let expected = "4ABCD\n5EFGH";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }
}

#[test]
fn screen_resize_less_cols_no_reflow() {
    let mut s = init(5, 3, 0);
    let str = "1AB\n2EF\n3IJ";
    s.test_write_string(str).unwrap();

    s.cursor_absolute(0, 0);
    let (cx, cy) = (s.cursor.x, s.cursor.y);
    resize(&mut s, 3, 3);

    // Cursor should not move
    assert_eq!(cx, s.cursor.x);
    assert_eq!(cy, s.cursor.y);

    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_resize_less_cols_with_reflow_but_row_space() {
    let mut s = init(5, 3, 1);
    let str = "1ABCD";
    s.test_write_string(str).unwrap();

    // Put our cursor on the end
    s.cursor_absolute(4, 0);
    {
        assert_eq!(u32::from('D'), cursor_cell_cp(&s));
    }

    resize(&mut s, 3, 3);
    {
        assert_eq!("1AB\nCD", dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!("1AB\nCD", dump(&s, screen_pt(0, 0)));
    }

    // Cursor should be on the last line
    assert_eq!(1, s.cursor.x);
    assert_eq!(1, s.cursor.y);
}

#[test]
fn screen_resize_less_cols_with_reflow_with_trimmed_rows() {
    let mut s = init(5, 3, 0);
    let str = "3IJKL\n4ABCD\n5EFGH";
    s.test_write_string(str).unwrap();
    resize(&mut s, 3, 3);

    {
        assert_eq!("CD\n5EF\nGH", dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!("CD\n5EF\nGH", dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_resize_less_cols_with_reflow_with_trimmed_rows_and_scrollback() {
    let mut s = init(5, 3, 1);
    let str = "3IJKL\n4ABCD\n5EFGH";
    s.test_write_string(str).unwrap();
    resize(&mut s, 3, 3);

    {
        assert_eq!("CD\n5EF\nGH", dump(&s, viewport(0, 0)));
    }
    {
        let expected = "3IJ\nKL\n4AB\nCD\n5EF\nGH";
        assert_eq!(expected, dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_resize_less_cols_with_reflow_previously_wrapped() {
    let mut s = init(5, 3, 0);
    let str = "3IJKL4ABCD5EFGH";
    s.test_write_string(str).unwrap();

    // Check
    {
        let expected = "3IJKL\n4ABCD\n5EFGH";
        assert_eq!(expected, dump(&s, screen_pt(0, 0)));
    }

    resize(&mut s, 3, 3);

    // {
    //     const contents = try s.testString(alloc, .viewport);
    //     const expected = "CD\n5EF\nGH";
    // }
    {
        let expected = "ABC\nD5E\nFGH";
        assert_eq!(expected, dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_resize_less_cols_with_reflow_and_scrollback() {
    let mut s = init(5, 3, 5);
    let str = "1A\n2B\n3C\n4D\n5E";
    s.test_write_string(str).unwrap();

    // Put our cursor on the end
    s.cursor_absolute(1, s.pages.rows - 1);
    {
        assert_eq!(u32::from('E'), cursor_cell_cp(&s));
    }

    resize(&mut s, 3, 3);

    {
        let expected = "3C\n4D\n5E";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Cursor should be on the last line
    assert_eq!(1, s.cursor.x);
    assert_eq!(2, s.cursor.y);
}

#[test]
fn screen_resize_less_cols_with_reflow_previously_wrapped_and_scrollback() {
    let mut s = init(5, 3, 2);
    let str = "1ABCD2EFGH3IJKL4ABCD5EFGH";
    s.test_write_string(str).unwrap();

    // Check
    {
        let expected = "3IJKL\n4ABCD\n5EFGH";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    // Put our cursor on the end
    s.cursor_absolute(s.pages.cols - 1, s.pages.rows - 1);
    {
        assert_eq!(u32::from('H'), cursor_cell_cp(&s));
    }

    resize(&mut s, 3, 3);

    {
        let expected = "CD5\nEFG\nH";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }
    {
        let expected = "1AB\nCD2\nEFG\nH3I\nJKL\n4AB\nCD5\nEFG\nH";
        assert_eq!(expected, dump(&s, screen_pt(0, 0)));
    }

    // Cursor should be on the last line
    assert_eq!(0, s.cursor.x);
    assert_eq!(2, s.cursor.y);
    {
        assert_eq!(u32::from('H'), cursor_cell_cp(&s));
    }
}

#[test]
fn screen_resize_less_cols_with_scrollback_keeps_cursor_row() {
    let mut s = init(5, 3, 5);
    let str = "1A\n2B\n3C\n4D\n5E";
    s.test_write_string(str).unwrap();

    // Lets do a scroll and clear operation
    s.scroll_clear().unwrap();

    // Move our cursor to the beginning
    s.cursor_absolute(0, 0);

    resize(&mut s, 3, 3);

    {
        assert_eq!("", dump(&s, viewport(0, 0)));
    }

    // Cursor should be on the last line
    assert_eq!(0, s.cursor.x);
    assert_eq!(0, s.cursor.y);
}

#[test]
fn screen_resize_more_rows_less_cols_with_reflow_with_scrollback() {
    let mut s = init(5, 3, 3);
    let str = "1ABCD\n2EFGH3IJKL\n4MNOP";
    s.test_write_string(str).unwrap();

    {
        let expected = "1ABCD\n2EFGH\n3IJKL\n4MNOP";
        assert_eq!(expected, dump(&s, screen_pt(0, 0)));
    }
    {
        let expected = "2EFGH\n3IJKL\n4MNOP";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }

    resize(&mut s, 2, 10);

    {
        let expected = "BC\nD\n2E\nFG\nH3\nIJ\nKL\n4M\nNO\nP";
        assert_eq!(expected, dump(&s, viewport(0, 0)));
    }
    {
        let expected = "1A\nBC\nD\n2E\nFG\nH3\nIJ\nKL\n4M\nNO\nP";
        assert_eq!(expected, dump(&s, screen_pt(0, 0)));
    }
}

// This seems like it should work fine but for some reason in practice
// in the initial implementation I found this bug! This is a regression
// test for that.
#[test]
fn screen_resize_more_rows_then_shrink_again() {
    let mut s = init(5, 3, 10);
    let str = "1ABC";
    s.test_write_string(str).unwrap();

    // Grow
    resize(&mut s, 5, 10);
    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }

    // Shrink
    resize(&mut s, 5, 3);
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }

    // Grow again
    resize(&mut s, 5, 10);
    {
        assert_eq!(str, dump(&s, viewport(0, 0)));
    }
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_resize_less_cols_to_eliminate_wide_char() {
    let mut s = init(2, 1, 0);
    let str = "😀";
    s.test_write_string(str).unwrap();
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
    {
        let cell = cell_at(&s, screen_pt(0, 0));
        assert_eq!(Wide::Wide, cell.wide());
        assert_eq!(u32::from('😀'), cell.codepoint());
    }

    // Resize to 1 column can't fit a wide char. So it should be deleted.
    resize(&mut s, 1, 1);
    {
        assert_eq!("", dump(&s, screen_pt(0, 0)));
    }
    {
        let cell = cell_at(&s, screen_pt(0, 0));
        assert_eq!(0, cell.codepoint());
        assert_eq!(Wide::Narrow, cell.wide());
    }
}

#[test]
fn screen_resize_less_cols_to_wrap_wide_char() {
    let mut s = init(3, 3, 0);
    let str = "x😀";
    s.test_write_string(str).unwrap();
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
    {
        let cell = cell_at(&s, screen_pt(1, 0));
        assert_eq!(Wide::Wide, cell.wide());
        assert_eq!(u32::from('😀'), cell.codepoint());
    }
    {
        let cell = cell_at(&s, screen_pt(2, 0));
        assert_eq!(Wide::SpacerTail, cell.wide());
    }

    resize(&mut s, 2, 3);
    {
        assert_eq!("x\n😀", dump(&s, screen_pt(0, 0)));
    }
    {
        let cell = cell_at(&s, screen_pt(1, 0));
        assert_eq!(Wide::SpacerHead, cell.wide());
        assert!(row_at(&s, screen_pt(0, 0)).wrap());
    }
}

#[test]
fn screen_resize_less_cols_to_eliminate_wide_char_with_row_space() {
    let mut s = init(2, 2, 0);
    let str = "😀";
    s.test_write_string(str).unwrap();
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
    {
        let cell = cell_at(&s, screen_pt(0, 0));
        assert_eq!(Wide::Wide, cell.wide());
        assert_eq!(u32::from('😀'), cell.codepoint());
    }
    {
        let cell = cell_at(&s, screen_pt(1, 0));
        assert_eq!(Wide::SpacerTail, cell.wide());
    }

    resize(&mut s, 1, 2);
    {
        assert_eq!("", dump(&s, screen_pt(0, 0)));
    }
}

#[test]
fn screen_resize_more_cols_with_wide_spacer_head() {
    let mut s = init(3, 2, 0);
    let str = "  😀";
    s.test_write_string(str).unwrap();
    {
        assert_eq!("  \n😀", dump(&s, screen_pt(0, 0)));
    }

    // So this is the key point: we end up with a wide spacer head at
    // the end of row 1, then the emoji, then a wide spacer tail on row 2.
    // We should expect that if we resize to more cols, the wide spacer
    // head is replaced with the emoji.
    {
        let cell = cell_at(&s, screen_pt(2, 0));
        assert_eq!(Wide::SpacerHead, cell.wide());
    }
    {
        let cell = cell_at(&s, screen_pt(0, 1));
        assert_eq!(Wide::Wide, cell.wide());
    }
    {
        let cell = cell_at(&s, screen_pt(1, 1));
        assert_eq!(Wide::SpacerTail, cell.wide());
    }

    resize(&mut s, 4, 2);
    {
        assert_eq!(str, dump(&s, screen_pt(0, 0)));
    }
    {
        let cell = cell_at(&s, screen_pt(2, 0));
        assert_eq!(Wide::Wide, cell.wide());
        assert_eq!(u32::from('😀'), cell.codepoint());
    }
    {
        let cell = cell_at(&s, screen_pt(3, 0));
        assert_eq!(Wide::SpacerTail, cell.wide());
    }
}
