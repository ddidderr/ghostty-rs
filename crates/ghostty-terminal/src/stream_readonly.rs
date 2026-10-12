//! A stream handler that applies all terminal-state-modifying actions to a
//! [`Terminal`] and ignores everything that would require a response.
//!
//! Port of `ZIG/src/terminal/stream_readonly.zig`.
//!
//! This is a Stream implementation that processes actions against a Terminal
//! and updates the Terminal state. It is called "readonly" because it only
//! processes actions that modify terminal state, while ignoring any actions
//! that require a response (like queries).
//!
//! If you're implementing a terminal emulator that only needs to render
//! output and doesn't need to respond (since it maybe isn't running the
//! actual program), this is the stream type to use. For example, this is
//! ideal for replay tooling, CI logs, PaaS builder output, etc.
use ghostty_vt::{
    ansi::{CursorStyle as AnsiCursorStyle, ModifyKeyFormat, ProtectedMode},
    color::Dynamic as DynamicColor,
    csi,
    kitty::{self, key::SetMode as KittySetMode},
    modes::Mode,
    osc::color::{self as osc_color, Request, Target},
    stream::{self, Action},
};

use crate::{
    cursor::Style as CursorStyle,
    terminal::{
        DeccolmMode,
        Error,
        MouseEvents,
        MouseFormat,
        MouseShiftCapture,
        SwitchScreenMode,
        Terminal,
    },
};

/// The stream type wrapping [`Handler`].
pub type Stream<'a> = stream::Stream<Handler<'a>>;

/// See [`Stream`], which is just the stream wrapper around this.
///
/// This isn't attached directly to Terminal because there is additional
/// state and options we plan to add in the future, such as APC/DCS which
/// don't make sense to me to add to the Terminal directly. Instead, you
/// can call `vt_handler` on Terminal to initialize this handler.
pub struct Handler<'a> {
    /// The terminal state to modify.
    pub terminal: &'a mut Terminal,
}

impl<'a> Handler<'a> {
    pub fn new(terminal: &'a mut Terminal) -> Self {
        Self { terminal }
    }

    fn horizontal_tab(&mut self, count: u16) {
        for _ in 0..count {
            let x = self.terminal.screen().cursor.x;
            self.terminal.horizontal_tab();
            if x == self.terminal.screen().cursor.x {
                break;
            }
        }
    }

    fn horizontal_tab_back(&mut self, count: u16) {
        for _ in 0..count {
            let x = self.terminal.screen().cursor.x;
            self.terminal.horizontal_tab_back();
            if x == self.terminal.screen().cursor.x {
                break;
            }
        }
    }

    fn set_mode(&mut self, mode: Mode, enabled: bool) -> Result<(), Error> {
        // Set the mode on the terminal
        self.terminal.modes.set(mode, enabled);

        // Some modes require additional processing
        match mode {
            Mode::Autorepeat | Mode::ReverseColors => {}

            Mode::Origin => self.terminal.set_cursor_pos(1, 1),

            Mode::EnableLeftAndRightMargin => {
                if !enabled {
                    self.terminal.scrolling_region.left = 0;
                    self.terminal.scrolling_region.right = self.terminal.cols - 1;
                }
            }

            Mode::AltScreenLegacy => self
                .terminal
                .switch_screen_mode(SwitchScreenMode::M47, enabled)?,
            Mode::AltScreen => self
                .terminal
                .switch_screen_mode(SwitchScreenMode::M1047, enabled)?,
            Mode::AltScreenSaveCursorClearEnter => self
                .terminal
                .switch_screen_mode(SwitchScreenMode::M1049, enabled)?,

            Mode::SaveCursor => {
                if enabled {
                    self.terminal.save_cursor();
                } else {
                    self.terminal.restore_cursor();
                }
            }

            Mode::EnableMode3 => {}

            Mode::M132Column => self.terminal.deccolm(if enabled {
                DeccolmMode::Cols132
            } else {
                DeccolmMode::Cols80
            })?,

            Mode::SynchronizedOutput
            | Mode::Linefeed
            | Mode::InBandSizeReports
            | Mode::FocusEvent => {}

            Mode::MouseEventX10 => {
                self.terminal.flags.mouse_event = if enabled {
                    MouseEvents::X10
                } else {
                    MouseEvents::None
                };
            }
            Mode::MouseEventNormal => {
                self.terminal.flags.mouse_event = if enabled {
                    MouseEvents::Normal
                } else {
                    MouseEvents::None
                };
            }
            Mode::MouseEventButton => {
                self.terminal.flags.mouse_event = if enabled {
                    MouseEvents::Button
                } else {
                    MouseEvents::None
                };
            }
            Mode::MouseEventAny => {
                self.terminal.flags.mouse_event = if enabled {
                    MouseEvents::Any
                } else {
                    MouseEvents::None
                };
            }

            Mode::MouseFormatUtf8 => {
                self.terminal.flags.mouse_format = if enabled {
                    MouseFormat::Utf8
                } else {
                    MouseFormat::X10
                };
            }
            Mode::MouseFormatSgr => {
                self.terminal.flags.mouse_format = if enabled {
                    MouseFormat::Sgr
                } else {
                    MouseFormat::X10
                };
            }
            Mode::MouseFormatUrxvt => {
                self.terminal.flags.mouse_format = if enabled {
                    MouseFormat::Urxvt
                } else {
                    MouseFormat::X10
                };
            }
            Mode::MouseFormatSgrPixels => {
                self.terminal.flags.mouse_format = if enabled {
                    MouseFormat::SgrPixels
                } else {
                    MouseFormat::X10
                };
            }

            _ => {}
        }
        Ok(())
    }

