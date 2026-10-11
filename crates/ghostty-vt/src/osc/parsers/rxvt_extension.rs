//! OSC 777 (ZIG/src/terminal/osc/parsers/rxvt_extension.zig).
use crate::osc::{Command, Parser, State};

/// Parse OSC 777
pub fn parse(parser: &mut Parser, _terminator_ch: Option<u8>) -> Option<Command> {
    let data = parser.data()?;
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
    Some(Command::ShowDesktopNotification {
        title: data[k + 1..t].to_vec(),
        body: data[t + 1..].to_vec(),
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
                title: b"Title".to_vec(),
                body: b"Body".to_vec()
            }
        );
    }
}
