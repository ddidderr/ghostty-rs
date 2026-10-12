use ghostty_vt::sgr::Attribute as A;

use super::*;

#[test]
fn screen_read_and_write() {
    let mut s = init(80, 24, 1000);
    assert_eq!(0, s.cursor.style_id);

    s.test_write_string("hello, world").unwrap();
    let str = dump(&s, screen_pt(0, 0));
    assert_eq!("hello, world", str);
}

#[test]
fn screen_read_and_write_newline() {
    let mut s = init(80, 24, 1000);
    assert_eq!(0, s.cursor.style_id);

    s.test_write_string("hello\nworld").unwrap();
    let str = dump(&s, screen_pt(0, 0));
    assert_eq!("hello\nworld", str);
}

#[test]
fn screen_read_and_write_scrollback() {
    let mut s = init(80, 2, 1000);

    s.test_write_string("hello\nworld\ntest").unwrap();
    assert_eq!("hello\nworld\ntest", dump(&s, screen_pt(0, 0)));
    assert_eq!("world\ntest", dump(&s, active(0, 0)));
}

#[test]
fn screen_read_and_write_no_scrollback_small() {
    let mut s = init(80, 2, 0);

    s.test_write_string("hello\nworld\ntest").unwrap();
    assert_eq!("world\ntest", dump(&s, screen_pt(0, 0)));
    assert_eq!("world\ntest", dump(&s, active(0, 0)));
}

#[test]
fn screen_read_and_write_no_scrollback_large() {
    let mut s = init(80, 2, 0);

    for i in 0..1_000 {
        s.test_write_string(&format!("{i}\n")).unwrap();
    }
    s.test_write_string("1000").unwrap();

    assert_eq!("999\n1000", dump(&s, screen_pt(0, 0)));
}

#[test]
fn screen_cursor_copy_x_y() {
    let mut s = init(10, 10, 0);
    s.cursor_absolute(2, 3);
    assert!(s.cursor.x == 2);
    assert!(s.cursor.y == 3);

    let mut s2 = init(10, 10, 0);
    s2.cursor_copy(&s.cursor, true).unwrap();
    assert!(s2.cursor.x == 2);
    assert!(s2.cursor.y == 3);
    s2.test_write_string("Hello").unwrap();

    assert_eq!("\n\n\n  Hello", dump(&s2, screen_pt(0, 0)));
}

#[test]
fn screen_cursor_copy_style_deref() {
    let s = init(10, 10, 0);

    let mut s2 = init(10, 10, 0);
    let page = s2.cursor_pin().node;

    // Bold should create our style
    s2.set_attribute(A::Bold).unwrap();
    assert_eq!(1, s2.page(page).styles.count());
    assert!(s2.cursor.style.flags.bold);

    // Copy default style, should release our style
    s2.cursor_copy(&s.cursor, true).unwrap();
    assert!(!s2.cursor.style.flags.bold);
    assert_eq!(0, s2.page(page).styles.count());
}

/// Builds the two-page setup shared by the "new page" cursorCopy tests.
fn cursor_on_second_page() -> Screen {
    let mut s2 = init(10, 10, 2048);

    // We need to get the cursor on a new page.
    let first = s2.pages.first.unwrap();
    let first_page_size = s2.page(first).capacity.rows;

    // Fill the scrollback with blank lines until
    // there are only 5 rows left on the first page.
    s2.page_mut(first).pause_integrity_checks(true);
    for _ in 0..first_page_size - 5 {
        s2.test_write_string("\n").unwrap();
    }
    s2.page_mut(first).pause_integrity_checks(false);

    s2.test_write_string("1\n2\n3\n4\n5\n6\n7\n8\n9\n10")
        .unwrap();

    // s2.pages.diagram(...):
    //
    //      +----------+ = PAGE 0
    //  ... :          :
    //     +-------------+ ACTIVE
    // 4300 |1         | | 0
    // 4301 |2         | | 1
    // 4302 |3         | | 2
    // 4303 |4         | | 3
    // 4304 |5         | | 4
    //      +----------+ :
    //      +----------+ : = PAGE 1
    //    0 |6         | | 5
    //    1 |7         | | 6
    //    2 |8         | | 7
    //    3 |9         | | 8
    //    4 |10        | | 9
    //      :  ^       : : = PIN 0
    //      +----------+ :
    //     +-------------+
    s2
}

