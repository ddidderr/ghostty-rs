//! OSC color operations: 4, 5, 10-19, 104, 105, 110-119
//! (ZIG/src/terminal/osc/parsers/color.zig).

use crate::{
    color::{Dynamic as DynamicColor, Rgb, Special as SpecialColor},
    osc::{Command, Parser, State, Terminator},
};

/// The possible operations we support for colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    Osc4,
    Osc5,
    Osc10,
    Osc11,
    Osc12,
    Osc13,
    Osc14,
    Osc15,
    Osc16,
    Osc17,
    Osc18,
    Osc19,
    Osc104,
    Osc105,
    Osc110,
    Osc111,
    Osc112,
    Osc113,
    Osc114,
    Osc115,
    Osc116,
    Osc117,
    Osc118,
    Osc119,
}

/// Parse OSCs 4, 5, 10-19, 104, 110-119
pub fn parse(parser: &mut Parser, terminator_ch: Option<u8>) -> Option<Command> {
    if !parser.alloc {
        parser.state = State::Invalid;
        return None;
    }
    // If we've collected any extra data parse that, otherwise use an empty
    // string.
    let data = if parser.has_writer() {
        parser.buffered()?
    } else {
        Vec::new()
    };
    // Check and make sure that we're parsing the correct OSCs
    let op = match parser.state {
        State::S4 => Operation::Osc4,
        State::S5 => Operation::Osc5,
        State::S10 => Operation::Osc10,
        State::S11 => Operation::Osc11,
        State::S12 => Operation::Osc12,
        State::S13 => Operation::Osc13,
        State::S14 => Operation::Osc14,
        State::S15 => Operation::Osc15,
        State::S16 => Operation::Osc16,
        State::S17 => Operation::Osc17,
        State::S18 => Operation::Osc18,
        State::S19 => Operation::Osc19,
        State::S104 => Operation::Osc104,
        State::S110 => Operation::Osc110,
        State::S111 => Operation::Osc111,
        State::S112 => Operation::Osc112,
        State::S113 => Operation::Osc113,
        State::S114 => Operation::Osc114,
        State::S115 => Operation::Osc115,
        State::S116 => Operation::Osc116,
        State::S117 => Operation::Osc117,
        State::S118 => Operation::Osc118,
        State::S119 => Operation::Osc119,
        _ => {
            parser.state = State::Invalid;
            return None;
        }
    };
    Some(Command::ColorOperation {
        op,
        requests: parse_color(op, &data),
        terminator: Terminator::init(terminator_ch),
    })
}

/// Zig `std.mem.tokenizeScalar(u8, buf, ';')`: skips empty tokens.
fn tokenize(buf: &[u8]) -> impl Iterator<Item = &[u8]> {
    buf.split(|&c| c == b';').filter(|t| !t.is_empty())
}

