//! tmux format-string variables and output parsing (port of `tmux/output.zig`).
//!
//! The Zig version builds a struct type at comptime from a variable list. In
//! Rust a [`FormatValues`] holds the parsed values in the same order, with
//! typed accessors.
#![allow(
    clippy::assert_is_empty,
    clippy::assigning_clones,
    clippy::collapsible_if,
    clippy::enum_glob_use,
    clippy::manual_assert_eq,
    clippy::needless_pass_by_value,
    clippy::single_match_else,
    clippy::type_complexity,
    clippy::unnecessary_wraps
)]

/// Errors from [`parse_format_struct`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// Fewer entries than variables.
    #[error("missing entry")]
    MissingEntry,
    /// More entries than variables.
    #[error("extra entry")]
    ExtraEntry,
    /// A value failed to parse.
    #[error("format error")]
    FormatError,
}

/// Errors from [`Variable::parse`] (mirrors the Zig `parseInt` error set plus
/// `FormatError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VariableError {
    /// Missing/incorrect id prefix.
    #[error("format error")]
    FormatError,
    /// Non-digit character (or empty).
    #[error("invalid character")]
    InvalidCharacter,
    /// Value doesn't fit in `usize` (or is negative).
    #[error("overflow")]
    Overflow,
}

/// A parsed variable value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Boolean flag (`"1"` is true).
    Bool(bool),
    /// Unsigned integer / id.
    Usize(usize),
    /// Raw string.
    Str(String),
}

fn parse_int(s: &str) -> Result<usize, VariableError> {
    ghostty_zigfmt::parse_int::<usize>(s.as_bytes(), 10).map_err(|e| match e {
        ghostty_zigfmt::ParseIntError::Overflow => VariableError::Overflow,
        ghostty_zigfmt::ParseIntError::InvalidCharacter => VariableError::InvalidCharacter,
    })
}

fn parse_prefixed(prefix: u8, value: &str) -> Result<usize, VariableError> {
    let b = value.as_bytes();
    if b.len() >= 2 && b[0] == prefix {
        parse_int(&value[1..])
    } else {
        Err(VariableError::FormatError)
    }
}

/// Possible variables in a tmux format string that we support.
///
/// Tmux supports a large number of variables, but we only implement
/// a subset of them here that are relevant to the use case of implementing
/// control mode for terminal emulators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variable {
    /// 1 if pane is in alternate screen.
    AlternateOn,
    /// Saved cursor X in alternate screen.
    AlternateSavedX,
    /// Saved cursor Y in alternate screen.
    AlternateSavedY,
    /// 1 if bracketed paste mode is enabled.
    BracketedPaste,
    /// 1 if the cursor is blinking.
    CursorBlinking,
    /// Cursor colour in pane. Possible formats:
    /// - Named colors: `black`, `red`, `green`, `yellow`, `blue`, `magenta`,
    ///   `cyan`, `white`, `default`, `terminal`, or bright variants.
    /// - 256 colors: `colour<N>` where N is 0-255 (e.g., `colour100`).
    /// - RGB hex: `#RRGGBB` (e.g., `#ff0000`).
    /// - Empty string if unset.
    CursorColour,
    /// Pane cursor flag.
    CursorFlag,
    /// Cursor shape in pane. Possible values: `block`, `underline`, `bar`,
    /// or `default`.
    CursorShape,
    /// Cursor X position in pane.
    CursorX,
    /// Cursor Y position in pane.
    CursorY,
    /// 1 if focus reporting is enabled.
    FocusFlag,
    /// Pane insert flag.
    InsertFlag,
    /// Pane keypad cursor flag.
    KeypadCursorFlag,
    /// Pane keypad flag.
    KeypadFlag,
    /// Pane mouse all flag.
    MouseAllFlag,
    /// Pane mouse any flag.
    MouseAnyFlag,
    /// Pane mouse button flag.
    MouseButtonFlag,
    /// Pane mouse SGR flag.
    MouseSgrFlag,
    /// Pane mouse standard flag.
    MouseStandardFlag,
    /// Pane mouse UTF-8 flag.
    MouseUtf8Flag,
    /// Pane origin flag.
    OriginFlag,
    /// Unique pane ID prefixed with `%` (e.g., `%0`, `%42`).
    PaneId,
    /// Pane tab positions as a comma-separated list of 0-indexed column
    /// numbers (e.g., `8,16,24,32`). Empty string if no tabs are set.
    PaneTabs,
    /// Bottom of scroll region in pane.
    ScrollRegionLower,
    /// Top of scroll region in pane.
    ScrollRegionUpper,
    /// Unique session ID prefixed with `$` (e.g., `$0`, `$42`).
    SessionId,
    /// Server version (e.g., `3.5a`).
    Version,
    /// Unique window ID prefixed with `@` (e.g., `@0`, `@42`).
    WindowId,
    /// Width of window.
    WindowWidth,
    /// Height of window.
    WindowHeight,
    /// Window layout description, ignoring zoomed window panes. Format is
    /// `<checksum>,<layout>` where checksum is a 4-digit hex CRC16 and layout
    /// encodes pane dimensions as `WxH,X,Y[,ID]` with `{...}` for horizontal
    /// splits and `[...]` for vertical splits.
    WindowLayout,
    /// Pane wrap flag.
    WrapFlag,
}

