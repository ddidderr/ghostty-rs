//! OSC 9 (iTerm2 notification / ConEmu extensions)
//! (ZIG/src/terminal/osc/parsers/osc9.zig).

use super::semantic_prompt;
use crate::osc::{
    ByteStorage,
    Command,
    ConemuChangeTabTitle,
    Parser,
    ProgressReport,
    ProgressState,
};

/// Parse a base-10 unsigned integer from raw bytes (Zig `parseUnsigned`).
fn parse_unsigned<T: std::str::FromStr>(bytes: &[u8]) -> Option<T> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Parse OSC 9, which could be an iTerm2 notification or a ConEmu extension.
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<Command<'_>> {
    // Zig prologue: writer must exist. The NUL-terminator reservation is NOT
    // done here: Zig only appends the NUL (`writer.writeByte(0)`) in the
    // branches that return text, so full-buffer control commands (sleep,
    // xterm emulation, progress, ...) still parse.
    parser.buffered()?;
    let data = parser.payload();

    // Check first to see if this is a ConEmu OSC
    // https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC
    if let Some((cmd, needs_nul)) = conemu(data) {
        if needs_nul && !parser.reserve_nul() {
            return None;
        }
        return Some(cmd);
    }

    // If it's not a ConEmu OSC, it's an iTerm2 notification
    if !parser.reserve_nul() {
        return None;
    }
    Some(Command::ShowDesktopNotification {
        title: std::borrow::Cow::Borrowed(&[]),
        body: data.into(),
    })
}

/// Returns None where the Zig code does `break :conemu`. The flag is true
/// when the Zig branch appends a NUL terminator (needs buffer room).
fn conemu(data: &[u8]) -> Option<(Command<'_>, bool)> {
    let r = conemu_inner(data)?;
    let needs_nul = matches!(
        r,
        Command::ConemuComment(_)
            | Command::ConemuShowMessageBox(_)
            | Command::ConemuChangeTabTitle(_)
            | Command::ConemuGuimacro(_)
            | Command::ConemuRunProcess(_)
            | Command::ConemuOutputEnvironmentVariable(_)
            | Command::ReportPwd { .. }
    );
    Some((r, needs_nul))
}

