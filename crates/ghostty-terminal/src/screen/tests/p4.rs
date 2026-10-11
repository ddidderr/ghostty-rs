use super::*;

/// Asserts the selection start/end map to the expected points in `tag`.
fn expect_sel(s: &Screen, sel: &Selection, tag: Tag, start: (u16, u32), end: (u16, u32)) {
    let mk = |(x, y): (u16, u32)| Point::new(tag, Coordinate { x, y });
    assert_eq!(
        mk(start),
        s.pages.point_from_pin(tag, sel.start(&s.pages)).unwrap()
    );
    assert_eq!(
        mk(end),
        s.pages.point_from_pin(tag, sel.end(&s.pages)).unwrap()
    );
}

fn line(s: &Screen, x: u16, y: u32) -> Selection {
    s.select_line(SelectLine::new(pin(s, active(x, y))))
        .unwrap()
}

fn sel_str(s: &Screen, sel: Selection, trim: bool) -> String {
    s.selection_string(SelectionString { sel, trim })
}

const BOUNDARY: &[u32] = &[
    0,
    ' ' as u32,
    '\t' as u32,
    '\'' as u32,
    '"' as u32,
    '│' as u32,
    '`' as u32,
    '|' as u32,
    ':' as u32,
    ';' as u32,
    ',' as u32,
    '(' as u32,
    ')' as u32,
    '[' as u32,
    ']' as u32,
    '{' as u32,
    '}' as u32,
    '<' as u32,
    '>' as u32,
    '$' as u32,
];

fn word(s: &Screen, x: u16, y: u32) -> Selection {
    s.select_word(pin(s, active(x, y)), BOUNDARY).unwrap()
}

#[test]
fn screen_resize_more_cols_with_wide_spacer_head_multiple_lines() {
    let mut s = init(3, 3, 0);
    let str = "xxxyy😀";
    s.test_write_string(str).unwrap();
    assert_eq!("xxx\nyy\n😀", dump(&s, screen_pt(0, 0)));

    // Similar to the "wide spacer head" test, but this time we'er going
    // to increase our columns such that multiple rows are unwrapped.
    assert_eq!(
        Wide::SpacerHead,
        s.pages.get_cell(screen_pt(2, 1)).unwrap().wide()
    );
    assert_eq!(
        Wide::Wide,
        s.pages.get_cell(screen_pt(0, 2)).unwrap().wide()
    );
    assert_eq!(
        Wide::SpacerTail,
        s.pages.get_cell(screen_pt(1, 2)).unwrap().wide()
    );

    s.resize(Resize::new(8, 2)).unwrap();
    assert_eq!(str, dump(&s, screen_pt(0, 0)));
    {
        let cell = s.pages.get_cell(screen_pt(5, 0)).unwrap();
        assert_eq!(Wide::Wide, cell.wide());
        assert_eq!('😀' as u32, cell.codepoint());
    }
    assert_eq!(
        Wide::SpacerTail,
        s.pages.get_cell(screen_pt(6, 0)).unwrap().wide()
    );
}

#[test]
fn screen_resize_more_cols_requiring_a_wide_spacer_head() {
    let mut s = init(2, 2, 0);
    let str = "xx😀";
    s.test_write_string(str).unwrap();
    assert_eq!("xx\n😀", dump(&s, screen_pt(0, 0)));
    assert_eq!(
        Wide::Wide,
        s.pages.get_cell(screen_pt(0, 1)).unwrap().wide()
    );
    assert_eq!(
        Wide::SpacerTail,
        s.pages.get_cell(screen_pt(1, 1)).unwrap().wide()
    );

    // This resizes to 3 columns, which isn't enough space for our wide
    // char to enter row 1. But we need to mark the wide spacer head on the
    // end of the first row since we're wrapping to the next row.
    s.resize(Resize::new(3, 2)).unwrap();
    assert_eq!("xx\n😀", dump(&s, screen_pt(0, 0)));
    assert_eq!(
        Wide::SpacerHead,
        s.pages.get_cell(screen_pt(2, 0)).unwrap().wide()
    );
    {
        let cell = s.pages.get_cell(screen_pt(0, 1)).unwrap();
        assert_eq!(Wide::Wide, cell.wide());
        assert_eq!('😀' as u32, cell.codepoint());
    }
    assert_eq!(
        Wide::SpacerTail,
        s.pages.get_cell(screen_pt(1, 1)).unwrap().wide()
    );
}