#[test]
fn screen_cursor_copy_style_deref_new_page() {
    let s = init(10, 10, 0);
    let mut s2 = cursor_on_second_page();

    // This should be PAGE 1
    let page = s2.cursor_pin().node;

    // It should be the last page in the list.
    assert_eq!(Some(page), s2.pages.last);
    // It should have a previous page.
    assert!(s2.pages.node(page).unwrap().prev.is_some());

    // The cursor should be at 2, 9
    assert!(s2.cursor.x == 2);
    assert!(s2.cursor.y == 9);

    // Bold should create our style in page 1.
    s2.set_attribute(A::Bold).unwrap();
    assert_eq!(1, s2.page(page).styles.count());
    assert!(s2.cursor.style.flags.bold);

    // Copy the cursor for the first screen. This should release
    // the style from page 1 and move the cursor back to page 0.
    s2.cursor_copy(&s.cursor, true).unwrap();
    assert!(!s2.cursor.style.flags.bold);
    assert_eq!(0, s2.page(page).styles.count());
    // The page after the page the cursor is now in should be page 1.
    assert_eq!(
        Some(page),
        s2.pages.node(s2.cursor_pin().node).unwrap().next
    );
    // The cursor should be at 0, 0
    assert!(s2.cursor.x == 0);
    assert!(s2.cursor.y == 0);
}

#[test]
fn screen_cursor_copy_style_copy() {
    let mut s = init(10, 10, 0);
    s.set_attribute(A::Bold).unwrap();

    let mut s2 = init(10, 10, 0);
    let page = s2.cursor_pin().node;
    s2.cursor_copy(&s.cursor, true).unwrap();
    assert!(s2.cursor.style.flags.bold);
    assert_eq!(1, s2.page(page).styles.count());
}

#[test]
fn screen_cursor_copy_hyperlink_deref() {
    let s = init(10, 10, 0);

    let mut s2 = init(10, 10, 0);
    let page = s2.cursor_pin().node;

    // Create a hyperlink for the cursor.
    s2.start_hyperlink(b"https://example.com/", None).unwrap();
    assert_eq!(1, s2.page(page).hyperlink_set.count());
    assert!(s2.cursor.hyperlink_id != 0);

    // Copy a cursor with no hyperlink, should release our hyperlink.
    s2.cursor_copy(&s.cursor, true).unwrap();
    assert_eq!(0, s2.page(page).hyperlink_set.count());
    assert!(s2.cursor.hyperlink_id == 0);
}

#[test]
fn screen_cursor_copy_hyperlink_deref_new_page() {
    let s = init(10, 10, 0);
    let mut s2 = cursor_on_second_page();

    // This should be PAGE 1
    let page = s2.cursor_pin().node;

    // It should be the last page in the list.
    assert_eq!(Some(page), s2.pages.last);
    // It should have a previous page.
    assert!(s2.pages.node(page).unwrap().prev.is_some());

    // The cursor should be at 2, 9
    assert!(s2.cursor.x == 2);
    assert!(s2.cursor.y == 9);

    // Create a hyperlink for the cursor, should be in page 1.
    s2.start_hyperlink(b"https://example.com/", None).unwrap();
    assert_eq!(1, s2.page(page).hyperlink_set.count());
    assert!(s2.cursor.hyperlink_id != 0);

    // Copy the cursor for the first screen. This should release
    // the hyperlink from page 1 and move the cursor back to page 0.
    s2.cursor_copy(&s.cursor, true).unwrap();
    assert_eq!(0, s2.page(page).hyperlink_set.count());
    assert!(s2.cursor.hyperlink_id == 0);
    // The page after the page the cursor is now in should be page 1.
    assert_eq!(
        Some(page),
        s2.pages.node(s2.cursor_pin().node).unwrap().next
    );
    // The cursor should be at 0, 0
    assert!(s2.cursor.x == 0);
    assert!(s2.cursor.y == 0);
}