    fn color_operation(&mut self, requests: &osc_color::List<'_>) {
        for req in requests.iter() {
            match req {
                Request::Set(set) => match set.target {
                    Target::Palette(i) => {
                        self.terminal.flags.dirty.palette = true;
                        self.terminal.colors.palette.set(i, set.color);
                    }
                    Target::Dynamic(dynamic) => match dynamic {
                        DynamicColor::Foreground => self.terminal.colors.foreground.set(set.color),
                        DynamicColor::Background => self.terminal.colors.background.set(set.color),
                        DynamicColor::Cursor => self.terminal.colors.cursor.set(set.color),
                        _ => {}
                    },
                    Target::Special(_) => {}
                },

                Request::Reset(target) => match target {
                    Target::Palette(i) => {
                        self.terminal.flags.dirty.palette = true;
                        self.terminal.colors.palette.reset(i);
                    }
                    Target::Dynamic(dynamic) => match dynamic {
                        DynamicColor::Foreground => self.terminal.colors.foreground.reset(),
                        DynamicColor::Background => self.terminal.colors.background.reset(),
                        DynamicColor::Cursor => self.terminal.colors.cursor.reset(),
                        _ => {}
                    },
                    Target::Special(_) => {}
                },

                Request::ResetPalette => {
                    let indices: Vec<usize> = self.terminal.colors.palette.mask.iter().collect();
                    for i in indices {
                        self.terminal.flags.dirty.palette = true;
                        self.terminal.colors.palette.reset(i as u8);
                    }
                    self.terminal.colors.palette.mask =
                        ghostty_vt::color::PaletteMask::init_empty();
                }

                Request::Query(_) | Request::ResetSpecial => {}
            }
        }
    }

    fn kitty_color_operation(&mut self, request: &kitty::color::OSC<'_>) {
        use kitty::color::{Kind, Request as KReq, Special};
        for item in request.list.iter() {
            match &item {
                KReq::Set { key, color } => match key {
                    Kind::Palette(palette) => {
                        self.terminal.flags.dirty.palette = true;
                        self.terminal.colors.palette.set(*palette, *color);
                    }
                    Kind::Special(special) => match special {
                        Special::Foreground => self.terminal.colors.foreground.set(*color),
                        Special::Background => self.terminal.colors.background.set(*color),
                        Special::Cursor => self.terminal.colors.cursor.set(*color),
                        _ => {}
                    },
                },
                KReq::Reset(key) => match key {
                    Kind::Palette(palette) => {
                        self.terminal.flags.dirty.palette = true;
                        self.terminal.colors.palette.reset(*palette);
                    }
                    Kind::Special(special) => match special {
                        Special::Foreground => self.terminal.colors.foreground.reset(),
                        Special::Background => self.terminal.colors.background.reset(),
                        Special::Cursor => self.terminal.colors.cursor.reset(),
                        _ => {}
                    },
                },
                KReq::Query(_) => {}
            }
        }
    }
}

