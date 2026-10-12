//! IME: preedit state and rendering, commit, composing key events.

// These drive the real surface over a headless OpenGL context; the Apple targets render
// with Metal.
#![cfg(not(target_vendor = "apple"))]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;
use ghostty_input::{Key, KeyEvent};

/// RGBA pixels of the cells `cols` on `row` of the last drawn frame.
fn region(h: &Harness, cols: std::ops::Range<u32>, row: u32) -> Vec<u8> {
    h.surface.draw().expect("draw");
    let Some((w, hgt, px)) = h.surface.frame_pixels() else {
        return Vec::new();
    };
    let s = h.surface.size();
    let x0 = (s.padding.left + cols.start * s.cell.width) as usize;
    let x1 = (s.padding.left + cols.end * s.cell.width) as usize;
    let y0 = (s.padding.top + row * s.cell.height) as usize;
    let y1 = y0 + s.cell.height as usize;
    let mut out = Vec::new();
    for y in y0..y1 {
        // frame is bottom-up
        let fy = hgt - 1 - y;
        let start = (fy * w + x0) * 4;
        let end = (fy * w + x1) * 4;
        out.extend_from_slice(&px[start..end]);
    }
    out
}

#[test]
fn preedit_state_and_ime_point() {
    let h = harness!(Harness::recorder("", &[]));
    let before = h.surface.ime_point();
    h.surface
        .preedit_callback(Some("\u{3042}\u{3044}"))
        .unwrap();
    {
        let st = h.surface.lock_state();
        let p = st.preedit.as_ref().expect("preedit set");
        assert_eq!(p.codepoints.len(), 2);
        assert_eq!(p.codepoints[0].codepoint, 0x3042);
        assert!(p.codepoints[0].wide);
    }
    // The candidate window sits below the cursor and spans the preedit.
    let after = h.surface.ime_point();
    assert_eq!(after.x, before.x);
    assert_eq!(after.y, before.y);
    assert!(after.width >= f64::from(2 * 2 * h.surface.size().cell.width) - 1.0);
    h.surface.preedit_callback(None).unwrap();
    assert!(h.surface.lock_state().preedit.is_none());
    // Nothing is sent to the child by preedit alone.
    h.settle(50);
    assert_eq!(h.recorded(), Vec::<u8>::new());
}

#[test]
fn preedit_is_rendered_and_cleared() {
    let h = harness!(Harness::recorder("", &[]));
    // Cursor sits after "READY" at column 5, row 0. Disable blink noise by
    // comparing the cells after the cursor cell.
    let cols = 7..10;
    let baseline = {
        let mut b = Vec::new();
        h.wait("first frame", |h| {
            b = region(h, cols.clone(), 0);
            !b.is_empty()
        });
        b
    };
    h.surface.preedit_callback(Some("abcd")).unwrap();
    h.wait("preedit visible", |h| {
        let r = region(h, cols.clone(), 0);
        !r.is_empty() && r != baseline
    });
    h.surface.preedit_callback(None).unwrap();
    h.wait("preedit cleared", |h| {
        region(h, cols.clone(), 0) == baseline
    });
}

#[test]
fn commit_sends_text_and_composing_keys_do_not() {
    let h = harness!(Harness::recorder("", &[]));
    // A key event that is part of a composition is not encoded.
    h.key_event(KeyEvent {
        key: Key::KeyK,
        composing: true,
        utf8: "k",
        unshifted_codepoint: 'k' as u32,
        ..KeyEvent::default()
    });
    h.settle(50);
    assert_eq!(h.recorded(), Vec::<u8>::new());

    // Commit: preedit ends, committed text is sent with no key.
    h.surface
        .preedit_callback(Some("\u{3053}\u{3093}"))
        .unwrap();
    h.surface.preedit_callback(None).unwrap();
    h.key_event(KeyEvent {
        key: Key::Unidentified,
        utf8: "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}",
        ..KeyEvent::default()
    });
    h.expect_input("\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}".as_bytes());
}

#[test]
fn enter_with_committed_text_sends_text_not_cr() {
    // IME confirmation sends enter with utf8: the text is sent, not "\r".
    let h = harness!(Harness::recorder("", &[]));
    h.key_event(KeyEvent {
        key: Key::Enter,
        utf8: "\u{6f22}",
        ..KeyEvent::default()
    });
    h.expect_input("\u{6f22}".as_bytes());
}