#[test]
fn screen_resize_more_cols_with_cursor_at_prompt() {
    let mut s = init(10, 3, 5);

    s.test_write_string("ABCDE\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("> ").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearEol));
    s.test_write_string("echo").unwrap();

    assert_eq!("ABCDE\n> echo", dump(&s, viewport(0, 0)));

    s.resize(Resize {
        prompt_redraw: Redraw::True,
        ..Resize::new(20, 3)
    })
    .unwrap();

    // Cursor should not move
    assert_eq!(6, s.cursor.x);
    assert_eq!(1, s.cursor.y);

    assert_eq!("ABCDE", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_resize_more_cols_with_cursor_not_at_prompt() {
    let mut s = init(10, 3, 5);

    s.test_write_string("ABCDE\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("> ").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearEol));
    s.test_write_string("echo\n").unwrap();
    s.test_write_string("output").unwrap();

    assert_eq!("ABCDE\n> echo\noutput", dump(&s, viewport(0, 0)));

    s.resize(Resize {
        prompt_redraw: Redraw::True,
        ..Resize::new(20, 3)
    })
    .unwrap();

    // Cursor should not move
    assert_eq!(6, s.cursor.x);
    assert_eq!(2, s.cursor.y);

    assert_eq!("ABCDE\n> echo\noutput", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_resize_with_prompt_redraw_last_clears_only_one_line() {
    let mut s = init(10, 4, 5);

    s.test_write_string("ABCDE\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("> ").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearExplicit));
    s.test_write_string("hello\n").unwrap();
    s.test_write_string("world").unwrap();

    assert_eq!("ABCDE\n> hello\nworld", dump(&s, viewport(0, 0)));

    // Cursor is at end of "world" line with semantic_content = .input
    s.resize(Resize {
        prompt_redraw: Redraw::Last,
        ..Resize::new(20, 4)
    })
    .unwrap();

    // With .last, only the current line where cursor is should be cleared
    assert_eq!("ABCDE\n> hello", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_resize_with_prompt_redraw_last_multiline_prompt_clears_only_last_line() {
    let mut s = init(20, 5, 5);

    // Create a 3-line prompt: 1 initial + 2 continuation lines
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("line1\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Continuation));
    s.test_write_string("line2\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Continuation));
    s.test_write_string("line3").unwrap();

    assert_eq!("line1\nline2\nline3", dump(&s, viewport(0, 0)));

    // Cursor is at end of line3 (the last continuation line)
    s.resize(Resize {
        prompt_redraw: Redraw::Last,
        ..Resize::new(30, 5)
    })
    .unwrap();

    // With .last, only line3 (where cursor is) should be cleared
    assert_eq!("line1\nline2", dump(&s, viewport(0, 0)));
}

#[test]
fn screen_select_untracked() {
    let mut s = init(10, 10, 0);
    s.test_write_string("ABC  DEF\n 123\n456").unwrap();

    assert!(s.selection.is_none());
    let tracked = s.pages.count_tracked_pins();
    let sel = Selection::new(pin(&s, active(0, 0)), pin(&s, active(3, 0)), false);
    s.select(Some(sel)).unwrap();
    assert_eq!(tracked + 2, s.pages.count_tracked_pins());
    s.select(None).unwrap();
    assert_eq!(tracked, s.pages.count_tracked_pins());
}

#[test]
fn screen_select_all() {
    let mut s = init(10, 10, 0);

    {
        s.test_write_string("ABC  DEF\n 123\n456").unwrap();
        let sel = s.select_all().unwrap();
        expect_sel(&s, &sel, Tag::Screen, (0, 0), (2, 2));
    }

    {
        s.test_write_string("\nFOO\n BAR\n BAZ\n QWERTY\n 12345678")
            .unwrap();
        let sel = s.select_all().unwrap();
        expect_sel(&s, &sel, Tag::Screen, (0, 0), (8, 7));
    }
}

#[test]
fn screen_select_line() {
    let mut s = init(10, 10, 0);
    s.test_write_string("ABC  DEF\n 123\n456").unwrap();

    // Going forward
    let sel = line(&s, 0, 0);
    expect_sel(&s, &sel, Tag::Screen, (0, 0), (7, 0));

    // Going backward
    let sel = line(&s, 7, 0);
    expect_sel(&s, &sel, Tag::Screen, (0, 0), (7, 0));

    // Going forward and backward
    let sel = line(&s, 3, 0);
    expect_sel(&s, &sel, Tag::Screen, (0, 0), (7, 0));

    // Outside active area
    let sel = line(&s, 9, 0);
    expect_sel(&s, &sel, Tag::Screen, (0, 0), (7, 0));
}

#[test]
fn screen_select_line_across_soft_wrap() {
    let mut s = init(5, 10, 0);
    s.test_write_string(" 12 34012   \n 123").unwrap();

    // Going forward
    let sel = line(&s, 1, 0);
    expect_sel(&s, &sel, Tag::Screen, (1, 0), (3, 1));
}

#[test]
fn screen_select_line_across_full_soft_wrap() {
    let mut s = init(5, 5, 0);
    s.test_write_string("1ABCD2EFGH\n3IJKL").unwrap();

    let sel = line(&s, 2, 1);
    expect_sel(&s, &sel, Tag::Screen, (0, 0), (4, 1));
}

#[test]
fn screen_select_line_across_soft_wrap_ignores_blank_lines() {
    let mut s = init(5, 10, 0);
    s.test_write_string(" 12 34012             \n 123").unwrap();

    // Going forward
    let sel = line(&s, 1, 0);
    expect_sel(&s, &sel, Tag::Screen, (1, 0), (3, 1));

    // Going backward
    let sel = line(&s, 1, 1);
    expect_sel(&s, &sel, Tag::Screen, (1, 0), (3, 1));

    // Going forward and backward
    let sel = line(&s, 3, 0);
    expect_sel(&s, &sel, Tag::Screen, (1, 0), (3, 1));
}

#[test]
fn screen_select_line_disabled_whitespace_trimming() {
    let mut s = init(5, 10, 0);
    s.test_write_string(" 12 34012   \n 123").unwrap();

    // Going forward
    let sel = s
        .select_line(SelectLine {
            whitespace: None,
            ..SelectLine::new(pin(&s, active(1, 0)))
        })
        .unwrap();
    expect_sel(&s, &sel, Tag::Screen, (0, 0), (4, 2));

    // Non-wrapped
    let sel = s
        .select_line(SelectLine {
            whitespace: None,
            ..SelectLine::new(pin(&s, active(1, 3)))
        })
        .unwrap();
    expect_sel(&s, &sel, Tag::Screen, (0, 3), (4, 3));
}

#[test]
fn screen_select_line_with_scrollback() {
    let mut s = init(2, 3, 5);
    s.test_write_string("1A\n2B\n3C\n4D\n5E").unwrap();

    // Selecting first line
    let sel = line(&s, 0, 0);
    expect_sel(&s, &sel, Tag::Active, (0, 0), (1, 0));

    // Selecting last line
    let sel = line(&s, 0, 2);
    expect_sel(&s, &sel, Tag::Active, (0, 2), (1, 2));
}

// https://github.com/mitchellh/ghostty/issues/1329
#[test]
fn screen_select_line_semantic_prompt_boundary() {
    let mut s = init(5, 10, 0);
    s.test_write_string("ABCDE\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("A    ").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Output);
    s.test_write_string("> ").unwrap();

    assert_eq!("ABCDE\nA    \n> ", dump(&s, screen_pt(0, 0)));

    // Selecting output stops at the prompt even if soft-wrapped
    {
        let sel = line(&s, 1, 1);
        assert_eq!("A", sel_str(&s, sel, false));
    }
    {
        let sel = line(&s, 1, 2);
        expect_sel(&s, &sel, Tag::Active, (0, 2), (0, 2));
    }
}

#[test]
fn screen_select_line_semantic_prompt_to_input_boundary() {
    let mut s = init(10, 5, 0);

    // Write prompt followed by user input on same row: "$>command"
    // Using non-whitespace to avoid whitespace trimming affecting the test
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("$>").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearExplicit));
    s.test_write_string("command").unwrap();

    // Selecting from prompt should only select prompt
    let sel = line(&s, 0, 0);
    expect_sel(&s, &sel, Tag::Active, (0, 0), (1, 0));

    // Selecting from input should only select input
    let sel = line(&s, 5, 0);
    expect_sel(&s, &sel, Tag::Active, (2, 0), (8, 0));
}

#[test]
fn screen_select_line_semantic_input_to_output_boundary() {
    let mut s = init(10, 5, 0);

    // Row 0: user input
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearExplicit));
    s.test_write_string("ls -la\n").unwrap();
    // Row 1: command output
    s.cursor_set_semantic_content(SemanticContentSet::Output);
    s.test_write_string("file.txt").unwrap();

    // Selecting from input should only select input
    let sel = line(&s, 2, 0);
    assert_eq!("ls -la", sel_str(&s, sel, false));

    // Selecting from output should only select output
    let sel = line(&s, 2, 1);
    assert_eq!("file.txt", sel_str(&s, sel, false));
}

#[test]
fn screen_select_line_semantic_mid_row_boundary() {
    let mut s = init(10, 5, 0);

    // Single row with output then prompt then input: "out$>cmd"
    // Using non-whitespace to avoid whitespace trimming affecting the test
    s.cursor_set_semantic_content(SemanticContentSet::Output);
    s.test_write_string("out").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("$>").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearExplicit));
    s.test_write_string("cmd").unwrap();

    // Selecting from output should stop at prompt
    let sel = line(&s, 1, 0);
    expect_sel(&s, &sel, Tag::Active, (0, 0), (2, 0));

    // Selecting from prompt should only select prompt
    let sel = line(&s, 3, 0);
    expect_sel(&s, &sel, Tag::Active, (3, 0), (4, 0));

    // Selecting from input should only select input
    let sel = line(&s, 6, 0);
    expect_sel(&s, &sel, Tag::Active, (5, 0), (7, 0));
}

#[test]
fn screen_select_line_semantic_boundary_soft_wrap_with_mid_row_transition() {
    let mut s = init(5, 5, 0);

    // Row 0: prompt "$ " + input "cmd" (soft-wraps)
    // Row 1: input continues "12" + output "out"
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("$ ").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearExplicit));
    s.test_write_string("cmd12").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Output);
    s.test_write_string("out").unwrap();

    // Verify layout
    assert_eq!("$ cmd\n12out", dump(&s, screen_pt(0, 0)));

    // Selecting from input on row 0 should get all input across soft-wrap
    let sel = line(&s, 3, 0);
    assert_eq!("cmd12", sel_str(&s, sel, false));

    // Selecting from input on row 1 should get all input across soft-wrap
    let sel = line(&s, 0, 1);
    assert_eq!("cmd12", sel_str(&s, sel, false));

    // Selecting from output should only get output
    let sel = line(&s, 3, 1);
    assert_eq!("out", sel_str(&s, sel, false));
}

#[test]
fn screen_select_line_semantic_boundary_disabled() {
    let mut s = init(10, 5, 0);

    // Write prompt followed by input
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("$ ").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearExplicit));
    s.test_write_string("command").unwrap();

    // With semantic_prompt_boundary = false, should select entire line
    let sel = s
        .select_line(SelectLine {
            semantic_prompt_boundary: false,
            ..SelectLine::new(pin(&s, active(0, 0)))
        })
        .unwrap();
    assert_eq!("$ command", sel_str(&s, sel, false));
}

#[test]
fn screen_select_line_semantic_boundary_first_cell_of_row() {
    let mut s = init(5, 5, 0);

    // Row 0: input that soft-wraps
    // Row 1: output starts at first cell
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearExplicit));
    s.test_write_string("12345").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Output);
    s.test_write_string("ABCDE").unwrap();

    // Verify soft-wrap happened
    {
        let p = pin(&s, active(0, 0));
        let (row, _) = p.row_and_cell(&s.pages).unwrap();
        assert!(row.wrap());
    }

    // Selecting from input should stop before output on row 1
    let sel = line(&s, 2, 0);
    expect_sel(&s, &sel, Tag::Active, (0, 0), (4, 0));

    // Selecting from output should only get output
    let sel = line(&s, 2, 1);
    expect_sel(&s, &sel, Tag::Active, (0, 1), (4, 1));
}

