//! OSC 777 (ZIG/src/terminal/osc/parsers/rxvt_extension.zig).
use crate::osc::{ByteStorage, Command, Parser, State};

/// Parse OSC 777
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<Command<'_>> {
    parser.data()?;
    let (k, t) = {
        let data = parser.payload();
        let Some(k) = data.iter().position(|&b| b == b';') else {
            parser.state = State::Invalid;
            return None;
        };
        let ext = &data[..k];
        if ext != b"notify" {
            log::warn!("unknown rxvt extension: {}", String::from_utf8_lossy(ext));
            parser.state = State::Invalid;
            return None;
        }
        let Some(t) = data[k + 1..]
            .iter()
            .position(|&b| b == b';')
            .map(|i| i + k + 1)
        else {
            log::warn!("rxvt notify extension is missing the title");
            parser.state = State::Invalid;
            return None;
        };
        (k, t)
    };
    let data = parser.payload();
    Some(Command::ShowDesktopNotification {
        title: data[k + 1..t].into(),
        body: data[t + 1..].into(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn osc_777_show_desktop_notification_with_title() {
        let mut p = Parser::init(false);
        for c in b"777;notify;Title;Body" {
            p.next(*c);
        }
        assert_eq!(
            p.end(Some(0x1b)).unwrap(),
            Command::ShowDesktopNotification {
                title: b"Title".to_vec().into(),
                body: b"Body".to_vec().into()
            }
        );
    }
}