#[test]
fn screen_cursor_copy_hyperlink_copy() {
    let mut s = init(10, 10, 0);

    // Create a hyperlink for the cursor.
    s.start_hyperlink(b"https://example.com/", None).unwrap();
    assert_eq!(1, s.cursor_page().hyperlink_set.count());
    assert!(s.cursor.hyperlink_id != 0);

    let mut s2 = init(10, 10, 0);
    let page = s2.cursor_pin().node;

    assert_eq!(0, s2.page(page).hyperlink_set.count());
    assert!(s2.cursor.hyperlink_id == 0);

    // Copy the cursor with the hyperlink.
    s2.cursor_copy(&s.cursor, true).unwrap();
    assert_eq!(1, s2.page(page).hyperlink_set.count());
    assert!(s2.cursor.hyperlink_id != 0);
}

#[test]
fn screen_cursor_copy_hyperlink_copy_disabled() {
    let mut s = init(10, 10, 0);

    // Create a hyperlink for the cursor.
    s.start_hyperlink(b"https://example.com/", None).unwrap();
    assert_eq!(1, s.cursor_page().hyperlink_set.count());
    assert!(s.cursor.hyperlink_id != 0);

    let mut s2 = init(10, 10, 0);
    let page = s2.cursor_pin().node;

    assert_eq!(0, s2.page(page).hyperlink_set.count());
    assert!(s2.cursor.hyperlink_id == 0);

    // Copy the cursor with the hyperlink.
    s2.cursor_copy(&s.cursor, false).unwrap();
    assert_eq!(0, s2.page(page).hyperlink_set.count());
    assert!(s2.cursor.hyperlink_id == 0);
}

#[test]
fn screen_style_basics() {
    let mut s = init(80, 24, 1000);
    let page = s.cursor_pin().node;
    assert_eq!(0, s.page(page).styles.count());

    // Set a new style
    s.set_attribute(A::Bold).unwrap();
    assert!(s.cursor.style_id != 0);
    assert_eq!(1, s.page(page).styles.count());
    assert!(s.cursor.style.flags.bold);

    // Set another style, we should still only have one since it was unused
    s.set_attribute(A::Italic).unwrap();
    assert!(s.cursor.style_id != 0);
    assert_eq!(1, s.page(page).styles.count());
    assert!(s.cursor.style.flags.italic);
}

#[test]
fn screen_style_reset_to_default() {
    let mut s = init(80, 24, 1000);
    let page = s.cursor_pin().node;
    assert_eq!(0, s.page(page).styles.count());

    // Set a new style
    s.set_attribute(A::Bold).unwrap();
    assert!(s.cursor.style_id != 0);
    assert_eq!(1, s.page(page).styles.count());

    // Reset to default
    s.set_attribute(A::ResetBold).unwrap();
    assert!(s.cursor.style_id == 0);
    assert_eq!(0, s.page(page).styles.count());
}

#[test]
fn screen_style_reset_with_unset() {
    let mut s = init(80, 24, 1000);
    let page = s.cursor_pin().node;
    assert_eq!(0, s.page(page).styles.count());

    // Set a new style
    s.set_attribute(A::Bold).unwrap();
    assert!(s.cursor.style_id != 0);
    assert_eq!(1, s.page(page).styles.count());

    // Reset to default
    s.set_attribute(A::Unset).unwrap();
    assert!(s.cursor.style_id == 0);
    assert_eq!(0, s.page(page).styles.count());
}