#[test]
fn screen_select_line_semantic_all_same_content() {
    let mut s = init(5, 5, 0);

    // All prompt content that soft-wraps
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("prompt text").unwrap();

    // Verify soft-wrap
    assert_eq!("promp\nt tex\nt", dump(&s, screen_pt(0, 0)));

    // Should select all prompt content across soft-wraps
    let sel = line(&s, 2, 1);
    assert_eq!("prompt text", sel_str(&s, sel, false));
}

#[test]
fn screen_select_word() {
    let mut s = init(10, 10, 0);
    s.test_write_string("ABC  DEF\n 123\n456").unwrap();

    // Going forward
    let sel = word(&s, 0, 0);
    expect_sel(&s, &sel, Tag::Screen, (0, 0), (2, 0));

    // Going backward
    let sel = word(&s, 2, 0);
    expect_sel(&s, &sel, Tag::Screen, (0, 0), (2, 0));

    // Going forward and backward
    let sel = word(&s, 1, 0);
    expect_sel(&s, &sel, Tag::Screen, (0, 0), (2, 0));

    // Whitespace
    let sel = word(&s, 3, 0);
    expect_sel(&s, &sel, Tag::Screen, (3, 0), (4, 0));

    // Whitespace single char
    let sel = word(&s, 0, 1);
    expect_sel(&s, &sel, Tag::Screen, (0, 1), (0, 1));

    // End of screen
    let sel = word(&s, 1, 2);
    expect_sel(&s, &sel, Tag::Screen, (0, 2), (2, 2));
}