fn conemu_inner(data: &[u8]) -> Option<Command<'_>> {
    if data.is_empty() {
        return None;
    }
    let msg = |skip: usize| -> crate::osc::Bytes<'_> { data[skip..].into() };
    match data[0] {
        // Check for OSC 9;1 9;10 9;11 9;12
        b'1' => {
            if data.len() < 2 {
                return None;
            }
            match data[1] {
                // OSC 9;1 sleep
                b';' => Some(Command::ConemuSleep {
                    duration_ms: match parse_unsigned::<u16>(&data[2..]) {
                        Some(num) => num.min(10_000),
                        None => 100,
                    },
                }),
                // OSC 9;10 xterm keyboard and output emulation
                b'0' => {
                    if data.len() == 2 {
                        return Some(Command::ConemuXtermEmulation {
                            keyboard: Some(true),
                            output: Some(true),
                        });
                    }
                    if data.len() < 4 {
                        return None;
                    }
                    if data[2] != b';' {
                        return None;
                    }
                    let (keyboard, output) = match data[3] {
                        b'0' => (Some(false), Some(false)),
                        b'1' => (Some(true), Some(true)),
                        b'2' => (None, Some(false)),
                        b'3' => (None, Some(true)),
                        _ => return None,
                    };
                    Some(Command::ConemuXtermEmulation { keyboard, output })
                }
                // OSC 9;11 comment
                b'1' => {
                    if data.len() < 3 {
                        return None;
                    }
                    if data[2] != b';' {
                        return None;
                    }
                    Some(Command::ConemuComment(msg(3)))
                }
                // OSC 9;12 mark prompt start
                b'2' => Some(Command::SemanticPrompt(semantic_prompt::Command::init(
                    semantic_prompt::Action::FreshLineNewPrompt,
                ))),
                _ => None,
            }
        }
        // OSC 9;2 show message box
        b'2' => {
            if data.len() < 2 || data[1] != b';' {
                return None;
            }
            Some(Command::ConemuShowMessageBox(msg(2)))
        }
        // OSC 9;3 change tab title
        b'3' => {
            if data.len() < 2 || data[1] != b';' {
                return None;
            }
            if data.len() == 2 {
                return Some(Command::ConemuChangeTabTitle(ConemuChangeTabTitle::Reset));
            }
            Some(Command::ConemuChangeTabTitle(ConemuChangeTabTitle::Value(
                msg(2),
            )))
        }
        // OSC 9;4 progress report
        b'4' => {
            if data.len() < 2 || data[1] != b';' || data.len() < 3 {
                return None;
            }
            let mut report = match data[2] {
                b'0' => ProgressReport {
                    state: ProgressState::Remove,
                    progress: None,
                },
                b'1' => ProgressReport {
                    state: ProgressState::Set,
                    progress: Some(0),
                },
                b'2' => ProgressReport {
                    state: ProgressState::Error,
                    progress: None,
                },
                b'3' => ProgressReport {
                    state: ProgressState::Indeterminate,
                    progress: None,
                },
                b'4' => ProgressReport {
                    state: ProgressState::Pause,
                    progress: None,
                },
                _ => return None,
            };
            match report.state {
                ProgressState::Remove | ProgressState::Indeterminate => {}
                ProgressState::Set | ProgressState::Error | ProgressState::Pause => {
                    if data.len() >= 4 && data[3] == b';' {
                        // parse the progress value
                        report.progress =
                            parse_unsigned::<usize>(&data[4..]).map(|v| v.min(100) as u8);
                    }
                }
            }
            Some(Command::ConemuProgressReport(report))
        }
        // OSC 9;5 wait for input
        b'5' => Some(Command::ConemuWaitInput),
        // OSC 9;6 guimacro
        b'6' => {
            if data.len() < 2 || data[1] != b';' {
                return None;
            }
            Some(Command::ConemuGuimacro(msg(2)))
        }
        // OSC 9;7 run process
        b'7' => {
            if data.len() < 2 || data[1] != b';' {
                return None;
            }
            Some(Command::ConemuRunProcess(msg(2)))
        }
        // OSC 9;8 output environment variable
        b'8' => {
            if data.len() < 2 || data[1] != b';' {
                return None;
            }
            Some(Command::ConemuOutputEnvironmentVariable(msg(2)))
        }
        // OSC 9;9 current working directory
        b'9' => {
            if data.len() < 2 || data[1] != b';' {
                return None;
            }
            Some(Command::ReportPwd { value: msg(2) })
        }
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn run(input: &str) -> Command<'static> {
        let mut p = Parser::init(false);
        for ch in input.bytes() {
            p.next(ch);
        }
        p.end(Some(0x1b)).unwrap().into_owned()
    }

    fn notif(input: &str, body: &str) {
        match run(input) {
            Command::ShowDesktopNotification { title, body: b } => {
                assert_eq!(&title[..], b"");
                assert_eq!(b, body.as_bytes());
            }
            c => panic!("unexpected {c:?}"),
        }
    }

    fn progress(input: &str, state: ProgressState, progress: Option<u8>) {
        assert_eq!(
            run(input),
            Command::ConemuProgressReport(ProgressReport { state, progress })
        );
    }

    fn xterm(input: &str, keyboard: Option<bool>, output: Option<bool>) {
        assert_eq!(
            run(input),
            Command::ConemuXtermEmulation { keyboard, output }
        );
    }

    fn run_opt(input: &str) -> Option<Command<'static>> {
        let mut p = Parser::init(false);
        for ch in input.bytes() {
            p.next(ch);
        }
        p.end(Some(0x1b)).map(Command::into_owned)
    }

    #[test]
    fn full_buffer_control_commands_do_not_need_nul() {
        let fill = crate::osc::MAX_BUF;
        // OSC 9;1 sleep with a buffer that has no room for a NUL: Zig never
        // appends one for this command, so it still parses (default 100ms).
        let input = format!("9;1;{}", "x".repeat(fill - 2));
        assert_eq!(
            run_opt(&input),
            Some(Command::ConemuSleep { duration_ms: 100 })
        );
        // OSC 9;5 wait for input.
        let input = format!("9;5{}", "x".repeat(fill - 1));
        assert_eq!(run_opt(&input), Some(Command::ConemuWaitInput));
    }

    #[test]
    fn full_buffer_text_commands_are_invalid() {
        let fill = crate::osc::MAX_BUF;
        // These append a NUL in Zig, which fails on a full buffer.
        let input = format!("9;6;{}", "a".repeat(fill - 2));
        assert_eq!(run_opt(&input), None);
        let input = format!("9;{}", "a".repeat(fill));
        assert_eq!(run_opt(&input), None);
    }

    #[test]
    fn osc_9_show_desktop_notification() {
        notif("9;Hello world", "Hello world");
    }

    #[test]
    fn osc_9_show_single_character_desktop_notification() {
        notif("9;H", "H");
    }

    #[test]
    fn osc_9_1_conemu_sleep() {
        assert_eq!(run("9;1;420"), Command::ConemuSleep { duration_ms: 420 });
    }

    #[test]
    fn osc_9_1_conemu_sleep_with_no_value_default_to_100ms() {
        assert_eq!(run("9;1;"), Command::ConemuSleep { duration_ms: 100 });
    }

    #[test]
    fn osc_9_1_conemu_sleep_cannot_exceed_10000ms() {
        assert_eq!(
            run("9;1;12345"),
            Command::ConemuSleep { duration_ms: 10000 }
        );
    }

    #[test]
    fn osc_9_1_conemu_sleep_invalid_input() {
        assert_eq!(run("9;1;foo"), Command::ConemuSleep { duration_ms: 100 });
    }

    #[test]
    fn osc_9_1_conemu_sleep_desktop_notification_1() {
        notif("9;1", "1");
    }

    #[test]
    fn osc_9_1_conemu_sleep_desktop_notification_2() {
        notif("9;1a", "1a");
    }

    #[test]
    fn osc_9_2_conemu_message_box() {
        assert_eq!(
            run("9;2;hello world"),
            Command::ConemuShowMessageBox(b"hello world".to_vec().into())
        );
    }

    #[test]
    fn osc_9_2_conemu_message_box_invalid_input() {
        notif("9;2", "2");
    }

    #[test]
    fn osc_9_2_conemu_message_box_empty_message() {
        assert_eq!(
            run("9;2;"),
            Command::ConemuShowMessageBox(Vec::new().into())
        );
    }

    #[test]
    fn osc_9_2_conemu_message_box_spaces_only_message() {
        assert_eq!(
            run("9;2;   "),
            Command::ConemuShowMessageBox(b"   ".to_vec().into())
        );
    }

    #[test]
    fn osc_9_2_message_box_desktop_notification_1() {
        notif("9;2", "2");
    }

    #[test]
    fn osc_9_2_message_box_desktop_notification_2() {
        notif("9;2a", "2a");
    }

    #[test]
    fn osc_9_3_conemu_change_tab_title() {
        assert_eq!(
            run("9;3;foo bar"),
            Command::ConemuChangeTabTitle(ConemuChangeTabTitle::Value(b"foo bar".to_vec().into()))
        );
    }

    #[test]
    fn osc_9_3_conemu_change_tab_title_reset() {
        assert_eq!(
            run("9;3;"),
            Command::ConemuChangeTabTitle(ConemuChangeTabTitle::Reset)
        );
    }

    #[test]
    fn osc_9_3_conemu_change_tab_title_spaces_only() {
        assert_eq!(
            run("9;3;   "),
            Command::ConemuChangeTabTitle(ConemuChangeTabTitle::Value(b"   ".to_vec().into()))
        );
    }

    #[test]
    fn osc_9_3_change_tab_title_desktop_notification_1() {
        notif("9;3", "3");
    }

    #[test]
    fn osc_9_3_message_box_desktop_notification_2() {
        notif("9;3a", "3a");
    }

    #[test]
    fn osc_9_4_conemu_progress_set() {
        progress("9;4;1;100", ProgressState::Set, Some(100));
    }

    #[test]
    fn osc_9_4_conemu_progress_set_overflow() {
        progress("9;4;1;900", ProgressState::Set, Some(100));
    }

    #[test]
    fn osc_9_4_conemu_progress_set_single_digit() {
        progress("9;4;1;9", ProgressState::Set, Some(9));
    }

    #[test]
    fn osc_9_4_conemu_progress_set_double_digit() {
        progress("9;4;1;94", ProgressState::Set, Some(94));
    }

    #[test]
    fn osc_9_4_conemu_progress_set_extra_semicolon_ignored() {
        progress("9;4;1;100", ProgressState::Set, Some(100));
    }

    #[test]
    fn osc_9_4_conemu_progress_remove_with_no_progress() {
        progress("9;4;0;", ProgressState::Remove, None);
    }

    #[test]
    fn osc_9_4_conemu_progress_remove_with_double_semicolon() {
        progress("9;4;0;;", ProgressState::Remove, None);
    }

    #[test]
    fn osc_9_4_conemu_progress_remove_ignores_progress() {
        progress("9;4;0;100", ProgressState::Remove, None);
    }

    #[test]
    fn osc_9_4_conemu_progress_remove_extra_semicolon() {
        match run("9;4;0;100;") {
            Command::ConemuProgressReport(r) => assert_eq!(r.state, ProgressState::Remove),
            c => panic!("unexpected {c:?}"),
        }
    }

    #[test]
    fn osc_9_4_conemu_progress_error() {
        progress("9;4;2", ProgressState::Error, None);
    }

    #[test]
    fn osc_9_4_conemu_progress_error_with_progress() {
        progress("9;4;2;100", ProgressState::Error, Some(100));
    }

    #[test]
    fn osc_9_4_progress_pause() {
        progress("9;4;4", ProgressState::Pause, None);
    }

    #[test]
    fn osc_9_4_conemu_progress_pause_with_progress() {
        progress("9;4;4;100", ProgressState::Pause, Some(100));
    }

    #[test]
    fn osc_9_4_progress_desktop_notification_1() {
        notif("9;4", "4");
    }

    #[test]
    fn osc_9_4_progress_desktop_notification_2() {
        notif("9;4;", "4;");
    }

    #[test]
    fn osc_9_4_progress_desktop_notification_3() {
        notif("9;4;5", "4;5");
    }

    #[test]
    fn osc_9_4_progress_desktop_notification_4() {
        notif("9;4;5a", "4;5a");
    }

    #[test]
    fn osc_9_5_conemu_wait_input() {
        assert_eq!(run("9;5"), Command::ConemuWaitInput);
    }

    #[test]
    fn osc_9_5_conemu_wait_ignores_trailing_characters() {
        assert_eq!(run("9;5;foo"), Command::ConemuWaitInput);
    }

    #[test]
    fn osc_9_6_conemu_guimacro_1() {
        assert_eq!(run("9;6;a"), Command::ConemuGuimacro(b"a".to_vec().into()));
    }

    #[test]
    fn osc_9_6_conemu_guimacro_2() {
        assert_eq!(
            run("9;6;ab"),
            Command::ConemuGuimacro(b"ab".to_vec().into())
        );
    }

    #[test]
    fn osc_9_6_conemu_guimacro_3_incomplete_desktop_notification() {
        notif("9;6", "6");
    }

    #[test]
    fn osc_9_7_conemu_run_process_1() {
        assert_eq!(
            run("9;7;ab"),
            Command::ConemuRunProcess(b"ab".to_vec().into())
        );
    }

    #[test]
    fn osc_9_7_conemu_run_process_2() {
        assert_eq!(run("9;7;"), Command::ConemuRunProcess(Vec::new().into()));
    }

    #[test]
    fn osc_9_7_conemu_run_process_incomplete_desktop_notification() {
        notif("9;7", "7");
    }

    #[test]
    fn osc_9_8_conemu_output_environment_variable_1() {
        assert_eq!(
            run("9;8;ab"),
            Command::ConemuOutputEnvironmentVariable(b"ab".to_vec().into())
        );
    }

    #[test]
    fn osc_9_8_conemu_output_environment_variable_2() {
        assert_eq!(
            run("9;8;"),
            Command::ConemuOutputEnvironmentVariable(Vec::new().into())
        );
    }

    #[test]
    fn osc_9_8_conemu_output_environment_variable_incomplete_desktop_notification() {
        notif("9;8", "8");
    }

    #[test]
    fn osc_9_9_conemu_set_current_working_directory() {
        assert_eq!(
            run("9;9;ab"),
            Command::ReportPwd {
                value: b"ab".to_vec().into()
            }
        );
    }

    #[test]
    fn osc_9_9_conemu_set_current_working_directory_incomplete_desktop_notification() {
        notif("9;9", "9");
    }

    #[test]
    fn osc_9_10_conemu_xterm_keyboard_and_output_emulation_1() {
        xterm("9;10", Some(true), Some(true));
    }

    #[test]
    fn osc_9_10_conemu_xterm_keyboard_and_output_emulation_2() {
        xterm("9;10;0", Some(false), Some(false));
    }

    #[test]
    fn osc_9_10_conemu_xterm_keyboard_and_output_emulation_3() {
        xterm("9;10;1", Some(true), Some(true));
    }

    #[test]
    fn osc_9_10_conemu_xterm_keyboard_and_output_emulation_4() {
        xterm("9;10;2", None, Some(false));
    }

    #[test]
    fn osc_9_10_conemu_xterm_keyboard_and_output_emulation_5() {
        xterm("9;10;3", None, Some(true));
    }

    #[test]
    fn osc_9_10_conemu_xterm_keyboard_and_output_emulation_6() {
        notif("9;10;4", "10;4");
    }

    #[test]
    fn osc_9_10_conemu_xterm_keyboard_and_output_emulation_7() {
        notif("9;10;", "10;");
    }

    #[test]
    fn osc_9_10_conemu_xterm_keyboard_and_output_emulation_8() {
        notif("9;10;abc", "10;abc");
    }

    #[test]
    fn osc_9_11_conemu_comment() {
        assert_eq!(
            run("9;11;ab"),
            Command::ConemuComment(b"ab".to_vec().into())
        );
    }

    #[test]
    fn osc_9_11_conemu_comment_incomplete_desktop_notification() {
        notif("9;11", "11");
    }

    #[test]
    fn osc_9_12_conemu_mark_prompt_start_1() {
        assert!(matches!(run("9;12"), Command::SemanticPrompt(_)));
    }

    #[test]
    fn osc_9_12_conemu_mark_prompt_start_2() {
        assert!(matches!(run("9;12;abc"), Command::SemanticPrompt(_)));
    }
}