/// Zig `std.fmt.parseInt(u9, s, 10)`: optional sign, digits only.
fn parse_u9(s: &[u8]) -> Option<u16> {
    let (neg, digits) = match s.first()? {
        b'+' => (false, &s[1..]),
        b'-' => (true, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() {
        return None;
    }
    let mut acc: u32 = 0;
    for &c in digits {
        if !c.is_ascii_digit() {
            return None;
        }
        acc = acc * 10 + (c - b'0') as u32;
        if acc > 511 {
            return None;
        }
    }
    if neg && acc != 0 {
        return None;
    }
    Some(acc as u16)
}

/// Parse any color operation string. This should NOT include the operation
/// itself, but only the body of the operation. e.g. for "4;a;b;c" the body
/// should be "a;b;c" and the operation should be set accordingly.
///
/// Color parsing is fairly complicated so we pull this out to a specialized
/// function rather than go through our OSC parsing state machine. This is
/// much slower and requires more memory (since we need to buffer the full
/// request) but grants us an easier to understand and testable implementation.
///
/// If color changing ends up being a bottleneck we can optimize this later.
pub fn parse_color(op: Operation, buf: &[u8]) -> List {
    let mut it = tokenize(buf);
    match op {
        Operation::Osc4 => parse_get_set_ansi_color(Operation::Osc4, &mut it),
        Operation::Osc5 => parse_get_set_ansi_color(Operation::Osc5, &mut it),
        Operation::Osc104 => parse_reset_ansi_color(Operation::Osc104, &mut it),
        Operation::Osc105 => parse_reset_ansi_color(Operation::Osc105, &mut it),
        Operation::Osc10 => parse_get_set_dynamic_color(DynamicColor::Foreground, &mut it),
        Operation::Osc11 => parse_get_set_dynamic_color(DynamicColor::Background, &mut it),
        Operation::Osc12 => parse_get_set_dynamic_color(DynamicColor::Cursor, &mut it),
        Operation::Osc13 => parse_get_set_dynamic_color(DynamicColor::PointerForeground, &mut it),
        Operation::Osc14 => parse_get_set_dynamic_color(DynamicColor::PointerBackground, &mut it),
        Operation::Osc15 => parse_get_set_dynamic_color(DynamicColor::TektronixForeground, &mut it),
        Operation::Osc16 => parse_get_set_dynamic_color(DynamicColor::TektronixBackground, &mut it),
        Operation::Osc17 => parse_get_set_dynamic_color(DynamicColor::HighlightBackground, &mut it),
        Operation::Osc18 => parse_get_set_dynamic_color(DynamicColor::TektronixCursor, &mut it),
        Operation::Osc19 => parse_get_set_dynamic_color(DynamicColor::HighlightForeground, &mut it),
        Operation::Osc110 => parse_reset_dynamic_color(DynamicColor::Foreground, &mut it),
        Operation::Osc111 => parse_reset_dynamic_color(DynamicColor::Background, &mut it),
        Operation::Osc112 => parse_reset_dynamic_color(DynamicColor::Cursor, &mut it),
        Operation::Osc113 => parse_reset_dynamic_color(DynamicColor::PointerForeground, &mut it),
        Operation::Osc114 => parse_reset_dynamic_color(DynamicColor::PointerBackground, &mut it),
        Operation::Osc115 => parse_reset_dynamic_color(DynamicColor::TektronixForeground, &mut it),
        Operation::Osc116 => parse_reset_dynamic_color(DynamicColor::TektronixBackground, &mut it),
        Operation::Osc117 => parse_reset_dynamic_color(DynamicColor::HighlightBackground, &mut it),
        Operation::Osc118 => parse_reset_dynamic_color(DynamicColor::TektronixCursor, &mut it),
        Operation::Osc119 => parse_reset_dynamic_color(DynamicColor::HighlightForeground, &mut it),
    }
}

/// Map a parsed numeric color to a target for OSC 4/5/104/105.
/// `special_only` is true for OSC 5/105; otherwise 0-255 is the palette and
/// 256+ is the special offset.
fn ansi_target(color: u16, special_only: bool) -> Option<Target> {
    if special_only {
        // std.math.cast(u3, color) then intToEnum
        let v = u8::try_from(color).ok().filter(|&v| v < 8)?;
        return SpecialColor::from_u8(v).map(Target::Special);
    }
    if let Ok(idx) = u8::try_from(color) {
        return Some(Target::Palette(idx));
    }
    let v = u8::try_from(color - 256).ok().filter(|&v| v < 8)?;
    SpecialColor::from_u8(v).map(Target::Special)
}

/// OSC 4/5
fn parse_get_set_ansi_color<'a>(op: Operation, it: &mut impl Iterator<Item = &'a [u8]>) -> List {
    // Note: in ANY error scenario below we return the accumulated results.
    // This matches the xterm behavior (see misc.c ChangeAnsiColorRequest)

    let mut result = List::new();
    loop {
        // We expect a `c; spec` pair. If either doesn't exist then
        // we return the results up to this point.
        let Some(color_str) = it.next() else {
            return result;
        };
        let Some(spec_str) = it.next() else {
            return result;
        };

        // Color must be numeric. u9 because that'll fit our palette + special
        let Some(color) = parse_u9(color_str) else {
            return result;
        };

        // Parse the color.
        let Some(target) = ansi_target(color, op == Operation::Osc5) else {
            return result;
        };

        // "?" always results in a query.
        if spec_str == b"?" {
            result.push(Request::Query(target));
            continue;
        }

        let Ok(rgb) = Rgb::parse_bytes(spec_str) else {
            return result;
        };
        result.push(Request::Set(ColoredTarget { target, color: rgb }));
    }
}

/// OSC 104/105: Reset ANSI Colors
fn parse_reset_ansi_color<'a>(op: Operation, it: &mut impl Iterator<Item = &'a [u8]>) -> List {
    // Note: xterm stops parsing the reset list on any error, but we're
    // more flexible and try the next value. This matches the behavior of
    // Kitty and I don't see a downside to being more flexible here. Hopefully
    // no one depends on the exact behavior of xterm.

    let mut result = List::new();
    loop {
        let Some(color_str) = it.next() else {
            // If no parameters are given, we reset the full table.
            if result.is_empty() {
                result.push(match op {
                    Operation::Osc104 => Request::ResetPalette,
                    Operation::Osc105 => Request::ResetSpecial,
                    _ => unreachable!(),
                });
            }
            return result;
        };

        // Empty color strings are ignored, not treated as an error.
        if color_str.is_empty() {
            continue;
        }

        // Color must be numeric. u9 because that'll fit our palette + special
        let Some(color) = parse_u9(color_str) else {
            continue;
        };

        // Parse the color.
        let Some(target) = ansi_target(color, op == Operation::Osc105) else {
            continue;
        };

        result.push(Request::Reset(target));
    }
}

