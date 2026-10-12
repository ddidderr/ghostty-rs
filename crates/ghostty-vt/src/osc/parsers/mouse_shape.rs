//! OSC 22 (ZIG/src/terminal/osc/parsers/mouse_shape.zig).
use crate::osc::{ByteStorage, Command, Parser, State};

/// Parse OSC 22
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<Command<'_>> {
    debug_assert_eq!(parser.state, State::S22);
    parser.data()?;
    Some(Command::MouseShape {
        value: parser.payload().into(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn osc_22_pointer_cursor() {
        let mut p = Parser::init(false);
        for c in b"22;pointer" {
            p.next(*c);
        }
        assert_eq!(
            p.end(None).unwrap(),
            Command::MouseShape {
                value: b"pointer".to_vec().into()
            }
        );
    }
}