#[test]
fn screen_select_word_across_soft_wrap() {
    let mut s = init(5, 10, 0);
    s.test_write_string(" 1234012\n 123").unwrap();

    assert_eq!(" 1234\n012\n 123", dump(&s, screen_pt(0, 0)));

    // Going forward
    let sel = word(&s, 1, 0);
    expect_sel(&s, &sel, Tag::Screen, (1, 0), (2, 1));

    // Going backward
    let sel = word(&s, 1, 1);
    expect_sel(&s, &sel, Tag::Screen, (1, 0), (2, 1));

    // Going forward and backward
    let sel = word(&s, 3, 0);
    expect_sel(&s, &sel, Tag::Screen, (1, 0), (2, 1));
}

#[test]
fn screen_select_word_whitespace_across_soft_wrap() {
    let mut s = init(5, 10, 0);
    s.test_write_string("1       1\n 123").unwrap();

    // Going forward
    let sel = word(&s, 1, 0);
    expect_sel(&s, &sel, Tag::Screen, (1, 0), (2, 1));

    // Going backward
    let sel = word(&s, 1, 1);
    expect_sel(&s, &sel, Tag::Screen, (1, 0), (2, 1));

    // Going forward and backward
    let sel = word(&s, 3, 0);
    expect_sel(&s, &sel, Tag::Screen, (1, 0), (2, 1));
}