/// OSC 10-19: Get/Set Dynamic Colors
fn parse_get_set_dynamic_color<'a>(
    start: DynamicColor,
    it: &mut impl Iterator<Item = &'a [u8]>,
) -> List {
    // Note: in ANY error scenario below we return the accumulated results.
    // This matches the xterm behavior (see misc.c ChangeColorsRequest)

    let mut result = List::new();
    let mut color = start;
    loop {
        let Some(spec_str) = it.next() else {
            return result;
        };

        if spec_str == b"?" {
            result.push(Request::Query(Target::Dynamic(color)));
        } else {
            let Ok(rgb) = Rgb::parse_bytes(spec_str) else {
                return result;
            };
            result.push(Request::Set(ColoredTarget {
                target: Target::Dynamic(color),
                color: rgb,
            }));
        }

        // Each successive value uses the next color so long as it exists.
        match color.next() {
            Some(n) => color = n,
            None => return result,
        }
    }
}

/// OSC 110-119: Reset Dynamic Colors
fn parse_reset_dynamic_color<'a>(
    color: DynamicColor,
    it: &mut impl Iterator<Item = &'a [u8]>,
) -> List {
    let mut result = List::new();
    if it.next().is_some() {
        return result;
    }
    result.push(Request::Reset(Target::Dynamic(color)));
    result
}

/// The list of requests in a color OSC. (Zig uses a SegmentedList to
/// avoid copying; a `Vec` is equivalent.)
pub type List = Vec<Request>;