#[test]
fn screen_clear_rows_active_one_line() {
    let mut s = init(80, 24, 1000);

    s.test_write_string("hello, world").unwrap();
    s.clear_rows(active(0, 0), None, false);
    assert!(s.pages.is_dirty(active(0, 0)));
    assert_eq!("", dump(&s, screen_pt(0, 0)));
}

#[test]
fn screen_clear_rows_active_multi_line() {
    let mut s = init(80, 24, 1000);

    s.test_write_string("hello\nworld").unwrap();
    s.clear_rows(active(0, 0), None, false);
    assert!(s.pages.is_dirty(active(0, 0)));
    assert!(s.pages.is_dirty(active(0, 1)));
    assert_eq!("", dump(&s, screen_pt(0, 0)));
}

#[test]
fn screen_clear_rows_active_styled_line() {
    let mut s = init(80, 24, 1000);

    s.set_attribute(A::Bold).unwrap();
    s.test_write_string("hello world").unwrap();
    s.set_attribute(A::Unset).unwrap();

    // We should have one style
    let page = s.cursor_pin().node;
    assert_eq!(1, s.page(page).styles.count());

    s.clear_rows(active(0, 0), None, false);

    // We should have none because active cleared it
    assert_eq!(0, s.page(page).styles.count());

    assert_eq!("", dump(&s, screen_pt(0, 0)));
}

#[test]
fn screen_clear_rows_protected() {
    let mut s = init(80, 24, 1000);

    s.test_write_string("UNPROTECTED").unwrap();
    s.cursor.protected = true;
    s.test_write_string("PROTECTED").unwrap();
    s.cursor.protected = false;
    s.test_write_string("UNPROTECTED").unwrap();
    s.test_write_string("\n").unwrap();
    s.cursor.protected = true;
    s.test_write_string("PROTECTED").unwrap();
    s.cursor.protected = false;
    s.test_write_string("UNPROTECTED").unwrap();
    s.cursor.protected = true;
    s.test_write_string("PROTECTED").unwrap();
    s.cursor.protected = false;

    s.clear_rows(active(0, 0), None, true);

    assert_eq!(
        "           PROTECTED\nPROTECTED           PROTECTED",
        dump(&s, screen_pt(0, 0))
    );
}

#[test]
fn screen_erase_rows_history() {
    let mut s = init(5, 5, 1000);

    s.test_write_string("1\n2\n3\n4\n5\n6").unwrap();

    assert_eq!("2\n3\n4\n5\n6", dump(&s, active(0, 0)));
    assert_eq!("1\n2\n3\n4\n5\n6", dump(&s, screen_pt(0, 0)));

    s.erase_rows(history(0, 0), None);

    assert_eq!("2\n3\n4\n5\n6", dump(&s, active(0, 0)));
    assert_eq!("2\n3\n4\n5\n6", dump(&s, screen_pt(0, 0)));
}

#[test]
fn screen_erase_rows_history_with_more_lines() {
    let mut s = init(5, 5, 1000);

    s.test_write_string("A\nB\nC\n1\n2\n3\n4\n5\n6").unwrap();

    assert_eq!("2\n3\n4\n5\n6", dump(&s, active(0, 0)));
    assert_eq!("A\nB\nC\n1\n2\n3\n4\n5\n6", dump(&s, screen_pt(0, 0)));

    s.erase_rows(history(0, 0), None);

    assert_eq!("2\n3\n4\n5\n6", dump(&s, active(0, 0)));
    assert_eq!("2\n3\n4\n5\n6", dump(&s, screen_pt(0, 0)));
}

#[test]
fn screen_erase_rows_active_partial() {
    let mut s = init(5, 5, 0);

    s.test_write_string("1\n2\n3").unwrap();

    assert_eq!("1\n2\n3", dump(&s, active(0, 0)));

    s.erase_rows(active(0, 0), Some(active(0, 1)));

    assert_eq!("3", dump(&s, active(0, 0)));
    assert_eq!("3", dump(&s, screen_pt(0, 0)));
}
