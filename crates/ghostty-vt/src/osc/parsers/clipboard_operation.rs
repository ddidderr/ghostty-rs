//! OSC 52 (ZIG/src/terminal/osc/parsers/clipboard_operation.zig).
use crate::osc::{Command, Parser, State};

/// Parse OSC 52
pub fn parse(parser: &mut Parser, _terminator_ch: Option<u8>) -> Option<Command> {
    debug_assert_eq!(parser.state, State::S52);
    let data = parser.data()?;
    // Zig's `data` includes the NUL terminator, so `data.len == 1` there
    // means the payload is empty.
    if data.is_empty() {
        parser.state = State::Invalid;
        return None;
    }
    if data[0] == b';' {
        Some(Command::ClipboardContents {
            kind: b'c',
            data: data[1..].to_vec(),
        })
    } else {
        if data.len() < 2 {
            parser.state = State::Invalid;
            return None;
        }
        if data[1] != b';' {
            parser.state = State::Invalid;
            return None;
        }
        Some(Command::ClipboardContents {
            kind: data[0],
            data: data[2..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn run(alloc: bool, input: &[u8]) -> Option<Command> {
        let mut p = Parser::init(alloc);
        for c in input {
            p.next(*c);
        }
        p.end(None)
    }

    #[test]
    fn osc_52_get_set_clipboard() {
        assert_eq!(
            run(false, b"52;s;?").unwrap(),
            Command::ClipboardContents {
                kind: b's',
                data: b"?".to_vec()
            }
        );
    }

    #[test]
    fn osc_52_get_set_clipboard_optional_parameter() {
        assert_eq!(
            run(false, b"52;;?").unwrap(),
            Command::ClipboardContents {
                kind: b'c',
                data: b"?".to_vec()
            }
        );
    }

    #[test]
    fn osc_52_get_set_clipboard_with_allocator() {
        assert_eq!(
            run(true, b"52;s;?").unwrap(),
            Command::ClipboardContents {
                kind: b's',
                data: b"?".to_vec()
            }
        );
    }

    #[test]
    fn osc_52_clear_clipboard() {
        assert_eq!(
            run(false, b"52;;").unwrap(),
            Command::ClipboardContents {
                kind: b'c',
                data: Vec::new()
            }
        );
    }
}