#[test]
fn screen_select_word_with_character_boundary() {
    let cases = [
        " 'abc' \n123",
        " \"abc\" \n123",
        " │abc│ \n123",
        " `abc` \n123",
        " |abc| \n123",
        " :abc: \n123",
        " ;abc; \n123",
        " ,abc, \n123",
        " (abc( \n123",
        " )abc) \n123",
        " [abc[ \n123",
        " ]abc] \n123",
        " {abc{ \n123",
        " }abc} \n123",
        " <abc< \n123",
        " >abc> \n123",
        " $abc$ \n123",
    ];

    for case in cases {
        let mut s = init(20, 10, 0);
        s.test_write_string(case).unwrap();

        // Inside character forward
        let sel = word(&s, 2, 0);
        expect_sel(&s, &sel, Tag::Screen, (2, 0), (4, 0));

        // Inside character backward
        let sel = word(&s, 4, 0);
        expect_sel(&s, &sel, Tag::Screen, (2, 0), (4, 0));

        // Inside character bidirectional
        let sel = word(&s, 3, 0);
        expect_sel(&s, &sel, Tag::Screen, (2, 0), (4, 0));

        // On quote
        // NOTE: this behavior is not ideal, so we can change this one day,
        // but I think its also not that important compared to the above.
        let sel = word(&s, 1, 0);
        expect_sel(&s, &sel, Tag::Screen, (0, 0), (1, 0));
    }
}

