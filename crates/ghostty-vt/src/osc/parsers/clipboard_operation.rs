//! OSC 52 (ZIG/src/terminal/osc/parsers/clipboard_operation.zig).
use crate::osc::{ByteStorage, Command, Parser, State};

/// Parse OSC 52
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<Command<'_>> {
    debug_assert_eq!(parser.state, State::S52);
    parser.data()?;
    let (kind, skip) = {
        let data = parser.payload();
        // Zig's `data` includes the NUL terminator, so `data.len == 1` there
        // means the payload is empty.
        if data.is_empty() {
            parser.state = State::Invalid;
            return None;
        }
        if data[0] == b';' {
            (b'c', 1)
        } else {
            if data.len() < 2 || data[1] != b';' {
                parser.state = State::Invalid;
                return None;
            }
            (data[0], 2)
        }
    };
    Some(Command::ClipboardContents {
        kind,
        data: parser.payload()[skip..].into(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn run(alloc: bool, input: &[u8]) -> Option<Command<'static>> {
        let mut p = Parser::init(alloc);
        for c in input {
            p.next(*c);
        }
        p.end(None).map(Command::into_owned)
    }

    #[test]
    fn osc_52_get_set_clipboard() {
        assert_eq!(
            run(false, b"52;s;?").unwrap(),
            Command::ClipboardContents {
                kind: b's',
                data: b"?".to_vec().into()
            }
        );
    }

    #[test]
    fn osc_52_get_set_clipboard_optional_parameter() {
        assert_eq!(
            run(false, b"52;;?").unwrap(),
            Command::ClipboardContents {
                kind: b'c',
                data: b"?".to_vec().into()
            }
        );
    }

    #[test]
    fn osc_52_get_set_clipboard_with_allocator() {
        assert_eq!(
            run(true, b"52;s;?").unwrap(),
            Command::ClipboardContents {
                kind: b's',
                data: b"?".to_vec().into()
            }
        );
    }

    #[test]
    fn osc_52_clear_clipboard() {
        assert_eq!(
            run(false, b"52;;").unwrap(),
            Command::ClipboardContents {
                kind: b'c',
                data: Vec::new().into()
            }
        );
    }
}
