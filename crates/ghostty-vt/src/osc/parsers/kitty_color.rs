//! OSC 21, the Kitty Color Protocol (ZIG/src/terminal/osc/parsers/kitty_color.zig).

use crate::{
    color::Rgb,
    kitty::color::{Kind, List, OSC, Request},
    osc::{ByteStorage, Command, Parser, State, Terminator},
};

/// Parse OSC 21, the Kitty Color Protocol.
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    terminator_ch: Option<u8>,
) -> Option<Command<'_>> {
    debug_assert_eq!(parser.state, State::S21);

    if !parser.alloc {
        parser.state = State::Invalid;
        return None;
    }
    if !parser.has_writer() {
        parser.state = State::Invalid;
        return None;
    }
    parser.buffered()?;

    // The parse reads the payload and appends encoded records to the
    // parser's auxiliary storage (the caller's allocator in the C API); the
    // command then borrows them.
    let (data, aux) = parser.payload_and_aux();
    aux.reset();
    let mut exceeded = false;
    let mut out = Pending {
        failed: false,
        count: 0,
    };
    for kv in data.split(|&c| c == b';') {
        if out.count >= Kind::MAX * 2 {
            log::warn!("exceeded limit for number of keys in kitty color protocol, ignoring");
            exceeded = true;
            break;
        }
        let (k, rest) = match kv.iter().position(|&c| c == b'=') {
            Some(p) => (&kv[..p], &kv[p + 1..]),
            None => (kv, &kv[kv.len()..]),
        };
        if k.is_empty() {
            log::warn!("zero length key in kitty color protocol");
            continue;
        }
        let Some(key) = Kind::parse(k) else {
            log::warn!(
                "unknown key in kitty color protocol: {}",
                String::from_utf8_lossy(k)
            );
            continue;
        };
        let value = rest.trim_ascii_matching(b' ');
        if value.is_empty() {
            out.push(aux, Request::Reset(key));
        } else if value == b"?" {
            out.push(aux, Request::Query(key));
        } else if let Ok(color) = Rgb::parse_bytes(value) {
            out.push(aux, Request::Set { key, color });
        } else {
            log::warn!(
                "invalid color format in kitty color protocol: {}",
                String::from_utf8_lossy(value)
            );
        }
    }
    if exceeded || out.failed {
        parser.state = State::Invalid;
        return None;
    }
    Some(Command::KittyColorProtocol(OSC {
        list: List::from_encoded(parser.aux_bytes()),
        terminator: Terminator::init(terminator_ch),
    }))
}

/// Bookkeeping while appending records to the parser's storage.
struct Pending {
    /// An allocation failed (the OSC cannot be represented).
    failed: bool,
    /// Number of requests so far (for the limit).
    count: usize,
}

impl Pending {
    fn push<S: ByteStorage>(&mut self, aux: &mut S, req: Request) {
        if !self.failed && aux.try_extend(&req.encode()).is_err() {
            self.failed = true;
        }
        self.count += 1;
    }
}

trait TrimMatching {
    fn trim_ascii_matching(&self, c: u8) -> &[u8];
}

impl TrimMatching for [u8] {
    fn trim_ascii_matching(&self, c: u8) -> &[u8] {
        let mut s = self;
        while let [first, rest @ ..] = s {
            if *first != c {
                break;
            }
            s = rest;
        }
        while let [rest @ .., last] = s {
            if *last != c {
                break;
            }
            s = rest;
        }
        s
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::kitty::color::Special;

    const INPUT: &str = "21;foreground=?;background=rgb:f0/f8/ff;cursor=aliceblue;cursor_text;visual_bell=;selection_foreground=#xxxyyzz;selection_background=?;selection_background=#aabbcc;2=?;3=rgbi:1.0/1.0/1.0";

    fn feed(p: &mut Parser, s: &str) {
        for ch in s.bytes() {
            p.next(ch);
        }
    }

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    #[test]
    fn osc_21_kitty_color_protocol() {
        let mut p = Parser::init(true);
        feed(&mut p, INPUT);

        let cmd = p.end(Some(0x1b)).unwrap();
        let Command::KittyColorProtocol(osc) = cmd else {
            panic!("wrong command");
        };
        let list = osc.list.to_vec();
        assert_eq!(9, list.len());
        assert_eq!(Request::Query(Kind::Special(Special::Foreground)), list[0]);
        assert_eq!(
            Request::Set {
                key: Kind::Special(Special::Background),
                color: rgb(0xf0, 0xf8, 0xff)
            },
            list[1]
        );
        assert_eq!(
            Request::Set {
                key: Kind::Special(Special::Cursor),
                color: rgb(0xf0, 0xf8, 0xff)
            },
            list[2]
        );
        assert_eq!(Request::Reset(Kind::Special(Special::CursorText)), list[3]);
        assert_eq!(Request::Reset(Kind::Special(Special::VisualBell)), list[4]);
        assert_eq!(
            Request::Query(Kind::Special(Special::SelectionBackground)),
            list[5]
        );
        assert_eq!(
            Request::Set {
                key: Kind::Special(Special::SelectionBackground),
                color: rgb(0xaa, 0xbb, 0xcc)
            },
            list[6]
        );
        assert_eq!(Request::Query(Kind::Palette(2)), list[7]);
        assert_eq!(
            Request::Set {
                key: Kind::Palette(3),
                color: rgb(0xff, 0xff, 0xff)
            },
            list[8]
        );
    }

    #[test]
    fn osc_21_kitty_color_protocol_without_allocator() {
        let mut p = Parser::init(false);
        feed(&mut p, "21;foreground=?");
        assert!(p.end(Some(0x1b)).is_none());
    }

    #[test]
    fn osc_21_kitty_color_protocol_double_reset() {
        let mut p = Parser::init(true);
        feed(&mut p, INPUT);
        let cmd = p.end(Some(0x1b)).unwrap();
        assert!(matches!(cmd, Command::KittyColorProtocol(_)));

        p.reset();
        p.reset();
    }

    #[test]
    fn osc_21_kitty_color_protocol_reset_after_invalid() {
        let mut p = Parser::init(true);
        feed(&mut p, INPUT);
        let cmd = p.end(Some(0x1b)).unwrap();
        assert!(matches!(cmd, Command::KittyColorProtocol(_)));

        p.reset();

        assert_eq!(State::Start, p.state);
        p.next(b'X');
        assert_eq!(State::Invalid, p.state);

        p.reset();
    }

    #[test]
    fn osc_21_kitty_color_protocol_no_key() {
        let mut p = Parser::init(true);
        feed(&mut p, "21;");
        let cmd = p.end(Some(0x1b)).unwrap();
        let Command::KittyColorProtocol(osc) = cmd else {
            panic!("wrong command");
        };
        assert_eq!(0, osc.list.len());
    }
}