#[test]
fn screen_select_output() {
    let mut s = init(10, 15, 0);

    // Build content with cell-level semantic content:
    // Row 0-1: output1 (output)
    // Row 2: prompt2 (prompt)
    // Row 3: input2 (input)
    // Row 4-7: output2 (output, with overflow causing wrap)
    // Row 8: "$ " (prompt) + "input3" (input)
    // Row 9-11: output3 (output)
    s.cursor_set_semantic_content(SemanticContentSet::Output);
    s.test_write_string("output1\n").unwrap();
    s.test_write_string("output1\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("prompt2\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearExplicit));
    s.test_write_string("input2\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Output);
    s.test_write_string("output2output2output2output2\n")
        .unwrap();
    s.test_write_string("output2\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Prompt(PromptKind::Initial));
    s.test_write_string("$ ").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Input(InputClear::ClearExplicit));
    s.test_write_string("input3\n").unwrap();
    s.cursor_set_semantic_content(SemanticContentSet::Output);
    s.test_write_string("output3\n").unwrap();
    s.test_write_string("output3\n").unwrap();
    s.test_write_string("output3").unwrap();

    // First output block (rows 0-1), should select those rows
    {
        let sel = s.select_output(pin(&s, active(1, 1))).unwrap();
        assert_eq!("output1\noutput1", sel_str(&s, sel, false));
    }

    // Second output block (rows 4-7)
    {
        let sel = s.select_output(pin(&s, active(3, 7))).unwrap();
        assert_eq!(
            "output2output2output2output2\noutput2",
            sel_str(&s, sel, false)
        );
    }

    // Third output block (rows 9-11)
    {
        let sel = s.select_output(pin(&s, active(2, 10))).unwrap();
        expect_sel(&s, &sel, Tag::Active, (0, 9), (6, 11));
    }

    // Click on prompt should return null
    assert!(s.select_output(pin(&s, active(1, 8))).is_none());

    // Click on input should return null
    assert!(s.select_output(pin(&s, active(5, 8))).is_none());
}

#[test]
fn screen_selection_string_basic() {
    let mut s = init(5, 3, 0);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    let sel = Selection::new(pin(&s, screen_pt(0, 1)), pin(&s, screen_pt(2, 2)), false);
    assert_eq!("2EFGH\n3IJ", sel_str(&s, sel, true));
}

#[test]
fn screen_selection_string_start_outside_of_written_area() {
    let mut s = init(5, 10, 0);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    let sel = Selection::new(pin(&s, screen_pt(0, 5)), pin(&s, screen_pt(2, 6)), false);
    assert_eq!("", sel_str(&s, sel, true));
}

#[test]
fn screen_selection_string_end_outside_of_written_area() {
    let mut s = init(5, 10, 0);
    s.test_write_string("1ABCD\n2EFGH\n3IJKL").unwrap();
    let sel = Selection::new(pin(&s, screen_pt(0, 2)), pin(&s, screen_pt(2, 6)), false);
    assert_eq!("3IJKL", sel_str(&s, sel, true));
}

#[test]
fn screen_selection_string_trim_space() {
    let mut s = init(5, 3, 0);
    s.test_write_string("1AB  \n2EFGH\n3IJKL").unwrap();
    let sel = Selection::new(pin(&s, screen_pt(0, 0)), pin(&s, screen_pt(2, 1)), false);
    assert_eq!("1AB\n2EF", sel_str(&s, sel, true));

    // No trim
    assert_eq!("1AB  \n2EF", sel_str(&s, sel, false));
}

#[test]
fn screen_selection_string_trim_empty_line() {
    let mut s = init(5, 5, 0);
    s.test_write_string("1AB  \n\n2EFGH\n3IJKL").unwrap();

    let sel = Selection::new(pin(&s, screen_pt(0, 0)), pin(&s, screen_pt(2, 2)), false);

    assert_eq!("1AB\n\n2EF", sel_str(&s, sel, true));

    // No trim
    assert_eq!("1AB  \n\n2EF", sel_str(&s, sel, false));
}