impl Variable {
    /// The tmux variable name (the Zig tag name).
    pub fn name(self) -> &'static str {
        match self {
            Variable::AlternateOn => "alternate_on",
            Variable::AlternateSavedX => "alternate_saved_x",
            Variable::AlternateSavedY => "alternate_saved_y",
            Variable::BracketedPaste => "bracketed_paste",
            Variable::CursorBlinking => "cursor_blinking",
            Variable::CursorColour => "cursor_colour",
            Variable::CursorFlag => "cursor_flag",
            Variable::CursorShape => "cursor_shape",
            Variable::CursorX => "cursor_x",
            Variable::CursorY => "cursor_y",
            Variable::FocusFlag => "focus_flag",
            Variable::InsertFlag => "insert_flag",
            Variable::KeypadCursorFlag => "keypad_cursor_flag",
            Variable::KeypadFlag => "keypad_flag",
            Variable::MouseAllFlag => "mouse_all_flag",
            Variable::MouseAnyFlag => "mouse_any_flag",
            Variable::MouseButtonFlag => "mouse_button_flag",
            Variable::MouseSgrFlag => "mouse_sgr_flag",
            Variable::MouseStandardFlag => "mouse_standard_flag",
            Variable::MouseUtf8Flag => "mouse_utf8_flag",
            Variable::OriginFlag => "origin_flag",
            Variable::PaneId => "pane_id",
            Variable::PaneTabs => "pane_tabs",
            Variable::ScrollRegionLower => "scroll_region_lower",
            Variable::ScrollRegionUpper => "scroll_region_upper",
            Variable::SessionId => "session_id",
            Variable::Version => "version",
            Variable::WindowId => "window_id",
            Variable::WindowWidth => "window_width",
            Variable::WindowHeight => "window_height",
            Variable::WindowLayout => "window_layout",
            Variable::WrapFlag => "wrap_flag",
        }
    }

    /// Parse the given string value into the appropriate value for this
    /// variable.
    pub fn parse(self, value: &str) -> Result<Value, VariableError> {
        use Variable::*;
        Ok(match self {
            AlternateOn | BracketedPaste | CursorBlinking | CursorFlag | FocusFlag | InsertFlag
            | KeypadCursorFlag | KeypadFlag | MouseAllFlag | MouseAnyFlag | MouseButtonFlag
            | MouseSgrFlag | MouseStandardFlag | MouseUtf8Flag | OriginFlag | WrapFlag => {
                Value::Bool(value == "1")
            }
            AlternateSavedX | AlternateSavedY | CursorX | CursorY | ScrollRegionLower
            | ScrollRegionUpper | WindowWidth | WindowHeight => Value::Usize(parse_int(value)?),
            SessionId => Value::Usize(parse_prefixed(b'$', value)?),
            WindowId => Value::Usize(parse_prefixed(b'@', value)?),
            PaneId => Value::Usize(parse_prefixed(b'%', value)?),
            CursorColour | CursorShape | PaneTabs | Version | WindowLayout => {
                Value::Str(value.to_owned())
            }
        })
    }
}

/// Parsed values for a list of variables, in order (the runtime analogue of
/// Zig's `FormatStruct`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatValues {
    entries: Vec<(Variable, Value)>,
}

impl FormatValues {
    /// Look up a value.
    pub fn get(&self, v: Variable) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| *k == v).map(|(_, v)| v)
    }

    /// Bool accessor; panics if `v` is absent or not a flag.
    pub fn bool(&self, v: Variable) -> bool {
        match self.get(v) {
            Some(Value::Bool(b)) => *b,
            _ => panic!("variable {} is not a bool", v.name()),
        }
    }

    /// Integer accessor; panics if `v` is absent or not an integer.
    pub fn usize(&self, v: Variable) -> usize {
        match self.get(v) {
            Some(Value::Usize(n)) => *n,
            _ => panic!("variable {} is not an integer", v.name()),
        }
    }

    /// String accessor; panics if `v` is absent or not a string.
    pub fn str(&self, v: Variable) -> &str {
        match self.get(v) {
            Some(Value::Str(s)) => s,
            _ => panic!("variable {} is not a string", v.name()),
        }
    }
}