impl stream::Handler for Handler<'_> {
    type Error = Error;

    fn vt(&mut self, action: Action<'_>) -> Result<(), Error> {
        let t = &mut *self.terminal;
        match action {
            Action::Print(v) => t.print(v.cp)?,
            Action::PrintRepeat(v) => t.print_repeat(v)?,
            Action::Backspace => t.backspace(),
            Action::CarriageReturn => t.carriage_return(),
            Action::Linefeed => t.linefeed()?,
            Action::Index => t.index()?,
            Action::NextLine => {
                t.index()?;
                t.carriage_return();
            }
            Action::ReverseIndex => t.reverse_index(),
            Action::CursorUp(v) => t.cursor_up(usize::from(v.value)),
            Action::CursorDown(v) => t.cursor_down(usize::from(v.value)),
            Action::CursorLeft(v) => t.cursor_left(usize::from(v.value)),
            Action::CursorRight(v) => t.cursor_right(usize::from(v.value)),
            Action::CursorPos(v) => t.set_cursor_pos(usize::from(v.row), usize::from(v.col)),
            Action::CursorCol(v) => {
                let row = usize::from(t.screen().cursor.y) + 1;
                t.set_cursor_pos(row, usize::from(v.value));
            }
            Action::CursorRow(v) => {
                let col = usize::from(t.screen().cursor.x) + 1;
                t.set_cursor_pos(usize::from(v.value), col);
            }
            Action::CursorColRelative(v) => {
                let row = usize::from(t.screen().cursor.y) + 1;
                let col =
                    (usize::from(t.screen().cursor.x) + 1).saturating_add(usize::from(v.value));
                t.set_cursor_pos(row, col);
            }
            Action::CursorRowRelative(v) => {
                let row =
                    (usize::from(t.screen().cursor.y) + 1).saturating_add(usize::from(v.value));
                let col = usize::from(t.screen().cursor.x) + 1;
                t.set_cursor_pos(row, col);
            }
            Action::CursorStyle(v) => {
                let blink = match v {
                    AnsiCursorStyle::Default
                    | AnsiCursorStyle::SteadyBlock
                    | AnsiCursorStyle::SteadyBar
                    | AnsiCursorStyle::SteadyUnderline => false,
                    AnsiCursorStyle::BlinkingBlock
                    | AnsiCursorStyle::BlinkingBar
                    | AnsiCursorStyle::BlinkingUnderline => true,
                };
                let style = match v {
                    AnsiCursorStyle::Default
                    | AnsiCursorStyle::BlinkingBlock
                    | AnsiCursorStyle::SteadyBlock => CursorStyle::Block,
                    AnsiCursorStyle::BlinkingBar | AnsiCursorStyle::SteadyBar => CursorStyle::Bar,
                    AnsiCursorStyle::BlinkingUnderline | AnsiCursorStyle::SteadyUnderline => {
                        CursorStyle::Underline
                    }
                };
                t.modes.set(Mode::CursorBlinking, blink);
                t.screen_mut().cursor.cursor_style = style;
            }
            Action::EraseDisplayBelow(p) => t.erase_display(csi::EraseDisplay::Below, p),
            Action::EraseDisplayAbove(p) => t.erase_display(csi::EraseDisplay::Above, p),
            Action::EraseDisplayComplete(p) => t.erase_display(csi::EraseDisplay::Complete, p),
            Action::EraseDisplayScrollback(p) => t.erase_display(csi::EraseDisplay::Scrollback, p),
            Action::EraseDisplayScrollComplete(p) => {
                t.erase_display(csi::EraseDisplay::ScrollComplete, p);
            }
            Action::EraseLineRight(p) => t.erase_line(csi::EraseLine::Right, p),
            Action::EraseLineLeft(p) => t.erase_line(csi::EraseLine::Left, p),
            Action::EraseLineComplete(p) => t.erase_line(csi::EraseLine::Complete, p),
            Action::EraseLineRightUnlessPendingWrap(p) => {
                t.erase_line(csi::EraseLine::RightUnlessPendingWrap, p);
            }
            Action::DeleteChars(v) => t.delete_chars(v),
            Action::EraseChars(v) => t.erase_chars(v),
            Action::InsertLines(v) => t.insert_lines(v),
            Action::InsertBlanks(v) => t.insert_blanks(v),
            Action::DeleteLines(v) => t.delete_lines(v),
            Action::ScrollUp(v) => t.scroll_up(v)?,
            Action::ScrollDown(v) => t.scroll_down(v),
            Action::HorizontalTab(v) => self.horizontal_tab(v),
            Action::HorizontalTabBack(v) => self.horizontal_tab_back(v),
            Action::TabClearCurrent => t.tab_clear(csi::TabClear::Current),
            Action::TabClearAll => t.tab_clear(csi::TabClear::All),
            Action::TabSet => t.tab_set(),
            Action::TabReset => t.tab_reset(),
            Action::SetMode(v) => self.set_mode(v.mode, true)?,
            Action::ResetMode(v) => self.set_mode(v.mode, false)?,
            Action::SaveMode(v) => t.modes.save(v.mode),
            Action::RestoreMode(v) => {
                let enabled = t.modes.restore(v.mode);
                self.set_mode(v.mode, enabled)?;
            }
            Action::TopAndBottomMargin(v) => {
                t.set_top_and_bottom_margin(usize::from(v.top_left), usize::from(v.bottom_right));
            }
            Action::LeftAndRightMargin(v) => {
                t.set_left_and_right_margin(usize::from(v.top_left), usize::from(v.bottom_right));
            }
            Action::LeftAndRightMarginAmbiguous => {
                if t.modes.get(Mode::EnableLeftAndRightMargin) {
                    t.set_left_and_right_margin(0, 0);
                } else {
                    t.save_cursor();
                }
            }
            Action::SaveCursor => t.save_cursor(),
            Action::RestoreCursor => t.restore_cursor(),
            Action::InvokeCharset(v) => t.invoke_charset(v.bank, v.charset, v.locking),
            Action::ConfigureCharset(v) => t.configure_charset(v.slot, v.charset),
            Action::SetAttribute(v) => {
                if !matches!(v, ghostty_vt::sgr::Attribute::Unknown(_)) {
                    let _ = t.set_attribute(v);
                }
            }
            Action::ProtectedModeOff => t.set_protected_mode(ProtectedMode::Off),
            Action::ProtectedModeIso => t.set_protected_mode(ProtectedMode::Iso),
            Action::ProtectedModeDec => t.set_protected_mode(ProtectedMode::Dec),
            Action::MouseShiftCapture(v) => {
                t.flags.mouse_shift_capture = if v {
                    MouseShiftCapture::True
                } else {
                    MouseShiftCapture::False
                };
            }
            Action::KittyKeyboardPush(v) => t.screen_mut().kitty_keyboard.push(v.flags),
            Action::KittyKeyboardPop(v) => t.screen_mut().kitty_keyboard.pop(usize::from(v)),
            Action::KittyKeyboardSet(v) => {
                t.screen_mut()
                    .kitty_keyboard
                    .set(KittySetMode::Set, v.flags);
            }
            Action::KittyKeyboardSetOr(v) => {
                t.screen_mut().kitty_keyboard.set(KittySetMode::Or, v.flags);
            }
            Action::KittyKeyboardSetNot(v) => {
                t.screen_mut()
                    .kitty_keyboard
                    .set(KittySetMode::Not, v.flags);
            }
            Action::ModifyKeyFormat(v) => {
                t.flags.modify_other_keys_2 = false;
                if v == ModifyKeyFormat::OtherKeysNumeric {
                    t.flags.modify_other_keys_2 = true;
                }
            }
            Action::ActiveStatusDisplay(v) => t.status_display = v,
            Action::Decaln => t.decaln()?,
            Action::FullReset => t.full_reset(),
            Action::StartHyperlink(v) => t.screen_mut().start_hyperlink(v.uri, v.id)?,
            Action::EndHyperlink => t.screen_mut().end_hyperlink(),
            Action::SemanticPrompt(v) => t.semantic_prompt(v)?,
            Action::MouseShape(v) => t.mouse_shape = v,
            Action::ColorOperation(v) => self.color_operation(v.requests),
            Action::KittyColorReport(v) => self.kitty_color_operation(v),

            // No supported DCS commands have any terminal-modifying effects,
            // but they may in the future. For now we just ignore it.
            Action::DcsHook(_) | Action::DcsPut(_) | Action::DcsUnhook => {}

            // APC can modify terminal state (Kitty graphics) but we don't
            // currently support it in the readonly stream.
            Action::ApcStart | Action::ApcEnd | Action::ApcPut(_) => {}

            // Have no terminal-modifying effect
            Action::Bell
            | Action::Enquiry
            | Action::RequestMode(_)
            | Action::RequestModeUnknown(_)
            | Action::SizeReport(_)
            | Action::Xtversion
            | Action::DeviceAttributes(_)
            | Action::DeviceStatus(_)
            | Action::KittyKeyboardQuery
            | Action::WindowTitle(_)
            | Action::ReportPwd(_)
            | Action::ShowDesktopNotification(_)
            | Action::ProgressReport(_)
            | Action::ClipboardContents(_)
            | Action::TitlePush(_)
            | Action::TitlePop(_) => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ghostty_vt::osc::semantic_prompt::Redraw;

    use super::*;
    use crate::{page::SemanticContent, screen_set::Key as ScreenKey, terminal::Options};

    fn term(cols: u16, rows: u16) -> Terminal {
        Terminal::new(Options::new(cols, rows)).unwrap()
    }

    #[test]
    fn basic_print() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        s.next_slice(b"Hello").unwrap();
        assert_eq!(5, s.handler.terminal.screen().cursor.x);
        assert_eq!(0, s.handler.terminal.screen().cursor.y);

        assert_eq!("Hello", s.handler.terminal.plain_string().unwrap());
    }

    #[test]
    fn cursor_movement() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Move cursor using escape sequences
        s.next_slice(b"Hello\x1B[1;1H").unwrap();
        assert_eq!(0, s.handler.terminal.screen().cursor.x);
        assert_eq!(0, s.handler.terminal.screen().cursor.y);

        // Move to position 2,3
        s.next_slice(b"\x1B[2;3H").unwrap();
        assert_eq!(2, s.handler.terminal.screen().cursor.x);
        assert_eq!(1, s.handler.terminal.screen().cursor.y);
    }

    #[test]
    fn erase_operations() {
        let mut t = term(20, 10);
        let mut s = t.vt_stream();

        // Print some text
        s.next_slice(b"Hello World").unwrap();
        assert_eq!(11, s.handler.terminal.screen().cursor.x);
        assert_eq!(0, s.handler.terminal.screen().cursor.y);

        // Move cursor to position 1,6 and erase from cursor to end of line
        s.next_slice(b"\x1B[1;6H").unwrap();
        s.next_slice(b"\x1B[K").unwrap();

        assert_eq!("Hello", s.handler.terminal.plain_string().unwrap());
    }

    #[test]
    fn tabs() {
        let mut t = term(80, 10);
        let mut s = t.vt_stream();

        s.next_slice(b"A\tB").unwrap();
        assert_eq!(9, s.handler.terminal.screen().cursor.x);
        assert_eq!("A       B", s.handler.terminal.plain_string().unwrap());
    }

    #[test]
    fn modes() {
        let mut t = term(80, 24);
        let mut s = t.vt_stream();

        // Test wraparound mode
        assert!(s.handler.terminal.modes.get(Mode::Wraparound));
        s.next_slice(b"\x1B[?7l").unwrap(); // Disable wraparound
        assert!(!s.handler.terminal.modes.get(Mode::Wraparound));
        s.next_slice(b"\x1B[?7h").unwrap(); // Enable wraparound
        assert!(s.handler.terminal.modes.get(Mode::Wraparound));
    }

    #[test]
    fn scrolling_regions() {
        let mut t = term(80, 24);
        let mut s = t.vt_stream();

        // Set scrolling region from line 5 to 20
        s.next_slice(b"\x1B[5;20r").unwrap();
        let sr = s.handler.terminal.scrolling_region;
        assert_eq!(4, sr.top);
        assert_eq!(19, sr.bottom);
        assert_eq!(0, sr.left);
        assert_eq!(79, sr.right);
    }

    #[test]
    fn charsets() {
        let mut t = term(80, 24);
        let mut s = t.vt_stream();

        // Configure G0 as DEC special graphics
        s.next_slice(b"\x1B(0").unwrap();
        s.next_slice(b"`").unwrap(); // Should print diamond character

        assert_eq!("\u{25c6}", s.handler.terminal.plain_string().unwrap());
    }

    #[test]
    fn alt_screen() {
        let mut t = term(10, 5);
        let mut s = t.vt_stream();

        // Write to primary screen
        s.next_slice(b"Primary").unwrap();
        assert_eq!(ScreenKey::Primary, s.handler.terminal.screens.active_key);

        // Switch to alt screen
        s.next_slice(b"\x1B[?1049h").unwrap();
        assert_eq!(ScreenKey::Alternate, s.handler.terminal.screens.active_key);

        // Write to alt screen
        s.next_slice(b"Alt").unwrap();

        // Switch back to primary
        s.next_slice(b"\x1B[?1049l").unwrap();
        assert_eq!(ScreenKey::Primary, s.handler.terminal.screens.active_key);

        assert_eq!("Primary", s.handler.terminal.plain_string().unwrap());
    }

    #[test]
    fn cursor_save_and_restore() {
        let mut t = term(80, 24);
        let mut s = t.vt_stream();

        // Move cursor to 10,15
        s.next_slice(b"\x1B[10;15H").unwrap();
        assert_eq!(14, s.handler.terminal.screen().cursor.x);
        assert_eq!(9, s.handler.terminal.screen().cursor.y);

        // Save cursor
        s.next_slice(b"\x1B7").unwrap();

        // Move cursor elsewhere
        s.next_slice(b"\x1B[1;1H").unwrap();
        assert_eq!(0, s.handler.terminal.screen().cursor.x);
        assert_eq!(0, s.handler.terminal.screen().cursor.y);

        // Restore cursor
        s.next_slice(b"\x1B8").unwrap();
        assert_eq!(14, s.handler.terminal.screen().cursor.x);
        assert_eq!(9, s.handler.terminal.screen().cursor.y);
    }

    #[test]
    fn attributes() {
        let mut t = term(80, 24);
        let mut s = t.vt_stream();

        // Set bold and write text
        s.next_slice(b"\x1B[1mBold\x1B[0m").unwrap();

        // Verify we can write attributes - just check the string was written
        assert_eq!("Bold", s.handler.terminal.plain_string().unwrap());
    }

    #[test]
    fn decaln_screen_alignment() {
        let mut t = term(10, 3);
        let mut s = t.vt_stream();

        // Run DECALN
        s.next_slice(b"\x1B#8").unwrap();

        // Verify entire screen is filled with 'E'
        assert_eq!(
            "EEEEEEEEEE\nEEEEEEEEEE\nEEEEEEEEEE",
            s.handler.terminal.plain_string().unwrap()
        );

        // Cursor should be at 1,1
        assert_eq!(0, s.handler.terminal.screen().cursor.x);
        assert_eq!(0, s.handler.terminal.screen().cursor.y);
    }

    #[test]
    fn full_reset() {
        let mut t = term(80, 24);
        let mut s = t.vt_stream();

        // Make some changes
        s.next_slice(b"Hello").unwrap();
        s.next_slice(b"\x1B[10;20H").unwrap();
        s.next_slice(b"\x1B[5;20r").unwrap(); // Set scroll region
        s.next_slice(b"\x1B[?7l").unwrap(); // Disable wraparound

        // Full reset
        s.next_slice(b"\x1Bc").unwrap();

        // Verify reset state
        let t = &*s.handler.terminal;
        assert_eq!(0, t.screen().cursor.x);
        assert_eq!(0, t.screen().cursor.y);
        assert_eq!(0, t.scrolling_region.top);
        assert_eq!(23, t.scrolling_region.bottom);
        assert!(t.modes.get(Mode::Wraparound));
    }

    #[test]
    fn ignores_query_actions() {
        let mut t = term(80, 24);
        let mut s = t.vt_stream();

        // These should be ignored without error
        s.next_slice(b"\x1B[c").unwrap(); // Device attributes
        s.next_slice(b"\x1B[5n").unwrap(); // Device status report
        s.next_slice(b"\x1B[6n").unwrap(); // Cursor position report

        // Terminal should still be functional
        s.next_slice(b"Test").unwrap();
        assert_eq!("Test", s.handler.terminal.plain_string().unwrap());
    }

    #[test]
    fn osc_4_set_and_reset_palette() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Save default color
        let default_color_0 = s.handler.terminal.colors.palette.original[0];

        // Set color 0 to red
        s.next_slice(b"\x1b]4;0;rgb:ff/00/00\x1b\\").unwrap();
        let p = &s.handler.terminal.colors.palette;
        assert_eq!(0xff, p.current[0].r);
        assert_eq!(0x00, p.current[0].g);
        assert_eq!(0x00, p.current[0].b);
        assert!(p.mask.is_set(0));

        // Reset color 0
        s.next_slice(b"\x1b]104;0\x1b\\").unwrap();
        let p = &s.handler.terminal.colors.palette;
        assert_eq!(default_color_0, p.current[0]);
        assert!(!p.mask.is_set(0));
    }

    #[test]
    fn osc_104_reset_all_palette_colors() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Set multiple colors
        s.next_slice(b"\x1b]4;0;rgb:ff/00/00\x1b\\").unwrap();
        s.next_slice(b"\x1b]4;1;rgb:00/ff/00\x1b\\").unwrap();
        s.next_slice(b"\x1b]4;2;rgb:00/00/ff\x1b\\").unwrap();
        let p = &s.handler.terminal.colors.palette;
        assert!(p.mask.is_set(0));
        assert!(p.mask.is_set(1));
        assert!(p.mask.is_set(2));

        // Reset all palette colors
        s.next_slice(b"\x1b]104\x1b\\").unwrap();
        let p = &s.handler.terminal.colors.palette;
        assert_eq!(p.original[0], p.current[0]);
        assert_eq!(p.original[1], p.current[1]);
        assert_eq!(p.original[2], p.current[2]);
        assert!(!p.mask.is_set(0));
        assert!(!p.mask.is_set(1));
        assert!(!p.mask.is_set(2));
    }

    #[test]
    fn osc_10_set_and_reset_foreground_color() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Initially unset
        assert!(s.handler.terminal.colors.foreground.get().is_none());

        // Set foreground to red
        s.next_slice(b"\x1b]10;rgb:ff/00/00\x1b\\").unwrap();
        let fg = s.handler.terminal.colors.foreground.get().unwrap();
        assert_eq!(0xff, fg.r);
        assert_eq!(0x00, fg.g);
        assert_eq!(0x00, fg.b);

        // Reset foreground
        s.next_slice(b"\x1b]110\x1b\\").unwrap();
        assert!(s.handler.terminal.colors.foreground.get().is_none());
    }

    #[test]
    fn osc_11_set_and_reset_background_color() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Set background to green
        s.next_slice(b"\x1b]11;rgb:00/ff/00\x1b\\").unwrap();
        let bg = s.handler.terminal.colors.background.get().unwrap();
        assert_eq!(0x00, bg.r);
        assert_eq!(0xff, bg.g);
        assert_eq!(0x00, bg.b);

        // Reset background
        s.next_slice(b"\x1b]111\x1b\\").unwrap();
        assert!(s.handler.terminal.colors.background.get().is_none());
    }

    #[test]
    fn osc_12_set_and_reset_cursor_color() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Set cursor to blue
        s.next_slice(b"\x1b]12;rgb:00/00/ff\x1b\\").unwrap();
        let cursor = s.handler.terminal.colors.cursor.get().unwrap();
        assert_eq!(0x00, cursor.r);
        assert_eq!(0x00, cursor.g);
        assert_eq!(0xff, cursor.b);

        // Reset cursor
        s.next_slice(b"\x1b]112\x1b\\").unwrap();
        // After reset, cursor might be null (using default)
    }

    #[test]
    fn kitty_color_protocol_set_palette() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Set palette color 5 to magenta using kitty protocol
        s.next_slice(b"\x1b]21;5=rgb:ff/00/ff\x1b\\").unwrap();
        let t = &*s.handler.terminal;
        assert_eq!(0xff, t.colors.palette.current[5].r);
        assert_eq!(0x00, t.colors.palette.current[5].g);
        assert_eq!(0xff, t.colors.palette.current[5].b);
        assert!(t.colors.palette.mask.is_set(5));
        assert!(t.flags.dirty.palette);
    }

    #[test]
    fn kitty_color_protocol_reset_palette() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Set and then reset palette color
        let original = s.handler.terminal.colors.palette.original[7];
        s.next_slice(b"\x1b]21;7=rgb:aa/bb/cc\x1b\\").unwrap();
        assert!(s.handler.terminal.colors.palette.mask.is_set(7));

        s.next_slice(b"\x1b]21;7=\x1b\\").unwrap();
        let p = &s.handler.terminal.colors.palette;
        assert_eq!(original, p.current[7]);
        assert!(!p.mask.is_set(7));
    }

    #[test]
    fn kitty_color_protocol_set_foreground() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Set foreground using kitty protocol
        s.next_slice(b"\x1b]21;foreground=rgb:12/34/56\x1b\\")
            .unwrap();
        let fg = s.handler.terminal.colors.foreground.get().unwrap();
        assert_eq!(0x12, fg.r);
        assert_eq!(0x34, fg.g);
        assert_eq!(0x56, fg.b);
    }

    #[test]
    fn kitty_color_protocol_set_background() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Set background using kitty protocol
        s.next_slice(b"\x1b]21;background=rgb:78/9a/bc\x1b\\")
            .unwrap();
        let bg = s.handler.terminal.colors.background.get().unwrap();
        assert_eq!(0x78, bg.r);
        assert_eq!(0x9a, bg.g);
        assert_eq!(0xbc, bg.b);
    }

    #[test]
    fn kitty_color_protocol_set_cursor() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Set cursor using kitty protocol
        s.next_slice(b"\x1b]21;cursor=rgb:de/f0/12\x1b\\").unwrap();
        let cursor = s.handler.terminal.colors.cursor.get().unwrap();
        assert_eq!(0xde, cursor.r);
        assert_eq!(0xf0, cursor.g);
        assert_eq!(0x12, cursor.b);
    }

    #[test]
    fn kitty_color_protocol_reset_foreground() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Set and reset foreground
        s.next_slice(b"\x1b]21;foreground=rgb:11/22/33\x1b\\")
            .unwrap();
        assert!(s.handler.terminal.colors.foreground.get().is_some());

        s.next_slice(b"\x1b]21;foreground=\x1b\\").unwrap();
        // After reset, should be unset
        assert!(s.handler.terminal.colors.foreground.get().is_none());
    }

    #[test]
    fn palette_dirty_flag_set_on_color_change() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Clear dirty flag
        s.handler.terminal.flags.dirty.palette = false;

        // Setting palette color should set dirty flag
        s.next_slice(b"\x1b]4;0;rgb:ff/00/00\x1b\\").unwrap();
        assert!(s.handler.terminal.flags.dirty.palette);

        // Clear and test reset
        s.handler.terminal.flags.dirty.palette = false;
        s.next_slice(b"\x1b]104;0\x1b\\").unwrap();
        assert!(s.handler.terminal.flags.dirty.palette);

        // Clear and test kitty protocol
        s.handler.terminal.flags.dirty.palette = false;
        s.next_slice(b"\x1b]21;1=rgb:00/ff/00\x1b\\").unwrap();
        assert!(s.handler.terminal.flags.dirty.palette);
    }

    #[test]
    fn semantic_prompt_fresh_line() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        s.next_slice(b"Hello").unwrap();
        s.next_slice(b"\x1b]133;L\x07").unwrap();
        assert_eq!(0, s.handler.terminal.screen().cursor.x);
        assert_eq!(1, s.handler.terminal.screen().cursor.y);
    }

    #[test]
    fn semantic_prompt_fresh_line_new_prompt() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Write some text and then send OSC 133;A (fresh_line_new_prompt)
        s.next_slice(b"Hello").unwrap();
        s.next_slice(b"\x1b]133;A\x07").unwrap();

        // Should do a fresh line (carriage return + index)
        assert_eq!(0, s.handler.terminal.screen().cursor.x);
        assert_eq!(1, s.handler.terminal.screen().cursor.y);

        // Should set cursor semantic_content to prompt
        assert_eq!(
            SemanticContent::Prompt,
            s.handler.terminal.screen().cursor.semantic_content
        );

        // Test with redraw option
        s.next_slice(b"prompt$ ").unwrap();
        s.next_slice(b"\x1b]133;A;redraw=1\x07").unwrap();
        assert_eq!(Redraw::True, s.handler.terminal.flags.shell_redraws_prompt);
    }

    #[test]
    fn semantic_prompt_end_of_input_then_start_output() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Write some text and then send OSC 133;A (fresh_line_new_prompt)
        s.next_slice(b"Hello").unwrap();
        s.next_slice(b"\x1b]133;A\x07").unwrap();
        s.next_slice(b"prompt$ ").unwrap();
        s.next_slice(b"\x1b]133;B\x07").unwrap();
        assert_eq!(
            SemanticContent::Input,
            s.handler.terminal.screen().cursor.semantic_content
        );
        s.next_slice(b"\x1b]133;C\x07").unwrap();
        assert_eq!(
            SemanticContent::Output,
            s.handler.terminal.screen().cursor.semantic_content
        );
    }

    #[test]
    fn semantic_prompt_prompt_start() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Write some text
        s.next_slice(b"Hello").unwrap();

        // OSC 133;P marks the start of a prompt (without fresh line behavior)
        s.next_slice(b"\x1b]133;P\x07").unwrap();
        let scr = s.handler.terminal.screen();
        assert_eq!(SemanticContent::Prompt, scr.cursor.semantic_content);
        assert_eq!(5, scr.cursor.x);
        assert_eq!(0, scr.cursor.y);
    }

    #[test]
    fn semantic_prompt_new_command() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Write some text
        s.next_slice(b"Hello").unwrap();
        s.next_slice(b"\x1b]133;N\x07").unwrap();

        // Should behave like fresh_line_new_prompt - cursor moves to column 0
        // on next line since we had content
        let scr = s.handler.terminal.screen();
        assert_eq!(0, scr.cursor.x);
        assert_eq!(1, scr.cursor.y);
        assert_eq!(SemanticContent::Prompt, scr.cursor.semantic_content);
    }

    #[test]
    fn semantic_prompt_new_command_at_column_zero() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // OSC 133;N when already at column 0 should stay on same line
        s.next_slice(b"\x1b]133;N\x07").unwrap();
        let scr = s.handler.terminal.screen();
        assert_eq!(0, scr.cursor.x);
        assert_eq!(0, scr.cursor.y);
        assert_eq!(SemanticContent::Prompt, scr.cursor.semantic_content);
    }

    #[test]
    fn semantic_prompt_end_prompt_start_input_terminate_eol_clears_on_linefeed() {
        let mut t = term(10, 10);
        let mut s = t.vt_stream();

        // Set input terminated by EOL
        s.next_slice(b"\x1b]133;I\x07").unwrap();
        assert_eq!(
            SemanticContent::Input,
            s.handler.terminal.screen().cursor.semantic_content
        );

        // Linefeed should reset semantic content to output
        s.next_slice(b"\n").unwrap();
        assert_eq!(
            SemanticContent::Output,
            s.handler.terminal.screen().cursor.semantic_content
        );
    }

    #[test]
    fn stream_csi_w_with_intermediate_but_no_params() {
        // Regression test from AFL++ crash. CSI ? W without
        // parameters caused an out-of-bounds access on input.params[0].
        let mut o = Options::new(80, 24);
        o.max_scrollback = 100;
        let mut t = Terminal::new(o).unwrap();
        let mut s = t.vt_stream();

        s.next_slice(b"\x1b[?W").unwrap();
    }

    // ---- Regressions found by the Zig-vs-Rust differential fuzzer
    // (tools/difftest, docs/rewrite/notes/difftest-DF.md). Expected values
    // come from the Zig reference driver.

    fn screen_row_wrap(t: &Terminal, y: u32) -> bool {
        use crate::point::{Coordinate, Point};
        let pages = &t.screen().pages;
        let pin = pages
            .pin(Point::Screen(Coordinate { x: 0, y }))
            .expect("row exists");
        pin.row_and_cell(pages).expect("row").0.wrap()
    }

    fn cell_cps(t: &Terminal, row: u32) -> Vec<u32> {
        use crate::point::{Coordinate, Point};
        (0..t.cols)
            .map(|x| {
                t.screen()
                    .pages
                    .get_cell(Point::Active(Coordinate { x, y: row }))
                    .map_or(0, crate::page::Cell::codepoint)
            })
            .collect()
    }

    #[test]
    fn difftest_invalid_utf8_overlong_swallows_trail_byte() {
        // Zig's SIMD path (utfcpp replace_invalid) emits ONE U+FFFD for an
        // overlong lead plus its continuation byte.
        let mut t = term(10, 2);
        t.vt_stream().next_slice(b"\xc0\xa9").unwrap();
        assert_eq!(1, t.screen().cursor.x);
        assert_eq!(0xFFFD, cell_cps(&t, 0)[0]);
    }

    #[test]
    fn difftest_invalid_utf8_truncated_then_control() {
        // e4 a9 followed by a non-trail byte: the truncated sequence is a
        // single replacement and the following byte decodes normally.
        let mut t = term(10, 2);
        t.vt_stream().next_slice(b"\xe4\xa9A\xf0\x9f").unwrap();
        let cps = cell_cps(&t, 0);
        assert_eq!(&cps[..2], &[0xFFFD, 'A' as u32]);
    }

    #[test]
    fn difftest_erase_scrollback_keeps_stale_row_flags() {
        // Zig's eraseRows swaps rows and leaves the stale `wrap` flag in the
        // leftover rows, which is visible once the page regrows.
        let mut t = term(1, 1);
        t.vt_stream().next_slice(b"xt\n\x1b[3Jex").unwrap();
        assert!(screen_row_wrap(&t, 0));
        assert!(!screen_row_wrap(&t, 1));
    }

    #[test]
    fn difftest_grapheme_wrap_with_margins_moves_prev_cell() {
        // Devanagari conjunct widening the cell at the last column while a
        // left/right margin keeps the wrap from scrolling that cell: Zig moves
        // the grapheme from the original cell pointer (not `up(1)`).
        use crate::point::{Coordinate, Point};
        let mut t = term(20, 8);
        t.vt_stream()
            .next_slice("\x1b[?2027h\x1b[?69h\x1b[;10s\x1b[50;41H\u{915}\u{94d}\u{937}".as_bytes())
            .unwrap();
        let pages = &t.screen().pages;
        let pin = pages.pin(Point::Active(Coordinate { x: 0, y: 7 })).unwrap();
        let (_, cell) = pin.row_and_cell(pages).unwrap();
        assert_eq!(cell.codepoint(), 0x915);
        assert_eq!(crate::page::Wide::Wide, cell.wide());
        assert_eq!(Some(&[0x94d, 0x937][..]), pin.grapheme(pages));
    }

    /// Upstream (Zig) bug, deliberately NOT replicated: Zig's
    /// `Terminal.print` grapheme path keeps `prev.cell` across a page
    /// reallocation (hyperlink/string capacity growth) and appends the
    /// grapheme into freed memory, silently losing U+FE0F here. Rust
    /// addresses cells by index and keeps it. See difftest-DF.md.
    #[test]
    fn difftest_upstream_uaf_grapheme_survives_page_growth() {
        let mut t = Terminal::new(Options {
            max_scrollback: 1,
            ..Options::new(5, 30)
        })
        .unwrap();
        let input = b"\x1b\x5d\x38\x3b\x3b\x6c\x07\x35\x36\x37\x38\x39\x1b\x5b\x38\x65\x5f\x2c\xba\x9f\x87\xb8\x80\xbc\xbb\x1b\x5b\x35\x32\x72\x54\x68\x65\x20\x71\x75\x69\x63\x6b\x20\x62\x72\x6f\x77\x6e\x20\x66\x6f\x78\x1b\x5c\x61\x20\x40\x1b\x5b\x53\x1b\x4b\x30\x31\x32\x33\x34\x35\x36\x37\x31\x53\x1b\x73\x5d\x5b\x32\x6d\x9b\xff\x80\x80\x1b\x5b\x3f\x32\x30\x32\x37\x68\x1b\x5b\x3b\x33\x48\xf0\xc0\x5c\x77\x70\x5e\x6f\x20\x70\xe2\x9d\xa4\xef\xb8\x8f";
        t.vt_stream().next_slice(input).unwrap();
        assert!(t.plain_string().unwrap().contains("\u{2764}\u{fe0f}"));
    }
}
