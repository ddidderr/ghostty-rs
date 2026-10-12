//! SGR (Select Graphic Rendition) attribute parsing and types
//! (ZIG/src/terminal/sgr.zig).
//!
//! C-ABI helpers of the Zig version (`Attribute.C`, `cval`, `lib.Enum`
//! `Tag`) are intentionally not ported.

use crate::{
    color::{Name, Rgb},
    parser::{MAX_PARAMS, SepList},
};

/// Attribute type for SGR
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attribute<'a> {
    /// Unset all attributes
    Unset,

    /// Unknown attribute, the raw CSI command parameters are here.
    Unknown(Unknown<'a>),

    /// Bold the text.
    Bold,
    ResetBold,

    /// Italic text.
    Italic,
    ResetItalic,

    /// Faint/dim text.
    /// Note: reset faint is the same SGR code as reset bold
    Faint,

    /// Underline the text
    Underline(Underline),
    UnderlineColor(Rgb),
    /// `256_underline_color`
    UnderlineColor256(u8),
    ResetUnderlineColor,

    // Overline the text
    Overline,
    ResetOverline,

    /// Blink the text
    Blink,
    ResetBlink,

    /// Invert fg/bg colors.
    Inverse,
    ResetInverse,

    /// Invisible
    Invisible,
    ResetInvisible,

    /// Strikethrough the text.
    Strikethrough,
    ResetStrikethrough,

    /// Set foreground color as RGB values.
    DirectColorFg(Rgb),

    /// Set background color as RGB values.
    DirectColorBg(Rgb),

    /// Set the background/foreground as a named color attribute (`8_bg`).
    Bg8(Name),
    /// `8_fg`
    Fg8(Name),

    /// Reset the fg/bg to their default values.
    ResetFg,
    ResetBg,

    /// Set the background/foreground as a named bright color attribute
    /// (`8_bright_bg`).
    Bg8Bright(Name),
    /// `8_bright_fg`
    Fg8Bright(Name),

    /// Set background color as 256-color palette (`256_bg`).
    Bg256(u8),

    /// Set foreground color as 256-color palette (`256_fg`).
    Fg256(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unknown<'a> {
    /// Full is the full SGR input.
    pub full: &'a [u16],

    /// Partial is the remaining, where we got hung up.
    pub partial: &'a [u16],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Underline {
    None = 0,
    Single = 1,
    Double = 2,
    Curly = 3,
    Dotted = 4,
    Dashed = 5,
}

/// Parser parses the attributes from a list of SGR parameters. Iterating
/// yields attributes until the parameters are exhausted.
#[derive(Debug, Clone, Default)]
pub struct Parser<'a> {
    pub params: &'a [u16],
    pub params_sep: SepList,
    pub idx: usize,
}

