//! OSC 22 (ZIG/src/terminal/osc/parsers/mouse_shape.zig).
use crate::osc::{Command, Parser, State};

/// Parse OSC 22
pub fn parse(parser: &mut Parser, _terminator_ch: Option<u8>) -> Option<Command> {
    debug_assert_eq!(parser.state, State::S22);
    let data = parser.data()?;
    Some(Command::MouseShape { value: data })
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
                value: b"pointer".to_vec()
            }
        );
    }
}
