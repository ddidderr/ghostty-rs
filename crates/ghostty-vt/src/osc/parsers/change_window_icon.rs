//! OSC 1 (ZIG/src/terminal/osc/parsers/change_window_icon.zig).
use crate::osc::{ByteStorage, Command, Parser};

/// Parse OSC 1
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<Command<'_>> {
    parser.data()?;
    Some(Command::ChangeWindowIcon(parser.payload().into()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn osc_1_change_window_icon() {
        let mut p = Parser::init(false);
        for c in b"1;ab" {
            p.next(*c);
        }
        let cmd = p.end(None).unwrap();
        assert_eq!(cmd, Command::ChangeWindowIcon(b"ab".to_vec().into()));
    }
}
