//! OSC 1337 (iTerm2) (ZIG/src/terminal/osc/parsers/iterm2.zig).

use crate::osc::{Command, Parser, State};

/// iTerm2 OSC 1337 keys. Lookup is ASCII case-insensitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    AddAnnotation,
    AddHiddenAnnotation,
    Block,
    Button,
    ClearCapturedOutput,
    ClearScrollback,
    Copy,
    CopyToClipboard,
    CurrentDir,
    CursorShape,
    Custom,
    Disinter,
    EndCopy,
    File,
    FileEnd,
    FilePart,
    HighlightCursorLine,
    MultipartFile,
    OpenURL,
    PopKeyLabels,
    PushKeyLabels,
    RemoteHost,
    ReportCellSize,
    ReportVariable,
    RequestAttention,
    RequestUpload,
    SetBackgroundImageFile,
    SetBadgeFormat,
    SetColors,
    SetKeyLabel,
    SetMark,
    SetProfile,
    SetUserVar,
    ShellIntegrationVersion,
    StealFocus,
    UnicodeVersion,
}

const KEYS: &[(&str, Key)] = &[
    ("AddAnnotation", Key::AddAnnotation),
    ("AddHiddenAnnotation", Key::AddHiddenAnnotation),
    ("Block", Key::Block),
    ("Button", Key::Button),
    ("ClearCapturedOutput", Key::ClearCapturedOutput),
    ("ClearScrollback", Key::ClearScrollback),
    ("Copy", Key::Copy),
    ("CopyToClipboard", Key::CopyToClipboard),
    ("CurrentDir", Key::CurrentDir),
    ("CursorShape", Key::CursorShape),
    ("Custom", Key::Custom),
    ("Disinter", Key::Disinter),
    ("EndCopy", Key::EndCopy),
    ("File", Key::File),
    ("FileEnd", Key::FileEnd),
    ("FilePart", Key::FilePart),
    ("HighlightCursorLine", Key::HighlightCursorLine),
    ("MultipartFile", Key::MultipartFile),
    ("OpenURL", Key::OpenURL),
    ("PopKeyLabels", Key::PopKeyLabels),
    ("PushKeyLabels", Key::PushKeyLabels),
    ("RemoteHost", Key::RemoteHost),
    ("ReportCellSize", Key::ReportCellSize),
    ("ReportVariable", Key::ReportVariable),
    ("RequestAttention", Key::RequestAttention),
    ("RequestUpload", Key::RequestUpload),
    ("SetBackgroundImageFile", Key::SetBackgroundImageFile),
    ("SetBadgeFormat", Key::SetBadgeFormat),
    ("SetColors", Key::SetColors),
    ("SetKeyLabel", Key::SetKeyLabel),
    ("SetMark", Key::SetMark),
    ("SetProfile", Key::SetProfile),
    ("SetUserVar", Key::SetUserVar),
    ("ShellIntegrationVersion", Key::ShellIntegrationVersion),
    ("StealFocus", Key::StealFocus),
    ("UnicodeVersion", Key::UnicodeVersion),
];

fn lookup(s: &[u8]) -> Option<Key> {
    KEYS.iter()
        .find(|(name, _)| name.as_bytes().eq_ignore_ascii_case(s))
        .map(|(_, k)| *k)
}

