//! OSC 133 semantic prompts
//! (ZIG/src/terminal/osc/parsers/semantic_prompt.zig).
//!
//! <https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md>

use crate::{
    osc::{ByteStorage, Command as OscCommand, Parser, State},
    string_encoding::{printf_q_decode, url_percent_decode},
};

/// Error returned when decoding a command line fails (Zig
/// `error.DecodeError`).
pub type DecodeError = crate::string_encoding::Error;

/// A single semantic prompt command.
///
/// Technically according to the spec, not all commands have options
/// but it is easier to be "liberal in what we accept" here since
/// all except one do and the spec does also say to ignore unknown
/// options. So, I think this is a fair interpretation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command<'a> {
    pub action: Action,
    pub options_unvalidated: crate::osc::Bytes<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    FreshLine,                       // 'L'
    FreshLineNewPrompt,              // 'A'
    NewCommand,                      // 'N'
    PromptStart,                     // 'P'
    EndPromptStartInput,             // 'B'
    EndPromptStartInputTerminateEol, // 'I'
    EndInputStartOutput,             // 'C'
    EndCommand,                      // 'D'
}

impl<'a> Command<'a> {
    pub fn into_owned(self) -> Command<'static> {
        Command {
            action: self.action,
            options_unvalidated: super::super::own(self.options_unvalidated),
        }
    }

    pub fn init(action: Action) -> Command<'a> {
        Command {
            action,
            options_unvalidated: std::borrow::Cow::Borrowed(&[]),
        }
    }

    /// Read an option for this command. Returns None if unset or invalid.
    pub fn read_option(&self, option: OptionKey) -> Option<OptionValue<'_>> {
        option.read(&self.options_unvalidated)
    }

    /// Typed convenience: `aid` option.
    pub fn aid(&self) -> Option<&[u8]> {
        match self.read_option(OptionKey::Aid)? {
            OptionValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Typed convenience: `cl` option.
    pub fn cl(&self) -> Option<Click> {
        match self.read_option(OptionKey::Cl)? {
            OptionValue::Click(c) => Some(c),
            _ => None,
        }
    }

    /// Typed convenience: `k` option.
    pub fn prompt_kind(&self) -> Option<PromptKind> {
        match self.read_option(OptionKey::PromptKind)? {
            OptionValue::PromptKind(c) => Some(c),
            _ => None,
        }
    }

    /// Typed convenience: `err` option.
    pub fn err(&self) -> Option<&[u8]> {
        match self.read_option(OptionKey::Err)? {
            OptionValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Typed convenience: `redraw` option.
    pub fn redraw(&self) -> Option<Redraw> {
        match self.read_option(OptionKey::Redraw)? {
            OptionValue::Redraw(c) => Some(c),
            _ => None,
        }
    }

    /// Typed convenience: `special_key` option.
    pub fn special_key(&self) -> Option<bool> {
        match self.read_option(OptionKey::SpecialKey)? {
            OptionValue::Bool(c) => Some(c),
            _ => None,
        }
    }

    /// Typed convenience: `click_events` option.
    pub fn click_events(&self) -> Option<bool> {
        match self.read_option(OptionKey::ClickEvents)? {
            OptionValue::Bool(c) => Some(c),
            _ => None,
        }
    }

    /// Typed convenience: `exit_code` (first positional value of 'D').
    pub fn exit_code(&self) -> Option<i32> {
        match self.read_option(OptionKey::ExitCode)? {
            OptionValue::Int(c) => Some(c),
            _ => None,
        }
    }

    /// Write the decoded command line (if any) to the writer. If an error
    /// occurs garbage may have been written to the writer.
    pub fn write_command_line(&self, writer: &mut Vec<u8>) -> Result<(), DecodeError> {
        if let Some(OptionValue::Bytes(command_line)) = self.read_option(OptionKey::Cmdline) {
            return printf_q_decode(writer, command_line);
        }
        if let Some(OptionValue::Bytes(command_line)) = self.read_option(OptionKey::CmdlineUrl) {
            return url_percent_decode(writer, command_line);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKey {
    Aid,
    Cl,
    PromptKind,
    Err,
    Cmdline,
    CmdlineUrl,

    // https://sw.kovidgoyal.net/kitty/shell-integration/#notes-for-shell-developers
    // Kitty supports a "redraw" option for prompt_start. This is extended
    // by Ghostty with the "last" option. See Redraw the type for more details.
    Redraw,

    // Use a special key instead of arrow keys to move the cursor on
    // mouse click. Useful if arrow keys have side-effets like triggering
    // auto-complete. The shell integration script should bind the special
    // key as needed.
    // See: https://sw.kovidgoyal.net/kitty/shell-integration/#notes-for-shell-developers
    SpecialKey,

    // If true, the shell is capable of handling mouse click events.
    // Ghostty will then send a click event to the shell when the user
    // clicks somewhere in the prompt. The shell can then move the cursor
    // to that position or perform some other appropriate action. If false,
    // Ghostty may generate a number of fake key events to move the cursor
    // which is not very robust.
    // See: https://sw.kovidgoyal.net/kitty/shell-integration/#notes-for-shell-developers
    ClickEvents,

    // Not technically an option that can be set with k=v and only
    // present currently with command 'D' but its easier to just
    // parse it into our options.
    ExitCode,
}

/// The value of an option; the variant is determined by the `Option` (Zig
/// `Option.Type()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionValue<'a> {
    Bytes(&'a [u8]),
    Click(Click),
    PromptKind(PromptKind),
    Redraw(Redraw),
    Bool(bool),
    Int(i32),
}

impl OptionKey {
    fn key(self) -> &'static [u8] {
        match self {
            OptionKey::Aid => b"aid",
            OptionKey::Cl => b"cl",
            OptionKey::PromptKind => b"k",
            OptionKey::Err => b"err",
            OptionKey::Redraw => b"redraw",
            OptionKey::SpecialKey => b"special_key",
            OptionKey::ClickEvents => b"click_events",
            OptionKey::Cmdline => b"cmdline",
            OptionKey::CmdlineUrl => b"cmdline_url",

            // special case, handled before ever calling key
            OptionKey::ExitCode => unreachable!(),
        }
    }

    /// Read the option value from the raw options string.
    ///
    /// The raw options string is the raw unparsed data after the
    /// OSC 133 command. e.g. for `133;A;aid=14;cl=line`, the
    /// raw options string would be `aid=14;cl=line`.
    ///
    /// Any errors in the raw string will return None since the OSC133
    /// specification says to ignore unknown or malformed options.
    pub fn read(self, raw: &[u8]) -> Option<OptionValue<'_>> {
        let mut remaining = raw;
        'outer: while !remaining.is_empty() {
            // Length of the next value is up to the `;` or the
            // end of the string.
            let len = remaining
                .iter()
                .position(|&b| b == b';')
                .unwrap_or(remaining.len());

            // Grab our full value and move our cursor past the `;`
            let full = &remaining[..len];

            // If we're looking for exit_code we special case it.
            // as the first value.
            if self == OptionKey::ExitCode {
                return ghostty_zigfmt::parse_int::<i32>(full, 10)
                    .ok()
                    .map(OptionValue::Int);
            }

            // Parse our key=value and verify our key matches our
            // expectation.
            let value = 'value: {
                if let Some(eql_idx) = full
                    .iter()
                    .position(|&b| b == b'=')
                    .filter(|&i| full[..i] == *self.key())
                {
                    break 'value &full[eql_idx + 1..];
                }

                // No match!
                if len < remaining.len() {
                    remaining = &remaining[len + 1..];
                    continue 'outer;
                }

                break 'outer;
            };

            return match self {
                OptionKey::Aid | OptionKey::Err | OptionKey::Cmdline | OptionKey::CmdlineUrl => {
                    Some(OptionValue::Bytes(value))
                }
                OptionKey::Cl => Click::init(value).map(OptionValue::Click),
                OptionKey::PromptKind => {
                    if value.len() == 1 {
                        PromptKind::init(value[0]).map(OptionValue::PromptKind)
                    } else {
                        None
                    }
                }
                OptionKey::Redraw => match value {
                    b"0" => Some(OptionValue::Redraw(Redraw::False)),
                    b"1" => Some(OptionValue::Redraw(Redraw::True)),
                    b"last" => Some(OptionValue::Redraw(Redraw::Last)),
                    _ => None,
                },
                OptionKey::SpecialKey | OptionKey::ClickEvents => match value {
                    b"0" => Some(OptionValue::Bool(false)),
                    b"1" => Some(OptionValue::Bool(true)),
                    _ => None,
                },
                // Handled above
                OptionKey::ExitCode => unreachable!(),
            };
        }

        // Not found
        None
    }
}

/// The `cl` option specifies what kind of cursor key sequences are handled
/// by the application for click-to-move-cursor functionality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// Value: "line". Allows motion within a single input line using standard
    /// left/right arrow escape sequences. Only a single left/right sequence
    /// should be emitted for double-width characters.
    Line,

    /// Value: "m". Allows movement between different lines in the same group,
    /// but only using left/right arrow escape sequences.
    Multiple,

    /// Value: "v". Like `multiple` but cursor up/down should be used. The
    /// terminal should be conservative when moving between lines: move the
    /// cursor left to the start of line, emit the needed up/down sequences,
    /// then move the cursor right to the clicked destination.
    ConservativeVertical,

    /// Value: "w". Like `conservative_vertical` but specifies that there are
    /// no spurious spaces at the end of the line, and the application editor
    /// handles "smart vertical movement" (moving 2 lines up from position 20,
    /// where the intermediate line is 15 chars wide and the destination is
    /// 18 chars wide, ends at position 18).
    SmartVertical,
}

impl Click {
    pub fn init(value: &[u8]) -> Option<Click> {
        if value.len() == 1 {
            match value[0] {
                b'm' => Some(Click::Multiple),
                b'v' => Some(Click::ConservativeVertical),
                b'w' => Some(Click::SmartVertical),
                _ => None,
            }
        } else if value == b"line" {
            Some(Click::Line)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Initial,
    Right,
    Continuation,
    Secondary,
}

impl PromptKind {
    pub fn init(c: u8) -> Option<PromptKind> {
        match c {
            b'i' => Some(PromptKind::Initial),
            b'r' => Some(PromptKind::Right),
            b'c' => Some(PromptKind::Continuation),
            b's' => Some(PromptKind::Secondary),
            _ => None,
        }
    }
}

/// The values for the `redraw` extension to OSC133. This was
/// started by Kitty[1] and extended by Ghostty (the "last" option).
///
/// [1]: https://sw.kovidgoyal.net/kitty/shell-integration/#notes-for-shell-developers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redraw {
    /// The shell supports redrawing the full prompt and all continuations.
    /// This is the default value, it does not need to be explicitly set
    /// unless it is to reset a prior other value.
    True,

    /// The shell does NOT support redrawing. In this case, Ghostty will NOT
    /// clear any prompt lines on resize.
    False,

    /// The shell supports redrawing only the LAST line of the prompt.
    /// Ghostty will only clear the last line of the prompt on resize.
    ///
    /// This is specifically introduced because Bash only redraws the last
    /// line. It is literally the only shell that does this and it does this
    /// because its bad and they should feel bad. Don't be like Bash.
    Last,
}

/// Parse OSC 133, semantic prompts
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<OscCommand<'_>> {
    parser.buffered()?;

    // Validate with a short-lived borrow (marking the parser invalid cannot
    // overlap with the borrow handed to the command below).
    let (action, has_options) = {
        let data = parser.payload();
        if data.is_empty() {
            parser.state = State::Invalid;
            return None;
        }

        // All valid cases terminate within this block. Any fallthroughs
        // are invalid.
        let action = match data[0] {
            b'A' => Action::FreshLineNewPrompt,
            b'B' => Action::EndPromptStartInput,
            b'I' => Action::EndPromptStartInputTerminateEol,
            b'C' => Action::EndInputStartOutput,
            b'D' => Action::EndCommand,
            b'N' => Action::NewCommand,
            b'P' => Action::PromptStart,
            b'L' => {
                if data.len() > 1 {
                    parser.state = State::Invalid;
                    return None;
                }
                return Some(OscCommand::SemanticPrompt(Command::init(Action::FreshLine)));
            }
            _ => {
                parser.state = State::Invalid;
                return None;
            }
        };

        if data.len() > 1 && data[1] != b';' {
            parser.state = State::Invalid;
            return None;
        }
        (action, data.len() > 1)
    };

    let mut cmd = Command::init(action);
    if has_options {
        cmd.options_unvalidated = parser.payload()[2..].into();
    }
    Some(OscCommand::SemanticPrompt(cmd))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn zig_parse_int_exit_code() {
        assert_eq!(run("133;D;1_0").unwrap().exit_code(), Some(10));
        assert_eq!(run("133;D;+5").unwrap().exit_code(), Some(5));
        assert_eq!(run("133;D;-5").unwrap().exit_code(), Some(-5));
        assert_eq!(run("133;D;0x10").unwrap().exit_code(), None);
        assert_eq!(run("133;D;1_").unwrap().exit_code(), None);
        assert_eq!(run("133;D;99999999999").unwrap().exit_code(), None);
    }

    fn run(input: &str) -> Option<Command<'static>> {
        let mut p = Parser::init(false);
        for ch in input.bytes() {
            p.next(ch);
        }
        match p.end(None) {
            Some(OscCommand::SemanticPrompt(c)) => Some(c.into_owned()),
            Some(c) => panic!("unexpected {c:?}"),
            None => None,
        }
    }

    fn cmdline(input: &str) -> (Command<'static>, Result<Vec<u8>, DecodeError>) {
        let cmd = run(input).unwrap();
        assert_eq!(cmd.action, Action::EndInputStartOutput);
        let mut w = Vec::new();
        let r = cmd.write_command_line(&mut w).map(|()| w);
        (cmd, r)
    }

    fn cmdline_ok(input: &str, expected: &str) {
        assert_eq!(cmdline(input).1.unwrap(), expected.as_bytes());
    }

    fn cmdline_err(input: &str) {
        assert_eq!(cmdline(input).1, Err(DecodeError::DecodeError));
    }

    #[test]
    fn osc_133_end_input_start_output() {
        let cmd = run("133;C").unwrap();
        assert_eq!(cmd.action, Action::EndInputStartOutput);
        assert!(cmd.aid().is_none());
        assert!(cmd.cl().is_none());
    }

    #[test]
    fn osc_133_end_input_start_output_extra_contents() {
        assert!(run("133;Cextra").is_none());
    }

    #[test]
    fn osc_133_end_input_start_output_with_options() {
        let cmd = run("133;C;aid=foo").unwrap();
        assert_eq!(cmd.action, Action::EndInputStartOutput);
        assert_eq!(cmd.aid().unwrap(), b"foo");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline() {
        cmdline_ok("133;C;cmdline=echo bobr kurwa", "echo bobr kurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_3() {
        cmdline_ok("133;C;cmdline=echo bobr\\nkurwa", "echo bobr\nkurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_4() {
        cmdline_ok("133;C;cmdline=$'echo bobr kurwa'", "echo bobr kurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_5() {
        cmdline_ok("133;C;cmdline='echo bobr kurwa'", "echo bobr kurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_6() {
        cmdline_err("133;C;cmdline='echo bobr kurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_7() {
        cmdline_err("133;C;cmdline=$'echo bobr kurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_8() {
        cmdline_err("133;C;cmdline=$'");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_9() {
        cmdline_ok("133;C;cmdline=", "");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_url_1() {
        cmdline_ok("133;C;cmdline_url=echo bobr kurwa", "echo bobr kurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_url_2() {
        cmdline_ok("133;C;cmdline_url=echo bobr%20kurwa", "echo bobr kurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_url_3() {
        cmdline_ok("133;C;cmdline_url=echo bobr%3bkurwa", "echo bobr;kurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_url_4() {
        cmdline_err("133;C;cmdline_url=echo bobr%3kurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_url_5() {
        cmdline_err("133;C;cmdline_url=echo bobr%kurwa");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_url_6() {
        cmdline_ok("133;C;cmdline_url=echo bobr kurwa%20", "echo bobr kurwa ");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_url_7() {
        cmdline_err("133;C;cmdline_url=echo bobr kurwa%2");
    }

    #[test]
    fn osc_133_end_input_start_output_with_cmdline_url_8() {
        cmdline_err("133;C;cmdline_url=echo bobr kurwa%");
    }

    #[test]
    fn osc_133_fresh_line() {
        assert_eq!(run("133;L").unwrap().action, Action::FreshLine);
    }

    #[test]
    fn osc_133_fresh_line_extra_contents() {
        // Random
        assert!(run("133;Lol").is_none());
        // Options
        assert!(run("133;L;aid=foo").is_none());
    }

    #[test]
    fn osc_133_fresh_line_new_prompt() {
        let cmd = run("133;A").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
        assert!(cmd.aid().is_none());
        assert!(cmd.cl().is_none());
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_aid() {
        let cmd = run("133;A;aid=14").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
        assert_eq!(cmd.aid().unwrap(), b"14");
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_equals_in_aid() {
        let cmd = run("133;A;aid=a=b").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
        assert_eq!(cmd.aid().unwrap(), b"a=b");
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_cl_line() {
        let cmd = run("133;A;cl=line").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
        assert_eq!(cmd.cl(), Some(Click::Line));
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_cl_m() {
        let cmd = run("133;A;cl=m").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
        assert_eq!(cmd.cl(), Some(Click::Multiple));
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_invalid_cl() {
        let cmd = run("133;A;cl=invalid").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
        assert!(cmd.cl().is_none());
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_trailing_semicolon() {
        let cmd = run("133;A;").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_bare_key() {
        let cmd = run("133;A;barekey").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
        assert!(cmd.aid().is_none());
        assert!(cmd.cl().is_none());
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_multiple_options() {
        let cmd = run("133;A;aid=foo;cl=line").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
        assert_eq!(cmd.aid().unwrap(), b"foo");
        assert_eq!(cmd.cl(), Some(Click::Line));
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_default_redraw() {
        let cmd = run("133;A").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
        assert!(cmd.redraw().is_none());
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_redraw_0() {
        let cmd = run("133;A;redraw=0").unwrap();
        assert_eq!(cmd.redraw(), Some(Redraw::False));
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_redraw_1() {
        let cmd = run("133;A;redraw=1").unwrap();
        assert_eq!(cmd.redraw(), Some(Redraw::True));
    }

    #[test]
    fn osc_133_fresh_line_new_prompt_with_invalid_redraw() {
        let cmd = run("133;A;redraw=x").unwrap();
        assert_eq!(cmd.action, Action::FreshLineNewPrompt);
        assert!(cmd.redraw().is_none());
    }

    #[test]
    fn osc_133_prompt_start() {
        let cmd = run("133;P").unwrap();
        assert_eq!(cmd.action, Action::PromptStart);
        assert!(cmd.prompt_kind().is_none());
    }

    #[test]
    fn osc_133_prompt_start_with_k_i() {
        let cmd = run("133;P;k=i").unwrap();
        assert_eq!(cmd.action, Action::PromptStart);
        assert_eq!(cmd.prompt_kind(), Some(PromptKind::Initial));
    }

    #[test]
    fn osc_133_prompt_start_with_k_r() {
        let cmd = run("133;P;k=r").unwrap();
        assert_eq!(cmd.prompt_kind(), Some(PromptKind::Right));
    }

    #[test]
    fn osc_133_prompt_start_with_k_c() {
        let cmd = run("133;P;k=c").unwrap();
        assert_eq!(cmd.prompt_kind(), Some(PromptKind::Continuation));
    }

    #[test]
    fn osc_133_prompt_start_with_k_s() {
        let cmd = run("133;P;k=s").unwrap();
        assert_eq!(cmd.prompt_kind(), Some(PromptKind::Secondary));
    }

    #[test]
    fn osc_133_prompt_start_with_invalid_k() {
        let cmd = run("133;P;k=x").unwrap();
        assert_eq!(cmd.action, Action::PromptStart);
        assert!(cmd.prompt_kind().is_none());
    }

    #[test]
    fn osc_133_prompt_start_extra_contents() {
        assert!(run("133;Pextra").is_none());
    }

    #[test]
    fn osc_133_new_command() {
        let cmd = run("133;N").unwrap();
        assert_eq!(cmd.action, Action::NewCommand);
        assert!(cmd.aid().is_none());
        assert!(cmd.cl().is_none());
    }

    #[test]
    fn osc_133_new_command_with_aid() {
        let cmd = run("133;N;aid=foo").unwrap();
        assert_eq!(cmd.action, Action::NewCommand);
        assert_eq!(cmd.aid().unwrap(), b"foo");
    }

    #[test]
    fn osc_133_new_command_with_cl_line() {
        let cmd = run("133;N;cl=line").unwrap();
        assert_eq!(cmd.action, Action::NewCommand);
        assert_eq!(cmd.cl(), Some(Click::Line));
    }

    #[test]
    fn osc_133_new_command_with_multiple_options() {
        let cmd = run("133;N;aid=foo;cl=line").unwrap();
        assert_eq!(cmd.action, Action::NewCommand);
        assert_eq!(cmd.aid().unwrap(), b"foo");
        assert_eq!(cmd.cl(), Some(Click::Line));
    }

    #[test]
    fn osc_133_new_command_extra_contents() {
        assert!(run("133;Nextra").is_none());
    }

    #[test]
    fn osc_133_end_prompt_start_input() {
        let cmd = run("133;B").unwrap();
        assert_eq!(cmd.action, Action::EndPromptStartInput);
    }

    #[test]
    fn osc_133_end_prompt_start_input_extra_contents() {
        assert!(run("133;Bextra").is_none());
    }

    #[test]
    fn osc_133_end_prompt_start_input_with_options() {
        let cmd = run("133;B;aid=foo").unwrap();
        assert_eq!(cmd.action, Action::EndPromptStartInput);
        assert_eq!(cmd.aid().unwrap(), b"foo");
    }

    #[test]
    fn osc_133_end_prompt_start_input_terminate_eol() {
        let cmd = run("133;I").unwrap();
        assert_eq!(cmd.action, Action::EndPromptStartInputTerminateEol);
    }

    #[test]
    fn osc_133_end_prompt_start_input_terminate_eol_extra_contents() {
        assert!(run("133;Iextra").is_none());
    }

    #[test]
    fn osc_133_end_prompt_start_input_terminate_eol_with_options() {
        let cmd = run("133;I;aid=foo").unwrap();
        assert_eq!(cmd.action, Action::EndPromptStartInputTerminateEol);
        assert_eq!(cmd.aid().unwrap(), b"foo");
    }

    #[test]
    fn osc_133_end_command() {
        let cmd = run("133;D").unwrap();
        assert_eq!(cmd.action, Action::EndCommand);
        assert!(cmd.exit_code().is_none());
        assert!(cmd.aid().is_none());
        assert!(cmd.err().is_none());
    }

    #[test]
    fn osc_133_end_command_extra_contents() {
        assert!(run("133;Dextra").is_none());
    }

    #[test]
    fn osc_133_end_command_with_exit_code_0() {
        let cmd = run("133;D;0").unwrap();
        assert_eq!(cmd.action, Action::EndCommand);
        assert_eq!(cmd.exit_code(), Some(0));
    }

    #[test]
    fn osc_133_end_command_with_exit_code_and_aid() {
        let cmd = run("133;D;12;aid=foo").unwrap();
        assert_eq!(cmd.action, Action::EndCommand);
        assert_eq!(cmd.aid().unwrap(), b"foo");
        assert_eq!(cmd.exit_code(), Some(12));
    }

    fn bytes(v: Option<OptionValue<'_>>) -> Option<&[u8]> {
        match v {
            Some(OptionValue::Bytes(b)) => Some(b),
            _ => None,
        }
    }

    #[test]
    fn option_read_aid() {
        let r = |s: &'static str| bytes(OptionKey::Aid.read(s.as_bytes()));
        assert_eq!(r("aid=test123").unwrap(), b"test123");
        assert_eq!(r("cl=line;aid=myaid;k=i").unwrap(), b"myaid");
        assert!(r("cl=line;k=i").is_none());
        assert_eq!(r("aid=").unwrap(), b"");
        assert_eq!(r("k=i;aid=last").unwrap(), b"last");
        assert_eq!(r("aid=first;k=i").unwrap(), b"first");
        assert!(r("").is_none());
        assert!(r("aid").is_none());
        assert_eq!(r(";;aid=value;;").unwrap(), b"value");
    }

    #[test]
    fn option_read_cl() {
        let r = |s: &'static str| OptionKey::Cl.read(s.as_bytes());
        assert_eq!(r("cl=line"), Some(OptionValue::Click(Click::Line)));
        assert_eq!(r("cl=m"), Some(OptionValue::Click(Click::Multiple)));
        assert_eq!(
            r("cl=v"),
            Some(OptionValue::Click(Click::ConservativeVertical))
        );
        assert_eq!(r("cl=w"), Some(OptionValue::Click(Click::SmartVertical)));
        assert!(r("cl=invalid").is_none());
        assert!(r("aid=foo").is_none());
    }

    #[test]
    fn option_read_prompt_kind() {
        let r = |s: &'static str| OptionKey::PromptKind.read(s.as_bytes());
        assert_eq!(r("k=i"), Some(OptionValue::PromptKind(PromptKind::Initial)));
        assert_eq!(r("k=r"), Some(OptionValue::PromptKind(PromptKind::Right)));
        assert_eq!(
            r("k=c"),
            Some(OptionValue::PromptKind(PromptKind::Continuation))
        );
        assert_eq!(
            r("k=s"),
            Some(OptionValue::PromptKind(PromptKind::Secondary))
        );
        assert!(r("k=x").is_none());
        assert!(r("k=ii").is_none());
        assert!(r("k=").is_none());
    }

    #[test]
    fn option_read_err() {
        assert_eq!(
            bytes(OptionKey::Err.read(b"err=some_error")).unwrap(),
            b"some_error"
        );
        assert!(OptionKey::Err.read(b"aid=foo").is_none());
    }

    #[test]
    fn option_read_redraw() {
        let r = |s: &'static str| OptionKey::Redraw.read(s.as_bytes());
        assert_eq!(r("redraw=1"), Some(OptionValue::Redraw(Redraw::True)));
        assert_eq!(r("redraw=0"), Some(OptionValue::Redraw(Redraw::False)));
        assert_eq!(r("redraw=last"), Some(OptionValue::Redraw(Redraw::Last)));
        assert!(r("redraw=2").is_none());
        assert!(r("redraw=10").is_none());
        assert!(r("redraw=").is_none());
    }

    #[test]
    fn option_read_special_key() {
        let r = |s: &'static str| OptionKey::SpecialKey.read(s.as_bytes());
        assert_eq!(r("special_key=1"), Some(OptionValue::Bool(true)));
        assert_eq!(r("special_key=0"), Some(OptionValue::Bool(false)));
        assert!(r("special_key=x").is_none());
    }

    #[test]
    fn option_read_click_events() {
        let r = |s: &'static str| OptionKey::ClickEvents.read(s.as_bytes());
        assert_eq!(r("click_events=1"), Some(OptionValue::Bool(true)));
        assert_eq!(r("click_events=0"), Some(OptionValue::Bool(false)));
        assert!(r("click_events=yes").is_none());
    }

    #[test]
    fn option_read_exit_code() {
        let r = |s: &'static str| OptionKey::ExitCode.read(s.as_bytes());
        assert_eq!(r("42"), Some(OptionValue::Int(42)));
        assert_eq!(r("0"), Some(OptionValue::Int(0)));
        assert_eq!(r("-1"), Some(OptionValue::Int(-1)));
        assert!(r("abc").is_none());
        assert_eq!(r("127;aid=foo"), Some(OptionValue::Int(127)));
    }
}