/// A single operation related to the terminal color palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Set(ColoredTarget),
    Query(Target),
    Reset(Target),
    ResetPalette,
    ResetSpecial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Palette(u8),
    Special(SpecialColor),
    Dynamic(DynamicColor),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColoredTarget {
    pub target: Target,
    pub color: Rgb,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const RED: Rgb = Rgb { r: 255, g: 0, b: 0 };
    const BLUE: Rgb = Rgb { r: 0, g: 0, b: 255 };

    fn set(target: Target, color: Rgb) -> Request {
        Request::Set(ColoredTarget { target, color })
    }

    const SPECIALS: [SpecialColor; 5] = [
        SpecialColor::Bold,
        SpecialColor::Underline,
        SpecialColor::Blink,
        SpecialColor::Reverse,
        SpecialColor::Italic,
    ];

    const DYNAMICS: [(DynamicColor, Operation, Operation); 10] = [
        (
            DynamicColor::Foreground,
            Operation::Osc10,
            Operation::Osc110,
        ),
        (
            DynamicColor::Background,
            Operation::Osc11,
            Operation::Osc111,
        ),
        (DynamicColor::Cursor, Operation::Osc12, Operation::Osc112),
        (
            DynamicColor::PointerForeground,
            Operation::Osc13,
            Operation::Osc113,
        ),
        (
            DynamicColor::PointerBackground,
            Operation::Osc14,
            Operation::Osc114,
        ),
        (
            DynamicColor::TektronixForeground,
            Operation::Osc15,
            Operation::Osc115,
        ),
        (
            DynamicColor::TektronixBackground,
            Operation::Osc16,
            Operation::Osc116,
        ),
        (
            DynamicColor::HighlightBackground,
            Operation::Osc17,
            Operation::Osc117,
        ),
        (
            DynamicColor::TektronixCursor,
            Operation::Osc18,
            Operation::Osc118,
        ),
        (
            DynamicColor::HighlightForeground,
            Operation::Osc19,
            Operation::Osc119,
        ),
    ];

    #[test]
    fn osc_4_empty_param() {
        let mut p = Parser::init(false);
        for ch in "4;;".bytes() {
            p.next(ch);
        }
        assert!(p.end(Some(0x1b)).is_none());
    }

    #[test]
    fn osc_4() {
        // Test every palette index
        for idx in 0..u8::MAX {
            // printf '\e]4;0;red\\'
            let list = parse_color(Operation::Osc4, format!("{idx};red").as_bytes());
            assert_eq!(vec![set(Target::Palette(idx), RED)], list);

            // printf '\e]4;0;?\\'
            let list = parse_color(Operation::Osc4, format!("{idx};?").as_bytes());
            assert_eq!(vec![Request::Query(Target::Palette(idx))], list);

            // Trailing invalid data produces results up to that point
            let list = parse_color(Operation::Osc4, format!("{idx};red;").as_bytes());
            assert_eq!(vec![set(Target::Palette(idx), RED)], list);

            // Whitespace doesn't produce a working value in xterm but we
            // allow it because Kitty does and it seems harmless.
            let list = parse_color(Operation::Osc4, format!("{idx};red ").as_bytes());
            assert_eq!(vec![set(Target::Palette(idx), RED)], list);
        }

        // Test every special color
        for (i, special) in SPECIALS.iter().enumerate() {
            let list = parse_color(Operation::Osc4, format!("{};red", 256 + i).as_bytes());
            assert_eq!(vec![set(Target::Special(*special), RED)], list);
        }
    }

    #[test]
    fn osc_5() {
        for (i, special) in SPECIALS.iter().enumerate() {
            let list = parse_color(Operation::Osc5, format!("{i};red").as_bytes());
            assert_eq!(vec![set(Target::Special(*special), RED)], list);
        }
    }

    #[test]
    fn osc_4_multiple_requests() {
        let list = parse_color(Operation::Osc4, b"0;red;1;blue");
        assert_eq!(
            vec![set(Target::Palette(0), RED), set(Target::Palette(1), BLUE)],
            list
        );

        // Multiple requests with same index overwrite each other
        let list = parse_color(Operation::Osc4, b"0;red;0;blue");
        assert_eq!(
            vec![set(Target::Palette(0), RED), set(Target::Palette(0), BLUE)],
            list
        );
    }

    #[test]
    fn osc_104() {
        for idx in 0..u8::MAX {
            let list = parse_color(Operation::Osc104, idx.to_string().as_bytes());
            assert_eq!(vec![Request::Reset(Target::Palette(idx))], list);
        }

        for (i, special) in SPECIALS.iter().enumerate() {
            let list = parse_color(Operation::Osc104, (256 + i).to_string().as_bytes());
            assert_eq!(vec![Request::Reset(Target::Special(*special))], list);
        }
    }

    #[test]
    fn osc_104_empty_index() {
        let list = parse_color(Operation::Osc104, b"0;;1");
        assert_eq!(
            vec![
                Request::Reset(Target::Palette(0)),
                Request::Reset(Target::Palette(1))
            ],
            list
        );
    }

    #[test]
    fn osc_104_invalid_index() {
        let list = parse_color(Operation::Osc104, b"ffff;1");
        assert_eq!(vec![Request::Reset(Target::Palette(1))], list);
    }

    #[test]
    fn osc_104_reset_all() {
        let list = parse_color(Operation::Osc104, b"");
        assert_eq!(vec![Request::ResetPalette], list);
    }

    #[test]
    fn osc_105_reset_all() {
        let list = parse_color(Operation::Osc105, b"");
        assert_eq!(vec![Request::ResetSpecial], list);
    }

    // OSC 10-19: Get/Set Dynamic Colors
    #[test]
    fn osc_10_to_19_dynamic() {
        for (color, op, _) in DYNAMICS {
            // printf '\e]10;red\e\\'
            let list = parse_color(op, b"red");
            assert_eq!(vec![set(Target::Dynamic(color), RED)], list);
        }
    }

    #[test]
    fn osc_10_to_19_dynamic_multiple() {
        // printf '\e]11;red;blue\e\\'
        let list = parse_color(Operation::Osc11, b"red;blue");
        assert_eq!(
            vec![
                set(Target::Dynamic(DynamicColor::Background), RED),
                set(Target::Dynamic(DynamicColor::Cursor), BLUE)
            ],
            list
        );
    }

    // OSC 110-119: Reset Dynamic Colors
    #[test]
    fn osc_110_to_119_reset_dynamic() {
        for (color, _, op) in DYNAMICS {
            let expected = vec![Request::Reset(Target::Dynamic(color))];

            // printf '\e]110\e\\'
            assert_eq!(expected, parse_color(op, b""));

            // xterm allows a trailing semicolon.
            assert_eq!(expected, parse_color(op, b";"));

            // xterm does NOT allow any whitespace
            assert_eq!(0, parse_color(op, b" ").len());
        }
    }

    #[test]
    fn osc_112_incomplete_sequence() {
        // https://github.com/darrenstarr/VtNetCore/pull/14
        let mut p = Parser::init(true);
        for ch in b"112" {
            p.next(*ch);
        }
        let cmd = p.end(Some(0x07)).unwrap();
        let Command::ColorOperation {
            op,
            requests,
            terminator,
        } = cmd
        else {
            panic!("wrong command");
        };
        assert_eq!(Terminator::Bel, terminator);
        assert_eq!(Operation::Osc112, op);
        assert_eq!(
            vec![Request::Reset(Target::Dynamic(DynamicColor::Cursor))],
            requests
        );
    }
}
