//! OSC 7 (ZIG/src/terminal/osc/parsers/report_pwd.zig).
use crate::osc::{ByteStorage, Command, Parser};

/// Parse OSC 7
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<Command<'_>> {
    parser.data()?;
    Some(Command::ReportPwd {
        value: parser.payload().into(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn osc_7_report_pwd() {
        let mut p = Parser::init(false);
        for c in b"7;file:///tmp/example" {
            p.next(*c);
        }
        assert_eq!(
            p.end(None).unwrap(),
            Command::ReportPwd {
                value: b"file:///tmp/example".to_vec().into()
            }
        );
    }

    #[test]
    fn osc_7_report_pwd_empty() {
        let mut p = Parser::init(false);
        for c in b"7;" {
            p.next(*c);
        }
        assert_eq!(
            p.end(None).unwrap(),
            Command::ReportPwd {
                value: Vec::new().into()
            }
        );
    }
}