/// Parse the output from a command with the given variables (in the order
/// of the format string). Each variable is expected to be a plain variable
/// separated by a single `delimiter`.
pub fn parse_format_struct(
    vars: &[Variable],
    str: &str,
    delimiter: char,
) -> Result<FormatValues, ParseError> {
    let mut it = str.split(delimiter);
    let mut entries = Vec::with_capacity(vars.len());
    for &var in vars {
        let part = it.next().ok_or(ParseError::MissingEntry)?;
        let value = var.parse(part).map_err(|_| ParseError::FormatError)?;
        entries.push((var, value));
    }
    if it.next().is_some() {
        return Err(ParseError::ExtraEntry);
    }
    Ok(FormatValues { entries })
}

/// Format a set of variables into the proper format string for tmux
/// that we can handle with [`parse_format_struct`].
pub fn format(vars: &[Variable], delimiter: char) -> String {
    let mut out = String::new();
    for (i, v) in vars.iter().enumerate() {
        if i != 0 {
            out.push(delimiter);
        }
        out.push_str("#{");
        out.push_str(v.name());
        out.push('}');
    }
    out
}

#[cfg(test)]
mod tests {
    use Variable::*;

    use super::*;

    #[test]
    fn zig_parse_int_differences() {
        assert_eq!(parse_int("1_0"), Ok(10));
        assert_eq!(parse_int("+5"), Ok(5));
        assert_eq!(parse_int("-0"), Ok(0));
        assert_eq!(parse_int("-1"), Err(VariableError::Overflow));
        assert_eq!(parse_int("0x10"), Err(VariableError::InvalidCharacter));
        assert_eq!(parse_int("1_"), Err(VariableError::InvalidCharacter));
        assert_eq!(parse_int(""), Err(VariableError::InvalidCharacter));
        assert_eq!(
            parse_int("99999999999999999999999"),
            Err(VariableError::Overflow)
        );
        assert_eq!(PaneId.parse("%1_2"), Ok(Value::Usize(12)));
        assert_eq!(WindowId.parse("@+7"), Ok(Value::Usize(7)));
        assert_eq!(WindowWidth.parse("8_0"), Ok(Value::Usize(80)));
    }

    fn flag(v: Variable) {
        assert_eq!(v.parse("1"), Ok(Value::Bool(true)));
        assert_eq!(v.parse("0"), Ok(Value::Bool(false)));
        assert_eq!(v.parse(""), Ok(Value::Bool(false)));
        assert_eq!(v.parse("true"), Ok(Value::Bool(false)));
    }

    fn uint(v: Variable, ok: &[(&str, usize)]) {
        for (s, n) in ok {
            assert_eq!(v.parse(s), Ok(Value::Usize(*n)));
        }
        assert_eq!(v.parse("abc"), Err(VariableError::InvalidCharacter));
    }

    fn s(x: &str) -> Result<Value, VariableError> {
        Ok(Value::Str(x.to_owned()))
    }

    #[test]
    fn parse_alternate_on() {
        flag(AlternateOn);
        assert_eq!(AlternateOn.parse("yes"), Ok(Value::Bool(false)));
    }

    #[test]
    fn parse_alternate_saved_x() {
        uint(AlternateSavedX, &[("0", 0), ("42", 42)]);
    }

    #[test]
    fn parse_alternate_saved_y() {
        uint(AlternateSavedY, &[("0", 0), ("42", 42)]);
    }

    #[test]
    fn parse_cursor_x() {
        uint(CursorX, &[("0", 0), ("79", 79)]);
    }

    #[test]
    fn parse_cursor_y() {
        uint(CursorY, &[("0", 0), ("23", 23)]);
    }

    #[test]
    fn parse_scroll_region_upper() {
        uint(ScrollRegionUpper, &[("0", 0), ("5", 5)]);
    }

    #[test]
    fn parse_scroll_region_lower() {
        uint(ScrollRegionLower, &[("0", 0), ("23", 23)]);
    }