impl<'a> Parser<'a> {
    /// Empty state parser.
    pub const EMPTY: Parser<'static> = Parser {
        params: &[],
        params_sep: SepList::init_empty(),
        idx: 0,
    };

    pub fn new(params: &'a [u16], params_sep: SepList) -> Self {
        Self {
            params,
            params_sep,
            idx: 0,
        }
    }

    /// `SepList::is_set` but tolerant of indices past the end.
    fn sep_is_set(&self, idx: usize) -> bool {
        idx < MAX_PARAMS && self.params_sep.is_set(idx)
    }

    /// Next returns the next attribute or None if there are no more attributes.
    fn next_attr(&mut self) -> Option<Attribute<'a>> {
        if self.idx >= self.params.len() {
            // Add one to ensure we don't loop on unset
            let was_zero = self.idx == 0;
            self.idx += 1;

            // If we're at index zero it means we must have an empty list
            // and an empty list implicitly means unset, otherwise we're
            // done and return None.
            return if was_zero {
                Some(Attribute::Unset)
            } else {
                None
            };
        }

        let params = self.params;
        let slice = &params[self.idx..];
        let colon = self.sep_is_set(self.idx);
        self.idx += 1;

        // Our last one will have an idx be the last value.
        if slice.is_empty() {
            return None;
        }

        // If we have a colon separator then we need to ensure we're
        // parsing a value that allows it.
        if colon {
            match slice[0] {
                4 | 38 | 48 | 58 => {}

                _ => {
                    // Consume all the colon separated
                    // values and return them as unknown.
                    let start = self.idx;
                    while self.sep_is_set(self.idx) {
                        self.idx += 1;
                    }
                    self.idx += 1;
                    return Some(Attribute::Unknown(Unknown {
                        full: params,
                        partial: &slice[..(self.idx - start + 1).min(slice.len())],
                    }));
                }
            }
        }

        match slice[0] {
            0 => return Some(Attribute::Unset),
            1 => return Some(Attribute::Bold),
            2 => return Some(Attribute::Faint),
            3 => return Some(Attribute::Italic),

            4 => {
                if colon {
                    // A trailing colon with no following sub-param
                    // (e.g. "ESC[58:4:m") leaves the colon separator
                    // bit set on the last param without adding another
                    // entry, so we can see param 4 with a colon but
                    // nothing after it.
                    if slice.len() < 2 {
                        // fall through to unknown
                    } else if self.is_colon() {
                        self.consume_unknown_colon();
                    } else {
                        self.idx += 1;
                        return Some(Attribute::Underline(match slice[1] {
                            0 => Underline::None,
                            1 => Underline::Single,
                            2 => Underline::Double,
                            3 => Underline::Curly,
                            4 => Underline::Dotted,
                            5 => Underline::Dashed,
                            // For unknown underline styles,
                            // just render a single underline.
                            _ => Underline::Single,
                        }));
                    }
                } else {
                    return Some(Attribute::Underline(Underline::Single));
                }
            }

            5 | 6 => return Some(Attribute::Blink),
            7 => return Some(Attribute::Inverse),
            8 => return Some(Attribute::Invisible),
            9 => return Some(Attribute::Strikethrough),
            21 => return Some(Attribute::Underline(Underline::Double)),
            22 => return Some(Attribute::ResetBold),
            23 => return Some(Attribute::ResetItalic),
            24 => return Some(Attribute::Underline(Underline::None)),
            25 => return Some(Attribute::ResetBlink),
            27 => return Some(Attribute::ResetInverse),
            28 => return Some(Attribute::ResetInvisible),
            29 => return Some(Attribute::ResetStrikethrough),

            30..=37 => return Some(Attribute::Fg8(Name((slice[0] - 30) as u8))),

            38 => {
                if slice.len() >= 2 {
                    match slice[1] {
                        // `2` indicates direct-color (r, g, b).
                        // We need at least 3 more params for this to make sense.
                        2 => {
                            if let Some(v) =
                                self.parse_direct_color(Attribute::DirectColorFg, slice, colon)
                            {
                                return Some(v);
                            }
                        }

                        // `5` indicates indexed color.
                        5 if slice.len() >= 3 => {
                            self.idx += 2;
                            return Some(Attribute::Fg256(slice[2] as u8));
                        }

                        _ => {}
                    }
                }
            }

            39 => return Some(Attribute::ResetFg),

            40..=47 => return Some(Attribute::Bg8(Name((slice[0] - 40) as u8))),

            48 => {
                if slice.len() >= 2 {
                    match slice[1] {
                        2 => {
                            if let Some(v) =
                                self.parse_direct_color(Attribute::DirectColorBg, slice, colon)
                            {
                                return Some(v);
                            }
                        }
                        5 if slice.len() >= 3 => {
                            self.idx += 2;
                            return Some(Attribute::Bg256(slice[2] as u8));
                        }
                        _ => {}
                    }
                }
            }

            49 => return Some(Attribute::ResetBg),

            53 => return Some(Attribute::Overline),
            55 => return Some(Attribute::ResetOverline),

            58 => {
                if slice.len() >= 2 {
                    match slice[1] {
                        2 => {
                            if let Some(v) =
                                self.parse_direct_color(Attribute::UnderlineColor, slice, colon)
                            {
                                return Some(v);
                            }
                        }
                        5 if slice.len() >= 3 => {
                            self.idx += 2;
                            return Some(Attribute::UnderlineColor256(slice[2] as u8));
                        }
                        _ => {}
                    }
                }
            }

            59 => return Some(Attribute::ResetUnderlineColor),

            // 82 instead of 90 to offset to "bright" colors
            90..=97 => return Some(Attribute::Fg8Bright(Name((slice[0] - 82) as u8))),

            100..=107 => return Some(Attribute::Bg8Bright(Name((slice[0] - 92) as u8))),

            _ => {}
        }

        Some(Attribute::Unknown(Unknown {
            full: params,
            partial: slice,
        }))
    }

    fn parse_direct_color(
        &mut self,
        make: fn(Rgb) -> Attribute<'a>,
        slice: &[u16],
        colon: bool,
    ) -> Option<Attribute<'a>> {
        // Any direct color style must have at least 5 values.
        if slice.len() < 5 {
            return None;
        }

        // Only used for direct color sets (38, 48, 58) and subparam 2.
        debug_assert_eq!(slice[1], 2);

        // Note: We truncate because the value should be 0 to 255. If
        // it isn't, the behavior is undefined so we just... truncate it.

        // If we don't have a colon, then we expect exactly 3 semicolon
        // separated values.
        if !colon {
            self.idx += 4;
            return Some(make(Rgb {
                r: slice[2] as u8,
                g: slice[3] as u8,
                b: slice[4] as u8,
            }));
        }

        // We have a colon, we might have either 5 or 6 values depending
        // on if the colorspace is present.
        match self.count_colon() {
            3 => {
                self.idx += 4;
                Some(make(Rgb {
                    r: slice[2] as u8,
                    g: slice[3] as u8,
                    b: slice[4] as u8,
                }))
            }

            4 => {
                self.idx += 5;
                Some(make(Rgb {
                    r: slice[3] as u8,
                    g: slice[4] as u8,
                    b: slice[5] as u8,
                }))
            }

            _ => {
                self.consume_unknown_colon();
                None
            }
        }
    }

    /// Returns true if the present position has a colon separator.
    /// This always returns false for the last value since it has no
    /// separator.
    fn is_colon(&self) -> bool {
        self.sep_is_set(self.idx)
    }

    fn count_colon(&self) -> usize {
        let mut count = 0;
        let mut idx = self.idx;
        while idx < self.params.len().saturating_sub(1) && self.sep_is_set(idx) {
            count += 1;
            idx += 1;
        }
        count
    }

    /// Consumes all the remaining parameters separated by a colon and
    /// returns an unknown attribute.
    fn consume_unknown_colon(&mut self) {
        let count = self.count_colon();
        self.idx += count + 1;
    }
}

