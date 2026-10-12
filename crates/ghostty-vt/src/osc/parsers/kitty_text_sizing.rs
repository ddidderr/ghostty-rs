//! Kitty's text sizing protocol (OSC 66)
//! Specification: https://sw.kovidgoyal.net/kitty/text-sizing-protocol/
//! (ZIG/src/terminal/osc/parsers/kitty_text_sizing.zig)

use crate::osc::{ByteStorage, Command, Parser, State, encoding};

pub const MAX_PAYLOAD_LENGTH: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    #[default]
    Top,
    Bottom,
    Center,
}

impl VAlign {
    fn from_int(v: u8) -> Option<VAlign> {
        match v {
            0 => Some(VAlign::Top),
            1 => Some(VAlign::Bottom),
            2 => Some(VAlign::Center),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HAlign {
    #[default]
    Left,
    Right,
    Center,
}

impl HAlign {
    fn from_int(v: u8) -> Option<HAlign> {
        match v {
            0 => Some(HAlign::Left),
            1 => Some(HAlign::Right),
            2 => Some(HAlign::Center),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OSC<'a> {
    /// 1 - 7 (u3)
    pub scale: u8,
    /// 0 - 7 (u3), 0 means default
    pub width: u8,
    /// u4
    pub numerator: u8,
    /// u4
    pub denominator: u8,
    pub valign: VAlign,
    pub halign: HAlign,
    pub text: std::borrow::Cow<'a, [u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateError {
    UnknownKey,
    InvalidValue,
}

/// Equivalent of Zig `std.fmt.parseInt(u4, value, 10)`.
fn parse_u4(value: &[u8]) -> Option<u8> {
    let v = ghostty_zigfmt::parse_int::<i32>(value, 10).ok()?;
    u8::try_from(v).ok().filter(|v| *v <= 15)
}

impl OSC<'_> {
    pub fn into_owned(self) -> OSC<'static> {
        OSC {
            scale: self.scale,
            width: self.width,
            numerator: self.numerator,
            denominator: self.denominator,
            valign: self.valign,
            halign: self.halign,
            text: super::super::own(self.text),
        }
    }

    fn update(&mut self, key: u8, value: &[u8]) -> Result<(), UpdateError> {
        // All values are numeric, so we can do a small hack here
        let v = parse_u4(value).ok_or(UpdateError::InvalidValue)?;

        match key {
            b's' => {
                if v == 0 || v > 7 {
                    return Err(UpdateError::InvalidValue);
                }
                self.scale = v;
            }
            b'w' => {
                if v > 7 {
                    return Err(UpdateError::InvalidValue);
                }
                self.width = v;
            }
            b'n' => self.numerator = v,
            b'd' => self.denominator = v,
            b'v' => self.valign = VAlign::from_int(v).ok_or(UpdateError::InvalidValue)?,
            b'h' => self.halign = HAlign::from_int(v).ok_or(UpdateError::InvalidValue)?,
            _ => return Err(UpdateError::UnknownKey),
        }
        Ok(())
    }
}

pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<Command<'_>> {
    debug_assert_eq!(parser.state, State::S66);

    parser.data()?;

    // Validate with a short-lived borrow (marking the parser invalid cannot
    // overlap with the borrow returned in the command below).
    let payload_start = {
        let data = parser.payload();
        let Some(payload_start) = data.iter().position(|&c| c == b';') else {
            log::warn!("missing semicolon before payload");
            parser.state = State::Invalid;
            return None;
        };
        let payload = &data[payload_start + 1..];

        // Payload has to be a URL-safe UTF-8 string,
        // and be under the size limit.
        if payload.len() > MAX_PAYLOAD_LENGTH {
            log::warn!("payload is too long");
            parser.state = State::Invalid;
            return None;
        }
        if !encoding::is_safe_utf8(payload) {
            log::warn!("payload is not escape code safe UTF-8");
            parser.state = State::Invalid;
            return None;
        }
        payload_start
    };

    let data = parser.payload();
    let mut cmd = OSC {
        scale: 1,
        width: 0,
        numerator: 0,
        denominator: 0,
        valign: VAlign::Top,
        halign: HAlign::Left,
        text: data[payload_start + 1..].into(),
    };

    // Parse any arguments if given
    if payload_start > 0 {
        for kv in data[..payload_start].split(|&c| c == b':') {
            let mut it = kv.split(|&c| c == b'=');
            let Some(k) = it.next() else {
                log::warn!("missing key");
                continue;
            };
            if k.len() != 1 {
                log::warn!("key must be a single character");
                continue;
            }

            let Some(value) = it.next() else {
                log::warn!("missing value");
                continue;
            };

            if let Err(err) = cmd.update(k[0], value) {
                match err {
                    UpdateError::UnknownKey => log::warn!("unknown key: '{}'", k[0] as char),
                    UpdateError::InvalidValue => {
                        log::warn!("invalid value for key '{}': {:?}", k[0] as char, err);
                    }
                }
            }
        }
    }

    Some(Command::KittyTextSizing(cmd))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn zig_parse_u4_differences() {
        assert_eq!(Some(10), parse_u4(b"1_0"));
        assert_eq!(Some(5), parse_u4(b"+5"));
        assert_eq!(Some(0), parse_u4(b"-0"));
        assert_eq!(Some(15), parse_u4(b"15"));
        assert_eq!(None, parse_u4(b"16"));
        assert_eq!(None, parse_u4(b"5_0_0"));
        assert_eq!(None, parse_u4(b"-1"));
        assert_eq!(None, parse_u4(b"0x1"));
        assert_eq!(None, parse_u4(b"1_"));
        assert_eq!(None, parse_u4(b"99999999999"));
    }

    fn run(input: &[u8]) -> Option<Command<'static>> {
        let mut p = Parser::init(false);
        for &ch in input {
            p.next(ch);
        }
        p.end(Some(0x1b)).map(Command::into_owned)
    }

    fn sizing(input: &[u8]) -> OSC<'static> {
        match run(input) {
            Some(Command::KittyTextSizing(o)) => o,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn osc_66_empty_parameters() {
        let c = sizing(b"66;;bobr");
        assert_eq!(1, c.scale);
        assert_eq!(b"bobr", &c.text[..]);
    }

    #[test]
    fn osc_66_single_parameter() {
        let c = sizing(b"66;s=2;kurwa");
        assert_eq!(2, c.scale);
        assert_eq!(b"kurwa", &c.text[..]);
    }

    #[test]
    fn osc_66_multiple_parameters() {
        let c = sizing(b"66;s=2:w=7:n=13:d=15:v=1:h=2;long");
        assert_eq!(2, c.scale);
        assert_eq!(7, c.width);
        assert_eq!(13, c.numerator);
        assert_eq!(15, c.denominator);
        assert_eq!(VAlign::Bottom, c.valign);
        assert_eq!(HAlign::Center, c.halign);
        assert_eq!(b"long", &c.text[..]);
    }

    #[test]
    fn osc_66_scale_is_zero() {
        let c = sizing(b"66;s=0;nope");
        assert_eq!(1, c.scale);
    }

    #[test]
    fn osc_66_invalid_parameters() {
        let c = sizing(b"66;w=8:v=3:n=16;");
        assert_eq!(0, c.width);
        assert_eq!(VAlign::Top, c.valign);
        assert_eq!(0, c.numerator);
    }

    #[test]
    fn osc_66_utf8() {
        let c = sizing("66;;👻魑魅魍魉ゴースッティ".as_bytes());
        assert_eq!("👻魑魅魍魉ゴースッティ".as_bytes(), &c.text[..]);
    }

    #[test]
    fn osc_66_unsafe_utf8() {
        assert!(run(b"66;;\n").is_none());
    }

    #[test]
    fn osc_66_overlong_utf8() {
        let mut input = b"66;;".to_vec();
        input.extend(b"bobr".repeat(1025));
        assert!(run(&input).is_none());
    }
}