    #[test]
    fn parse_session_id() {
        assert_eq!(SessionId.parse("$42"), Ok(Value::Usize(42)));
        assert_eq!(SessionId.parse("$0"), Ok(Value::Usize(0)));
        assert_eq!(SessionId.parse("0"), Err(VariableError::FormatError));
        assert_eq!(SessionId.parse("@0"), Err(VariableError::FormatError));
        assert_eq!(SessionId.parse("$"), Err(VariableError::FormatError));
        assert_eq!(SessionId.parse(""), Err(VariableError::FormatError));
        assert_eq!(
            SessionId.parse("$abc"),
            Err(VariableError::InvalidCharacter)
        );
    }

    #[test]
    fn parse_window_id() {
        assert_eq!(WindowId.parse("@42"), Ok(Value::Usize(42)));
        assert_eq!(WindowId.parse("@0"), Ok(Value::Usize(0)));
        assert_eq!(WindowId.parse("0"), Err(VariableError::FormatError));
        assert_eq!(WindowId.parse("$0"), Err(VariableError::FormatError));
        assert_eq!(WindowId.parse("@"), Err(VariableError::FormatError));
        assert_eq!(WindowId.parse(""), Err(VariableError::FormatError));
        assert_eq!(WindowId.parse("@abc"), Err(VariableError::InvalidCharacter));
        assert_eq!(WindowId.parse("@12345"), Ok(Value::Usize(12345)));
    }

    #[test]
    fn parse_pane_id() {
        assert_eq!(PaneId.parse("%42"), Ok(Value::Usize(42)));
        assert_eq!(PaneId.parse("%0"), Ok(Value::Usize(0)));
        assert_eq!(PaneId.parse("0"), Err(VariableError::FormatError));
        assert_eq!(PaneId.parse("@0"), Err(VariableError::FormatError));
        assert_eq!(PaneId.parse("%"), Err(VariableError::FormatError));
        assert_eq!(PaneId.parse(""), Err(VariableError::FormatError));
        assert_eq!(PaneId.parse("%abc"), Err(VariableError::InvalidCharacter));
    }

    #[test]
    fn parse_window_width() {
        uint(WindowWidth, &[("80", 80), ("0", 0), ("12345", 12345)]);
        assert_eq!(
            WindowWidth.parse("80px"),
            Err(VariableError::InvalidCharacter)
        );
        assert_eq!(WindowWidth.parse("-1"), Err(VariableError::Overflow));
    }

    #[test]
    fn parse_window_height() {
        uint(WindowHeight, &[("80", 80), ("0", 0), ("12345", 12345)]);
        assert_eq!(
            WindowHeight.parse("80px"),
            Err(VariableError::InvalidCharacter)
        );
        assert_eq!(WindowHeight.parse("-1"), Err(VariableError::Overflow));
    }

    #[test]
    fn parse_window_layout() {
        assert_eq!(WindowLayout.parse("abc123"), s("abc123"));
        assert_eq!(WindowLayout.parse(""), s(""));
        assert_eq!(WindowLayout.parse("a]b,c{d}e(f)"), s("a]b,c{d}e(f)"));
    }

    #[test]
    fn parse_cursor_flag() {
        flag(CursorFlag);
    }

    #[test]
    fn parse_insert_flag() {
        flag(InsertFlag);
    }

    #[test]
    fn parse_keypad_cursor_flag() {
        flag(KeypadCursorFlag);
    }

    #[test]
    fn parse_keypad_flag() {
        flag(KeypadFlag);
    }

    #[test]
    fn parse_mouse_any_flag() {
        flag(MouseAnyFlag);
    }

    #[test]
    fn parse_mouse_button_flag() {
        flag(MouseButtonFlag);
    }

    #[test]
    fn parse_mouse_sgr_flag() {
        flag(MouseSgrFlag);
    }

    #[test]
    fn parse_mouse_standard_flag() {
        flag(MouseStandardFlag);
    }

    #[test]
    fn parse_mouse_utf8_flag() {
        flag(MouseUtf8Flag);
    }

    #[test]
    fn parse_wrap_flag() {
        flag(WrapFlag);
    }

    #[test]
    fn parse_bracketed_paste() {
        flag(BracketedPaste);
    }

    #[test]
    fn parse_cursor_blinking() {
        flag(CursorBlinking);
    }

    #[test]
    fn parse_focus_flag() {
        flag(FocusFlag);
    }

    #[test]
    fn parse_mouse_all_flag() {
        flag(MouseAllFlag);
    }

    #[test]
    fn parse_origin_flag() {
        flag(OriginFlag);
    }

    #[test]
    fn parse_cursor_colour() {
        assert_eq!(CursorColour.parse("red"), s("red"));
        assert_eq!(CursorColour.parse("#ff0000"), s("#ff0000"));
        assert_eq!(CursorColour.parse(""), s(""));
    }