impl<'a> Iterator for Parser<'a> {
    type Item = Attribute<'a>;

    fn next(&mut self) -> Option<Attribute<'a>> {
        self.next_attr()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use Attribute as A;

    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    fn test_parse(params: &[u16]) -> Attribute<'_> {
        let mut p = Parser::new(params, SepList::init_empty());
        p.next().unwrap()
    }

    fn test_parse_colon(params: &[u16]) -> Attribute<'_> {
        let mut p = Parser::new(params, SepList::init_empty());
        // Mark all parameters except the last as having a colon after.
        for i in 0..params.len() - 1 {
            p.params_sep.set(i);
        }
        p.next().unwrap()
    }

    fn sep(idxs: &[usize]) -> SepList {
        let mut l = SepList::init_empty();
        for &i in idxs {
            l.set(i);
        }
        l
    }

    fn is_unknown(a: Option<Attribute<'_>>) -> bool {
        matches!(a, Some(A::Unknown(_)))
    }

    #[test]
    fn sgr_parser() {
        assert_eq!(test_parse(&[]), A::Unset);
        assert_eq!(test_parse(&[0]), A::Unset);

        assert_eq!(
            test_parse(&[38, 2, 40, 44, 52]),
            A::DirectColorFg(rgb(40, 44, 52))
        );
        assert!(is_unknown(Some(test_parse(&[38, 2, 44, 52]))));

        assert_eq!(
            test_parse(&[48, 2, 40, 44, 52]),
            A::DirectColorBg(rgb(40, 44, 52))
        );
        assert!(is_unknown(Some(test_parse(&[48, 2, 44, 52]))));
    }

    #[test]
    fn sgr_parser_multiple() {
        let mut p = Parser::new(&[0, 38, 2, 40, 44, 52], SepList::init_empty());
        assert_eq!(p.next(), Some(A::Unset));
        assert!(matches!(p.next(), Some(A::DirectColorFg(_))));
        assert_eq!(p.next(), None);
        assert_eq!(p.next(), None);
    }

    #[test]
    fn sgr_unsupported_with_colon() {
        let mut p = Parser::new(&[0, 4, 1], sep(&[0]));
        assert!(is_unknown(p.next()));
        assert_eq!(p.next(), Some(A::Bold));
        assert_eq!(p.next(), None);
    }

    #[test]
    fn sgr_unsupported_with_multiple_colon() {
        let mut p = Parser::new(&[0, 4, 2, 1], sep(&[0, 1]));
        assert!(is_unknown(p.next()));
        assert_eq!(p.next(), Some(A::Bold));
        assert_eq!(p.next(), None);
    }

    #[test]
    fn sgr_bold() {
        assert_eq!(test_parse(&[1]), A::Bold);
        assert_eq!(test_parse(&[22]), A::ResetBold);
    }

    #[test]
    fn sgr_italic() {
        assert_eq!(test_parse(&[3]), A::Italic);
        assert_eq!(test_parse(&[23]), A::ResetItalic);
    }

    #[test]
    fn sgr_underline() {
        assert_eq!(test_parse(&[4]), A::Underline(Underline::Single));
        assert_eq!(test_parse(&[24]), A::Underline(Underline::None));
    }

    #[test]
    fn sgr_underline_styles() {
        assert_eq!(test_parse_colon(&[4, 2]), A::Underline(Underline::Double));
        assert_eq!(test_parse_colon(&[4, 0]), A::Underline(Underline::None));
        assert_eq!(test_parse_colon(&[4, 1]), A::Underline(Underline::Single));
        assert_eq!(test_parse_colon(&[4, 3]), A::Underline(Underline::Curly));
        assert_eq!(test_parse_colon(&[4, 4]), A::Underline(Underline::Dotted));
        assert_eq!(test_parse_colon(&[4, 5]), A::Underline(Underline::Dashed));
    }

    #[test]
    fn sgr_underline_style_with_more() {
        let mut p = Parser::new(&[4, 2, 1], sep(&[0]));
        assert!(matches!(p.next(), Some(A::Underline(_))));
        assert_eq!(p.next(), Some(A::Bold));
        assert_eq!(p.next(), None);
    }

    #[test]
    fn sgr_underline_style_with_too_many_colons() {
        let mut p = Parser::new(&[4, 2, 3, 1], sep(&[0, 1]));
        assert!(is_unknown(p.next()));
        assert_eq!(p.next(), Some(A::Bold));
        assert_eq!(p.next(), None);
    }

    #[test]
    fn sgr_blink() {
        assert_eq!(test_parse(&[5]), A::Blink);
        assert_eq!(test_parse(&[6]), A::Blink);
        assert_eq!(test_parse(&[25]), A::ResetBlink);
    }

    #[test]
    fn sgr_inverse() {
        assert_eq!(test_parse(&[7]), A::Inverse);
        assert_eq!(test_parse(&[27]), A::ResetInverse);
    }

    #[test]
    fn sgr_strikethrough() {
        assert_eq!(test_parse(&[9]), A::Strikethrough);
        assert_eq!(test_parse(&[29]), A::ResetStrikethrough);
    }

    #[test]
    fn sgr_8_color() {
        let mut p = Parser::new(&[31, 43, 90, 103], SepList::init_empty());
        assert_eq!(p.next(), Some(A::Fg8(Name::RED)));
        assert_eq!(p.next(), Some(A::Bg8(Name::YELLOW)));
        assert_eq!(p.next(), Some(A::Fg8Bright(Name::BRIGHT_BLACK)));
        assert_eq!(p.next(), Some(A::Bg8Bright(Name::BRIGHT_YELLOW)));
    }

    #[test]
    fn sgr_256_color() {
        let mut p = Parser::new(&[38, 5, 161, 48, 5, 236], SepList::init_empty());
        assert_eq!(p.next(), Some(A::Fg256(161)));
        assert_eq!(p.next(), Some(A::Bg256(236)));
        assert_eq!(p.next(), None);
    }

    #[test]
    fn sgr_256_color_underline() {
        let mut p = Parser::new(&[58, 5, 9], SepList::init_empty());
        assert_eq!(p.next(), Some(A::UnderlineColor256(9)));
        assert_eq!(p.next(), None);
    }

    #[test]
    fn sgr_24_bit_bg_color() {
        assert_eq!(
            test_parse_colon(&[48, 2, 1, 2, 3]),
            A::DirectColorBg(rgb(1, 2, 3))
        );
    }

    #[test]
    fn sgr_underline_color() {
        assert_eq!(
            test_parse_colon(&[58, 2, 1, 2, 3]),
            A::UnderlineColor(rgb(1, 2, 3))
        );
        assert_eq!(
            test_parse_colon(&[58, 2, 0, 1, 2, 3]),
            A::UnderlineColor(rgb(1, 2, 3))
        );
    }

    #[test]
    fn sgr_reset_underline_color() {
        let mut p = Parser::new(&[59], SepList::init_empty());
        assert_eq!(p.next(), Some(A::ResetUnderlineColor));
    }

    #[test]
    fn sgr_invisible() {
        let mut p = Parser::new(&[8, 28], SepList::init_empty());
        assert_eq!(p.next(), Some(A::Invisible));
        assert_eq!(p.next(), Some(A::ResetInvisible));
    }

    #[test]
    fn sgr_underline_bg_and_fg() {
        let mut p = Parser::new(
            &[4, 38, 2, 255, 247, 219, 48, 2, 242, 93, 147, 4],
            SepList::init_empty(),
        );
        assert_eq!(p.next(), Some(A::Underline(Underline::Single)));
        assert_eq!(p.next(), Some(A::DirectColorFg(rgb(255, 247, 219))));
        assert_eq!(p.next(), Some(A::DirectColorBg(rgb(242, 93, 147))));
        assert_eq!(p.next(), Some(A::Underline(Underline::Single)));
    }

    #[test]
    fn sgr_direct_color_fg_missing_color() {
        // This used to crash
        let p = Parser::new(&[38, 5], SepList::init_empty());
        for _ in p {}
    }

    #[test]
    fn sgr_direct_color_bg_missing_color() {
        // This used to crash
        let p = Parser::new(&[48, 5], SepList::init_empty());
        for _ in p {}
    }

    #[test]
    fn sgr_direct_fg_bg_underline_ignore_optional_color_space() {
        // These behaviors have been verified against xterm.

        // Colon version should skip the optional color space identifier
        assert_eq!(
            test_parse_colon(&[38, 2, 0, 1, 2, 3]),
            A::DirectColorFg(rgb(1, 2, 3))
        );
        assert_eq!(
            test_parse_colon(&[48, 2, 0, 1, 2, 3]),
            A::DirectColorBg(rgb(1, 2, 3))
        );
        assert_eq!(
            test_parse_colon(&[58, 2, 0, 1, 2, 3]),
            A::UnderlineColor(rgb(1, 2, 3))
        );

        // Semicolon version should not parse optional color space identifier
        assert_eq!(
            test_parse(&[38, 2, 0, 1, 2, 3]),
            A::DirectColorFg(rgb(0, 1, 2))
        );
        assert_eq!(
            test_parse(&[48, 2, 0, 1, 2, 3]),
            A::DirectColorBg(rgb(0, 1, 2))
        );
        assert_eq!(
            test_parse(&[58, 2, 0, 1, 2, 3]),
            A::UnderlineColor(rgb(0, 1, 2))
        );
    }

    #[test]
    fn sgr_direct_fg_colon_with_too_many_colons() {
        let mut p = Parser::new(&[38, 2, 0, 1, 2, 3, 4, 1], sep(&[0, 1, 2, 3, 4, 5]));
        assert!(is_unknown(p.next()));
        assert_eq!(p.next(), Some(A::Bold));
        assert_eq!(p.next(), None);
    }

    #[test]
    fn sgr_direct_fg_colon_with_colorspace_and_extra_param() {
        let mut p = Parser::new(&[38, 2, 0, 1, 2, 3, 1], sep(&[0, 1, 2, 3, 4]));
        assert_eq!(p.next(), Some(A::DirectColorFg(rgb(1, 2, 3))));
        assert_eq!(p.next(), Some(A::Bold));
        assert_eq!(p.next(), None);
    }

    #[test]
    fn sgr_direct_fg_colon_no_colorspace_and_extra_param() {
        let mut p = Parser::new(&[38, 2, 1, 2, 3, 1], sep(&[0, 1, 2, 3]));
        assert_eq!(p.next(), Some(A::DirectColorFg(rgb(1, 2, 3))));
        assert_eq!(p.next(), Some(A::Bold));
        assert_eq!(p.next(), None);
    }

    // Kakoune sent this complex SGR sequence that caused invalid behavior.
    #[test]
    fn sgr_kakoune_input() {
        // This used to crash
        let mut p = Parser::new(
            &[0, 4, 3, 38, 2, 175, 175, 215, 58, 2, 0, 190, 80, 70],
            sep(&[1, 8, 9, 10, 11, 12]),
        );
        assert_eq!(p.next(), Some(A::Unset));
        assert_eq!(p.next(), Some(A::Underline(Underline::Curly)));
        assert_eq!(p.next(), Some(A::DirectColorFg(rgb(175, 175, 215))));
        assert_eq!(p.next(), Some(A::UnderlineColor(rgb(190, 80, 70))));
    }

    // Discussion #5930, another input sent by kakoune
    #[test]
    fn sgr_kakoune_input_issue_underline_fg_and_bg() {
        // echo -e "\033[4:3;38;2;51;51;51;48;2;170;170;170;58;2;255;97;136mset everything in one sequence, broken\033[m"
        let mut p = Parser::new(
            &[
                4, 3, 38, 2, 51, 51, 51, 48, 2, 170, 170, 170, 58, 2, 255, 97, 136,
            ],
            sep(&[0]),
        );
        assert_eq!(p.next(), Some(A::Underline(Underline::Curly)));
        assert_eq!(p.next(), Some(A::DirectColorFg(rgb(51, 51, 51))));
        assert_eq!(p.next(), Some(A::DirectColorBg(rgb(170, 170, 170))));
        assert_eq!(p.next(), Some(A::UnderlineColor(rgb(255, 97, 136))));
        assert_eq!(p.next(), None);
    }

    // Fuzz crash: afl-out/stream/default/crashes/id:000021
    // Input "ESC [ 5 8 : 4 : m" produces params [58, 4] with colon
    // separator bits set at indices 0 and 1.
    #[test]
    fn sgr_underline_colon_with_trailing_separator_and_short_slice() {
        let mut p = Parser::new(&[58, 4], sep(&[0, 1]));
        // 58:4 is not a valid underline color (sub-param 4 is not 2 or 5),
        // so it falls through as unknown.
        assert!(is_unknown(p.next()));
        // Param 4 with a trailing colon but no sub-param is malformed,
        // so it also falls through as unknown rather than panicking.
        assert!(is_unknown(p.next()));
        assert_eq!(p.next(), None);
    }
}
