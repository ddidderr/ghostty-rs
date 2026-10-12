//! Cursor style decision logic (`renderer/cursor.zig`).

use ghostty_terminal::render::RenderState;

/// Terminal cursor styles (`terminal.CursorStyle`).
pub type TerminalCursorStyle = ghostty_terminal::cursor::Style;

/// Available cursor styles for drawing that renderers must support.
/// This is a superset of terminal cursor styles since the renderer supports
/// some additional cursor states such as the hollow block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    // Typical cursor input styles
    Block,
    BlockHollow,
    Bar,
    Underline,

    // Special cursor styles
    Lock,
}

impl Style {
    /// Create a cursor style from the terminal style request.
    pub fn from_terminal(term: TerminalCursorStyle) -> Style {
        match term {
            TerminalCursorStyle::Bar => Style::Bar,
            TerminalCursorStyle::Block => Style::Block,
            TerminalCursorStyle::BlockHollow => Style::BlockHollow,
            TerminalCursorStyle::Underline => Style::Underline,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StyleOptions {
    pub preedit: bool,
    pub focused: bool,
    pub blink_visible: bool,
}

/// Returns the cursor style to use for the current render state or None
/// if a cursor should not be rendered at all.
pub fn style(state: &RenderState, opts: StyleOptions) -> Option<Style> {
    // Note the order of conditionals below is important. It represents
    // a priority system of how we determine what state overrides cursor
    // visibility and style.

    // The cursor must be visible in the viewport to be rendered.
    state.cursor.viewport?;

    // If we are in preedit, then we always show the block cursor. We do
    // this even if the cursor is explicitly not visible because it shows
    // an important editing state to the user.
    if opts.preedit {
        return Some(Style::Block);
    }

    // If we're at a password input its always a lock.
    if state.cursor.password_input {
        return Some(Style::Lock);
    }

    // If the cursor is explicitly not visible by terminal mode, we don't render.
    if !state.cursor.visible {
        return None;
    }

    // If we're not focused, our cursor is always visible so that
    // we can show the hollow box.
    if !opts.focused {
        return Some(Style::BlockHollow);
    }

    // If the cursor is blinking and our blink state is not visible,
    // then we don't show the cursor.
    if state.cursor.blinking && !opts.blink_visible {
        return None;
    }

    // Otherwise, we use whatever style the terminal wants.
    Some(Style::from_terminal(state.cursor.visual_style))
}

#[cfg(test)]
mod tests {
    use ghostty_terminal::terminal::{Options, ScrollViewport, Terminal};
    use ghostty_vt::modes::Mode;

    use super::*;

    fn o(preedit: bool, focused: bool, blink_visible: bool) -> StyleOptions {
        StyleOptions {
            preedit,
            focused,
            blink_visible,
        }
    }

    fn term() -> Terminal {
        Terminal::new(Options::new(10, 10)).unwrap()
    }

    fn update(term: &mut Terminal, state: &mut RenderState) {
        state.update(term).unwrap();
    }

    #[test]
    fn cursor_default_uses_configured_style() {
        let mut term = term();
        term.screen_mut().cursor.cursor_style = TerminalCursorStyle::Bar;
        term.modes.set(Mode::CursorBlinking, true);

        let mut state = RenderState::new();
        update(&mut term, &mut state);

        assert_eq!(style(&state, o(false, true, true)), Some(Style::Bar));
        assert_eq!(
            style(&state, o(false, false, true)),
            Some(Style::BlockHollow)
        );
        assert_eq!(
            style(&state, o(false, false, false)),
            Some(Style::BlockHollow)
        );
        assert_eq!(style(&state, o(false, true, false)), None);
    }

    #[test]
    fn cursor_blinking_disabled() {
        let mut term = term();
        term.screen_mut().cursor.cursor_style = TerminalCursorStyle::Bar;
        term.modes.set(Mode::CursorBlinking, false);

        let mut state = RenderState::new();
        update(&mut term, &mut state);

        assert_eq!(style(&state, o(false, true, true)), Some(Style::Bar));
        assert_eq!(style(&state, o(false, true, false)), Some(Style::Bar));
        assert_eq!(
            style(&state, o(false, false, true)),
            Some(Style::BlockHollow)
        );
        assert_eq!(
            style(&state, o(false, false, false)),
            Some(Style::BlockHollow)
        );
    }

    #[test]
    fn cursor_explicitly_not_visible() {
        let mut term = term();
        term.screen_mut().cursor.cursor_style = TerminalCursorStyle::Bar;
        term.modes.set(Mode::CursorVisible, false);
        term.modes.set(Mode::CursorBlinking, false);

        let mut state = RenderState::new();
        update(&mut term, &mut state);

        for focused in [true, false] {
            for blink in [true, false] {
                assert_eq!(style(&state, o(false, focused, blink)), None);
            }
        }
    }

    #[test]
    fn cursor_always_block_with_preedit() {
        let mut term = term();
        let mut state = RenderState::new();
        update(&mut term, &mut state);

        // In any bool state
        for focused in [true, false] {
            for blink in [true, false] {
                assert_eq!(style(&state, o(true, focused, blink)), Some(Style::Block));
            }
        }

        // If we're scrolled though, then we don't show the cursor.
        for _ in 0..100 {
            term.index().unwrap();
        }
        term.scroll_viewport(ScrollViewport::Top);
        update(&mut term, &mut state);

        // In any bool state
        for focused in [true, false] {
            for blink in [true, false] {
                assert_eq!(style(&state, o(true, focused, blink)), None);
            }
        }
    }

    #[test]
    fn cursor_password_input_is_lock() {
        let mut term = term();
        term.flags.password_input = true;
        let mut state = RenderState::new();
        update(&mut term, &mut state);
        assert_eq!(style(&state, o(false, true, true)), Some(Style::Lock));
        // Preedit still wins.
        assert_eq!(style(&state, o(true, true, true)), Some(Style::Block));
    }
}
