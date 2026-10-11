//! Cursor style decision logic (`renderer/cursor.zig`).

/// Terminal cursor styles (`terminal.CursorStyle`).
// TODO(integration): replace with ghostty_terminal's CursorStyle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TerminalCursorStyle {
    Bar,
    #[default]
    Block,
    BlockHollow,
    Underline,
}

/// The subset of `terminal.RenderState.cursor` that the cursor style
/// decision needs.
// TODO(integration): replace with `&ghostty_terminal::RenderState` (use its
// `cursor` field) once the terminal crate is a dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderStateCursor {
    /// Viewport position of the cursor (`None` if it isn't in the viewport).
    pub viewport: Option<(u16, u16)>,
    /// Whether the terminal is at a password prompt.
    pub password_input: bool,
    /// Cursor visibility via terminal mode.
    pub visible: bool,
    /// Whether the cursor is blinking.
    pub blinking: bool,
    /// The visual style requested by the terminal.
    pub visual_style: TerminalCursorStyle,
}

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
pub fn style(state: &RenderStateCursor, opts: StyleOptions) -> Option<Style> {
    // Note the order of conditionals below is important. It represents
    // a priority system of how we determine what state overrides cursor
    // visibility and style.

    // The cursor must be visible in the viewport to be rendered.
    state.viewport?;

    // If we are in preedit, then we always show the block cursor. We do
    // this even if the cursor is explicitly not visible because it shows
    // an important editing state to the user.
    if opts.preedit {
        return Some(Style::Block);
    }

    // If we're at a password input its always a lock.
    if state.password_input {
        return Some(Style::Lock);
    }

    // If the cursor is explicitly not visible by terminal mode, we don't render.
    if !state.visible {
        return None;
    }

    // If we're not focused, our cursor is always visible so that
    // we can show the hollow box.
    if !opts.focused {
        return Some(Style::BlockHollow);
    }

    // If the cursor is blinking and our blink state is not visible,
    // then we don't show the cursor.
    if state.blinking && !opts.blink_visible {
        return None;
    }

    // Otherwise, we use whatever style the terminal wants.
    Some(Style::from_terminal(state.visual_style))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The Zig tests drive a real Terminal + RenderState; until
    // ghostty-terminal is a dependency these build the cursor state directly
    // with the values the Zig tests produce.
    // TODO(integration): port to use Terminal/RenderState.
    fn state(visible: bool, blinking: bool) -> RenderStateCursor {
        RenderStateCursor {
            viewport: Some((0, 0)),
            password_input: false,
            visible,
            blinking,
            visual_style: TerminalCursorStyle::Bar,
        }
    }

    fn o(preedit: bool, focused: bool, blink_visible: bool) -> StyleOptions {
        StyleOptions {
            preedit,
            focused,
            blink_visible,
        }
    }

    #[test]
    fn cursor_default_uses_configured_style() {
        let s = state(true, true);
        assert_eq!(style(&s, o(false, true, true)), Some(Style::Bar));
        assert_eq!(style(&s, o(false, false, true)), Some(Style::BlockHollow));
        assert_eq!(style(&s, o(false, false, false)), Some(Style::BlockHollow));
        assert_eq!(style(&s, o(false, true, false)), None);
    }

    #[test]
    fn cursor_blinking_disabled() {
        let s = state(true, false);
        assert_eq!(style(&s, o(false, true, true)), Some(Style::Bar));
        assert_eq!(style(&s, o(false, true, false)), Some(Style::Bar));
        assert_eq!(style(&s, o(false, false, true)), Some(Style::BlockHollow));
        assert_eq!(style(&s, o(false, false, false)), Some(Style::BlockHollow));
    }

    #[test]
    fn cursor_explicitly_not_visible() {
        let s = state(false, false);
        for focused in [true, false] {
            for blink in [true, false] {
                assert_eq!(style(&s, o(false, focused, blink)), None);
            }
        }
    }

    #[test]
    fn cursor_always_block_with_preedit() {
        let mut s = state(true, true);
        for focused in [true, false] {
            for blink in [true, false] {
                assert_eq!(style(&s, o(true, focused, blink)), Some(Style::Block));
            }
        }

        // If we're scrolled though, then we don't show the cursor.
        s.viewport = None;
        for focused in [true, false] {
            for blink in [true, false] {
                assert_eq!(style(&s, o(true, focused, blink)), None);
            }
        }
    }

    #[test]
    fn cursor_password_input_is_lock() {
        let mut s = state(false, true);
        s.password_input = true;
        assert_eq!(style(&s, o(false, true, true)), Some(Style::Lock));
        assert_eq!(style(&s, o(true, true, true)), Some(Style::Block));
    }
}
