//! OSC 0 and OSC 2 (ZIG/src/terminal/osc/parsers/change_window_title.zig).
use crate::osc::{ByteStorage, Command, Parser};

/// Parse OSC 0 and OSC 2
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<Command<'_>> {
    parser.data()?;
    Some(Command::ChangeWindowTitle(parser.payload().into()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn feed(p: &mut Parser, s: &[u8]) {
        for c in s {
            p.next(*c);
        }
    }

    #[test]
    fn osc_0_change_window_title() {
        let mut p = Parser::init(false);
        feed(&mut p, b"0;ab");
        assert_eq!(
            p.end(None).unwrap(),
            Command::ChangeWindowTitle(b"ab".to_vec().into())
        );
    }

    #[test]
    fn osc_0_longer_than_buffer() {
        let mut p = Parser::init(false);
        let mut input = b"0;".to_vec();
        input.extend(std::iter::repeat_n(b'a', crate::osc::MAX_BUF + 2));
        feed(&mut p, &input);
        assert!(p.end(None).is_none());
    }

    #[test]
    fn osc_0_one_shorter_than_buffer_length() {
        let mut p = Parser::init(false);
        let title = vec![b'a'; crate::osc::MAX_BUF - 1];
        let mut input = b"0;".to_vec();
        input.extend(&title);
        feed(&mut p, &input);
        assert_eq!(
            p.end(None).unwrap(),
            Command::ChangeWindowTitle(title.into())
        );
    }

    #[test]
    fn osc_0_exactly_at_buffer_length() {
        let mut p = Parser::init(false);
        let mut input = b"0;".to_vec();
        input.extend(vec![b'a'; crate::osc::MAX_BUF]);
        feed(&mut p, &input);
        // This should be None because we always reserve space for a null terminator.
        assert!(p.end(None).is_none());
    }

    #[test]
    fn osc_2_change_window_title_with_2() {
        let mut p = Parser::init(false);
        feed(&mut p, b"2;ab");
        assert_eq!(
            p.end(None).unwrap(),
            Command::ChangeWindowTitle(b"ab".to_vec().into())
        );
    }

    #[test]
    fn osc_2_change_window_title_with_utf8() {
        let mut p = Parser::init(false);
        feed(&mut p, b"2;");
        // '—' EM DASH U+2014 (E2 80 94)
        feed(&mut p, &[0xE2, 0x80, 0x94]);
        p.next(b' ');
        // '‐' HYPHEN U+2010 (E2 80 90)
        // Intententionally chosen to conflict with the 0x90 C1 control
        feed(&mut p, &[0xE2, 0x80, 0x90]);
        assert_eq!(
            p.end(None).unwrap(),
            Command::ChangeWindowTitle("— ‐".as_bytes().to_vec().into())
        );
    }

    #[test]
    fn osc_2_change_window_title_empty() {
        let mut p = Parser::init(false);
        feed(&mut p, b"2;");
        assert_eq!(
            p.end(None).unwrap(),
            Command::ChangeWindowTitle(Vec::new().into())
        );
    }
}