    #[test]
    fn parse_cursor_shape() {
        assert_eq!(CursorShape.parse("block"), s("block"));
        assert_eq!(CursorShape.parse("underline"), s("underline"));
        assert_eq!(CursorShape.parse("bar"), s("bar"));
        assert_eq!(CursorShape.parse(""), s(""));
    }

    #[test]
    fn parse_pane_tabs() {
        assert_eq!(PaneTabs.parse("0,8,16,24"), s("0,8,16,24"));
        assert_eq!(PaneTabs.parse(""), s(""));
        assert_eq!(PaneTabs.parse("0"), s("0"));
    }

    #[test]
    fn parse_version() {
        assert_eq!(Version.parse("3.5a"), s("3.5a"));
        assert_eq!(Version.parse("3.5"), s("3.5"));
        assert_eq!(Version.parse("next-3.5"), s("next-3.5"));
        assert_eq!(Version.parse(""), s(""));
    }

    #[test]
    fn parse_format_struct_single_field() {
        let r = parse_format_struct(&[SessionId], "$42", ' ').unwrap();
        assert_eq!(r.usize(SessionId), 42);
    }

    #[test]
    fn parse_format_struct_multiple_fields() {
        let r = parse_format_struct(
            &[SessionId, WindowId, WindowWidth, WindowHeight],
            "$1 @2 80 24",
            ' ',
        )
        .unwrap();
        assert_eq!(r.usize(SessionId), 1);
        assert_eq!(r.usize(WindowId), 2);
        assert_eq!(r.usize(WindowWidth), 80);
        assert_eq!(r.usize(WindowHeight), 24);
    }

    #[test]
    fn parse_format_struct_with_string_field() {
        let r = parse_format_struct(&[WindowId, WindowLayout], "@5,abc123", ',').unwrap();
        assert_eq!(r.usize(WindowId), 5);
        assert_eq!(r.str(WindowLayout), "abc123");
    }

    #[test]
    fn parse_format_struct_different_delimiter() {
        let r = parse_format_struct(&[WindowWidth, WindowHeight], "120\t40", '\t').unwrap();
        assert_eq!(r.usize(WindowWidth), 120);
        assert_eq!(r.usize(WindowHeight), 40);
    }

    #[test]
    fn parse_format_struct_missing_entry() {
        assert_eq!(
            parse_format_struct(&[SessionId, WindowId], "$1", ' '),
            Err(ParseError::MissingEntry)
        );
    }

    #[test]
    fn parse_format_struct_extra_entry() {
        assert_eq!(
            parse_format_struct(&[SessionId], "$1 @2", ' '),
            Err(ParseError::ExtraEntry)
        );
    }

    #[test]
    fn parse_format_struct_format_error() {
        for x in ["42", "@42", "$abc"] {
            assert_eq!(
                parse_format_struct(&[SessionId], x, ' '),
                Err(ParseError::FormatError)
            );
        }
    }

    #[test]
    fn parse_format_struct_empty_string() {
        assert_eq!(
            parse_format_struct(&[SessionId], "", ' '),
            Err(ParseError::FormatError)
        );
    }

    #[test]
    fn parse_format_struct_with_empty_layout_field() {
        let r = parse_format_struct(&[SessionId, WindowLayout], "$1,", ',').unwrap();
        assert_eq!(r.usize(SessionId), 1);
        assert_eq!(r.str(WindowLayout), "");
    }

    #[test]
    fn format_single_variable() {
        assert_eq!(format(&[SessionId], ' '), "#{session_id}");
    }

    #[test]
    fn format_multiple_variables() {
        assert_eq!(
            format(&[SessionId, WindowId, WindowWidth, WindowHeight], ' '),
            "#{session_id} #{window_id} #{window_width} #{window_height}"
        );
    }

    #[test]
    fn format_with_comma_delimiter() {
        assert_eq!(
            format(&[WindowId, WindowLayout], ','),
            "#{window_id},#{window_layout}"
        );
    }

    #[test]
    fn format_with_tab_delimiter() {
        assert_eq!(
            format(&[WindowWidth, WindowHeight], '\t'),
            "#{window_width}\t#{window_height}"
        );
    }

    #[test]
    fn format_empty_variables() {
        assert_eq!(format(&[], ' '), "");
    }

    #[test]
    fn format_all_variables() {
        assert_eq!(
            format(
                &[SessionId, WindowId, WindowWidth, WindowHeight, WindowLayout],
                ' '
            ),
            "#{session_id} #{window_id} #{window_width} #{window_height} #{window_layout}"
        );
    }
}
