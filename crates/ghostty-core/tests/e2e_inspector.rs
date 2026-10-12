//! The terminal inspector hooks of the surface: activation, key event
//! recording (with the pty encoding and triggered bindings) and VT event
//! recording while data is read from the pty.

// These drive the real surface over a headless OpenGL context; the Apple targets render
// with Metal.
#![cfg(not(target_vendor = "apple"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::assert_is_empty)]

mod common;
use common::*;
use ghostty_input::Key;

#[test]
fn inspector_records_key_events_with_pty_data_and_bindings() {
    let h = harness!(Harness::recorder(
        "",
        &["--keybind=ctrl+shift+c=scroll_page_up"]
    ));
    h.surface.activate_inspector().unwrap();
    let insp = h.surface.inspector().expect("active inspector");

    h.key(Key::KeyA, none(), "a");
    h.key(Key::KeyC, ctrl_shift(), "C");
    h.expect_input(b"a");

    let g = insp.lock();
    let events = &g.gui.key_stream.events;
    // A press for each key.
    let press_a = events
        .iter()
        .find(|e| e.event.key == Key::KeyA && e.event.action == ghostty_input::Action::Press)
        .expect("press a");
    assert_eq!(press_a.pty, b"a");
    assert_eq!(press_a.event.utf8, "a");
    assert!(press_a.binding.is_empty());

    let press_c = events
        .iter()
        .find(|e| e.event.key == Key::KeyC && e.event.action == ghostty_input::Action::Press)
        .expect("press ctrl+shift+c");
    assert!(
        press_c
            .binding
            .iter()
            .any(|a| a.key().name() == "scroll_page_up"),
        "binding recorded: {:?}",
        press_c.binding
    );
    // The consumed binding was not encoded to the pty.
    assert!(press_c.pty.is_empty());
}

#[test]
fn inspector_records_vt_events_read_from_the_pty() {
    let script = "stty raw -echo; printf READY; dd bs=1 count=1 >/dev/null 2>&1; \
                  printf '\\033[31mX\\033[0m\\033]0;title\\007'; printf DONE; exec cat >'{out}'";
    let h = harness!(Harness::new(Opts::default().script(script)));
    h.wait_screen("READY");

    h.surface.activate_inspector().unwrap();
    let insp = h.surface.inspector().expect("active inspector");
    {
        // The overlay features come from the inspector's renderer info.
        let mut g = insp.lock();
        assert!(g.renderer_info().features.is_empty());
    }

    h.type_text("a");
    h.wait_screen("DONE");

    let g = insp.lock();
    let descs: Vec<String> = g
        .gui
        .vt_stream
        .events()
        .iter()
        .map(|e| e.raw_description.clone())
        .collect();
    assert!(descs.iter().any(|d| d == "31 m"), "events: {descs:?}");
    assert!(descs.iter().any(|d| d == "0 m"), "events: {descs:?}");
    assert!(
        descs.iter().any(|d| d == "change_window_title "),
        "events: {descs:?}"
    );
    drop(g);

    // Deactivating drops the inspector and removes it from the render state.
    h.surface.deactivate_inspector();
    assert!(h.surface.inspector().is_none());
    assert!(h.surface.lock_state().inspector.is_none());
}

#[test]
fn inspector_overlay_features_reach_the_render_state() {
    use ghostty_renderer::overlay::Feature;

    let h = harness!(Harness::recorder("", &[]));
    h.surface.activate_inspector().unwrap();
    let insp = h.surface.inspector().expect("active inspector");
    insp.lock()
        .renderer_info()
        .features
        .push(Feature::HighlightHyperlinks);

    let st = h.surface.lock_state();
    let hook = st.inspector.as_ref().expect("inspector in render state");
    assert_eq!(hook.overlay_features(), vec![Feature::HighlightHyperlinks]);
}

#[test]
fn activating_twice_is_a_no_op() {
    let h = harness!(Harness::recorder("", &[]));
    h.surface.activate_inspector().unwrap();
    let first = h.surface.inspector().unwrap();
    h.surface.activate_inspector().unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &first,
        &h.surface.inspector().unwrap()
    ));
    h.surface.deactivate_inspector();
    // Deactivating when there is none is fine, too.
    h.surface.deactivate_inspector();
}