/// Parse OSC 1337
/// https://iterm2.com/documentation-escape-codes.html
pub fn parse(parser: &mut Parser, _terminator_ch: Option<u8>) -> Option<Command> {
    debug_assert_eq!(parser.state, State::S1337);

    let data = parser.data()?;

    let (key_str, value): (&[u8], Option<&[u8]>) = match data.iter().position(|&b| b == b'=') {
        None => (&data[..], None),
        Some(index) => (&data[..index], Some(&data[index + 1..])),
    };

    // (Zig sets `parser.command = .invalid` and returns null on all of the
    // failure paths below.)
    let key = lookup(key_str)?;

    match key {
        Key::Copy => {
            let value = value?;

            // Sending a blank entry to clear the clipboard is an OSC 52-ism,
            // make sure that is invalid here.
            if value.is_empty() {
                return None;
            }

            // base64 value must be prefixed by a colon
            if value[0] != b':' {
                return None;
            }

            let value = &value[1..];

            // Sending a blank entry to clear the clipboard is an OSC 52-ism,
            // make sure that is invalid here.
            if value.is_empty() {
                return None;
            }

            // Sending a '?' to query the clipboard is an OSC 52-ism, make sure
            // that is invalid here.
            if value == b"?" {
                return None;
            }

            // It would be better to check for valid base64 data here, but that
            // would mean parsing the base64 data twice in the "normal" case.

            Some(Command::ClipboardContents {
                kind: b'c',
                data: value.to_vec(),
            })
        }

        Key::CurrentDir => {
            let value = value?;
            if value.is_empty() {
                return None;
            }
            Some(Command::ReportPwd {
                value: value.to_vec(),
            })
        }

        _ => {
            log::debug!("unimplemented OSC 1337: {key:?}");
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn run(input: &str) -> Option<Command> {
        let mut p = Parser::init(true);
        for ch in input.bytes() {
            p.next(ch);
        }
        p.end(Some(0x1b))
    }

    #[test]
    fn osc_1337_test_valid_unimplemented_key_with_no_value() {
        assert!(run("1337;SetBadgeFormat").is_none());
    }

    #[test]
    fn osc_1337_test_valid_unimplemented_key_with_empty_value() {
        assert!(run("1337;SetBadgeFormat=").is_none());
    }

    #[test]
    fn osc_1337_test_valid_unimplemented_key_with_non_empty_value() {
        assert!(run("1337;SetBadgeFormat=abc123").is_none());
    }

    #[test]
    fn osc_1337_test_valid_key_with_lower_case_and_with_no_value() {
        assert!(run("1337;setbadgeformat").is_none());
    }

    #[test]
    fn osc_1337_test_valid_key_with_lower_case_and_with_empty_value() {
        assert!(run("1337;setbadgeformat=").is_none());
    }

    #[test]
    fn osc_1337_test_valid_key_with_lower_case_and_with_non_empty_value() {
        assert!(run("1337;setbadgeformat=abc123").is_none());
    }

    #[test]
    fn osc_1337_test_invalid_key_with_no_value() {
        assert!(run("1337;BobrKurwa").is_none());
    }

    #[test]
    fn osc_1337_test_invalid_key_with_empty_value() {
        assert!(run("1337;BobrKurwa=").is_none());
    }

    #[test]
    fn osc_1337_test_invalid_key_with_non_empty_value() {
        assert!(run("1337;BobrKurwa=abc123").is_none());
    }

    #[test]
    fn osc_1337_test_copy_with_no_value() {
        assert!(run("1337;Copy").is_none());
    }

    #[test]
    fn osc_1337_test_copy_with_empty_value() {
        assert!(run("1337;Copy=").is_none());
    }

    #[test]
    fn osc_1337_test_copy_with_only_prefix_colon() {
        assert!(run("1337;Copy=:").is_none());
    }

    #[test]
    fn osc_1337_test_copy_with_question_mark() {
        assert!(run("1337;Copy=:?").is_none());
    }

    // Skipped in Zig too (`return error.SkipZigTest`): for performance reasons
    // base64 validity is not checked.
    #[test]
    #[ignore = "skipped in Zig: base64 is not validated for performance"]
    fn osc_1337_test_copy_with_non_empty_value_that_is_invalid_base64() {
        assert!(run("1337;Copy=:abc123").is_none());
    }

    #[test]
    fn osc_1337_test_copy_with_non_empty_value_that_is_valid_base64_but_not_prefixed_with_a_colon()
    {
        assert!(run("1337;Copy=YWJjMTIz").is_none());
    }

    #[test]
    fn osc_1337_test_copy_with_non_empty_value_that_is_valid_base64() {
        assert_eq!(
            run("1337;Copy=:YWJjMTIz"),
            Some(Command::ClipboardContents {
                kind: b'c',
                data: b"YWJjMTIz".to_vec()
            })
        );
    }

    #[test]
    fn osc_1337_test_current_dir_with_no_value() {
        assert!(run("1337;CurrentDir").is_none());
    }

    #[test]
    fn osc_1337_test_current_dir_with_empty_value() {
        assert!(run("1337;CurrentDir=").is_none());
    }

    #[test]
    fn osc_1337_test_current_dir_with_non_empty_value() {
        assert_eq!(
            run("1337;CurrentDir=abc123"),
            Some(Command::ReportPwd {
                value: b"abc123".to_vec()
            })
        );
    }
}
