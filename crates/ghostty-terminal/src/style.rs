//! Cell styles, color resolution, and VT/HTML serialization.
use std::{
    fmt,
    hash::{Hash, Hasher},
};

use ghostty_vt::{
    color::{Palette, Rgb},
    sgr::Underline,
};

use crate::page::{Cell, ContentTag};
/// Nonzero page-local style identifier.
pub type Id = u16;
/// The default style needs no set lookup.
pub const DEFAULT_ID: Id = 0;
/// A style color retains its palette or RGB origin.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    /// Unspecified terminal default.
    #[default]
    None,
    /// Index in the current palette.
    Palette(u8),
    /// Literal RGB value.
    Rgb(Rgb),
}
impl Color {
    /// Compare source and value.
    pub fn eql(self, other: Self) -> bool {
        self == other
    }
    /// Resolve a palette-dependent color.
    pub fn resolve(self, palette: &Palette) -> Option<Rgb> {
        match self {
            Self::None => None,
            Self::Palette(i) => Some(palette[usize::from(i)]),
            Self::Rgb(v) => Some(v),
        }
    }
}
/// SGR boolean flags and underline kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    /// SGR bold attribute.
    pub bold: bool,
    /// SGR italic attribute.
    pub italic: bool,
    /// SGR faint attribute.
    pub faint: bool,
    /// SGR blink attribute.
    pub blink: bool,
    /// SGR inverse attribute.
    pub inverse: bool,
    /// SGR invisible attribute.
    pub invisible: bool,
    /// SGR strikethrough attribute.
    pub strikethrough: bool,
    /// SGR overline attribute.
    pub overline: bool,
    /// Underline decoration kind.
    pub underline: Underline,
}
impl Default for Flags {
    fn default() -> Self {
        Self {
            bold: false,
            italic: false,
            faint: false,
            blink: false,
            inverse: false,
            invisible: false,
            strikethrough: false,
            overline: false,
            underline: Underline::None,
        }
    }
}
impl Hash for Flags {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bold.hash(state);
        self.italic.hash(state);
        self.faint.hash(state);
        self.blink.hash(state);
        self.inverse.hash(state);
        self.invisible.hash(state);
        self.strikethrough.hash(state);
        self.overline.hash(state);
        (self.underline as u8).hash(state);
    }
}
/// Fully self-contained cell styling.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Style {
    /// Foreground color.
    pub fg_color: Color,
    /// Background color.
    pub bg_color: Color,
    /// Underline color.
    pub underline_color: Color,
    /// SGR flags.
    pub flags: Flags,
}
/// Bold foreground policy, independent of the configuration crate.
// TODO(integration): convert ghostty-config's BoldColor at the Screen boundary.
#[derive(Debug, Clone, Copy)]
pub enum BoldColor {
    /// Use bright palette colors for the first eight entries.
    Bright,
    /// Override the default foreground when bold.
    Color(Rgb),
}
/// Inputs needed to resolve foreground colors.
#[derive(Clone, Copy)]
pub struct Fg<'a> {
    /// Terminal default foreground.
    pub default: Rgb,
    /// Current palette.
    pub palette: &'a Palette,
    /// Optional bold override.
    pub bold: Option<BoldColor>,
}
impl Style {
    /// Ghostty's packed-style hash: fold the exact 128-bit representation and
    /// apply Zig's 64-bit integer hash. This determines page-local probe order.
    pub(crate) fn zig_hash(&self) -> u64 {
        const MULTIPLIER: u64 = 0xbea225f9eb34556d;
        let mut packed = 0u128;
        for (index, color) in [self.fg_color, self.bg_color, self.underline_color]
            .into_iter()
            .enumerate()
        {
            let (tag, data) = match color {
                Color::None => (0, 0),
                Color::Palette(value) => (1, u32::from(value)),
                Color::Rgb(rgb) => (
                    2,
                    u32::from(rgb.r) | u32::from(rgb.g) << 8 | u32::from(rgb.b) << 16,
                ),
            };
            packed |= (tag as u128) << (index * 8);
            packed |= u128::from(data) << (24 + index * 24);
        }
        let flags = self.flags;
        let mut bits = 0u16;
        for (index, value) in [
            flags.bold,
            flags.italic,
            flags.faint,
            flags.blink,
            flags.inverse,
            flags.invisible,
            flags.strikethrough,
            flags.overline,
        ]
        .into_iter()
        .enumerate()
        {
            bits |= u16::from(value) << index;
        }
        bits |= (flags.underline as u16) << 8;
        packed |= u128::from(bits) << 96;
        let mut hash = packed as u64 ^ (packed >> 64) as u64;
        hash = (hash ^ (hash >> 32)).wrapping_mul(MULTIPLIER);
        hash = (hash ^ (hash >> 29)).wrapping_mul(MULTIPLIER);
        hash = (hash ^ (hash >> 32)).wrapping_mul(MULTIPLIER);
        hash ^ (hash >> 29)
    }
    /// True for default styling.
    pub fn is_default(self) -> bool {
        self == Self::default()
    }
    /// Compare all style fields.
    pub fn eql(self, other: Self) -> bool {
        self == other
    }
    /// Resolve background, including background-only cells.
    pub fn bg(self, cell: &Cell, palette: &Palette) -> Option<Rgb> {
        match cell.content_tag() {
            ContentTag::BgColorPalette => Some(palette[usize::from(cell.color_palette())]),
            ContentTag::BgColorRgb => Some(cell.color_rgb()),
            _ => self.bg_color.resolve(palette),
        }
    }
    /// Resolve foreground using bold palette and default overrides.
    pub fn fg(self, opts: Fg<'_>) -> Rgb {
        match self.fg_color {
            Color::None => {
                if self.flags.bold {
                    match opts.bold {
                        Some(BoldColor::Color(c)) => c,
                        _ => opts.default,
                    }
                } else {
                    opts.default
                }
            }
            Color::Palette(i) => {
                opts.palette[usize::from(if self.flags.bold && opts.bold.is_some() && i < 8 {
                    i + 8
                } else {
                    i
                })]
            }
            Color::Rgb(c) => {
                if self.flags.bold && c == opts.default {
                    match opts.bold {
                        Some(BoldColor::Color(v)) => v,
                        _ => c,
                    }
                } else {
                    c
                }
            }
        }
    }
    /// Resolve underline color.
    pub fn underline_color(self, palette: &Palette) -> Option<Rgb> {
        self.underline_color.resolve(palette)
    }
    /// Make a background-only cell, if a background is specified.
    pub fn bg_cell(self) -> Option<Cell> {
        let mut c = Cell::default();
        match self.bg_color {
            Color::None => return None,
            Color::Palette(i) => {
                c.set_content_tag(ContentTag::BgColorPalette);
                c.set_color_palette(i);
            }
            Color::Rgb(v) => {
                c.set_content_tag(ContentTag::BgColorRgb);
                c.set_color_rgb(v);
            }
        }
        Some(c)
    }
    /// Render self-contained SGR sequences, starting with reset.
    pub fn formatter_vt(&self) -> VtFormatter<'_> {
        VtFormatter {
            style: self,
            palette: None,
        }
    }
    /// Render inline CSS declarations.
    pub fn formatter_html(&self) -> HtmlFormatter<'_> {
        HtmlFormatter {
            style: self,
            palette: None,
        }
    }
}
/// VT sequence formatter.
pub struct VtFormatter<'a> {
    /// Style being formatted.
    pub style: &'a Style,
    /// Resolve palette entries to RGB when provided.
    pub palette: Option<&'a Palette>,
}
impl fmt::Display for VtFormatter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\x1b[0m")?;
        if self.style.flags.bold {
            f.write_str("\x1b[1m")?;
        }
        if self.style.flags.faint {
            f.write_str("\x1b[2m")?;
        }
        if self.style.flags.italic {
            f.write_str("\x1b[3m")?;
        }
        if self.style.flags.blink {
            f.write_str("\x1b[5m")?;
        }
        if self.style.flags.inverse {
            f.write_str("\x1b[7m")?;
        }
        if self.style.flags.invisible {
            f.write_str("\x1b[8m")?;
        }
        if self.style.flags.strikethrough {
            f.write_str("\x1b[9m")?;
        }
        if self.style.flags.overline {
            f.write_str("\x1b[53m")?;
        }
        f.write_str(match self.style.flags.underline {
            Underline::None => "",
            Underline::Single => "\x1b[4m",
            Underline::Double => "\x1b[4:2m",
            Underline::Curly => "\x1b[4:3m",
            Underline::Dotted => "\x1b[4:4m",
            Underline::Dashed => "\x1b[4:5m",
        })?;
        for (prefix, mut c) in [
            (38, self.style.fg_color),
            (48, self.style.bg_color),
            (58, self.style.underline_color),
        ] {
            if let (Color::Palette(i), Some(p)) = (c, self.palette) {
                c = Color::Rgb(p[usize::from(i)]);
            }
            match c {
                Color::None => {}
                Color::Palette(i) => write!(f, "\x1b[{prefix};5;{i}m")?,
                Color::Rgb(v) => write!(f, "\x1b[{prefix};2;{};{};{}m", v.r, v.g, v.b)?,
            }
        }
        Ok(())
    }
}
/// Inline CSS formatter.
pub struct HtmlFormatter<'a> {
    /// Style being formatted.
    pub style: &'a Style,
    /// Resolve palette entries to RGB when provided.
    pub palette: Option<&'a Palette>,
}
impl fmt::Display for HtmlFormatter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (prop, mut c) in [
            ("color", self.style.fg_color),
            ("background-color", self.style.bg_color),
            ("text-decoration-color", self.style.underline_color),
        ] {
            if let (Color::Palette(i), Some(p)) = (c, self.palette) {
                c = Color::Rgb(p[usize::from(i)]);
            }
            match c {
                Color::None => {}
                Color::Palette(i) => write!(f, "{prop}: var(--vt-palette-{i});")?,
                Color::Rgb(v) => write!(f, "{prop}: rgb({}, {}, {});", v.r, v.g, v.b)?,
            }
        }
        let flags = self.style.flags;
        if flags.underline != Underline::None
            || flags.strikethrough
            || flags.overline
            || flags.blink
        {
            f.write_str("text-decoration-line:")?;
            for (b, s) in [
                (flags.underline != Underline::None, " underline"),
                (flags.strikethrough, " line-through"),
                (flags.overline, " overline"),
                (flags.blink, " blink"),
            ] {
                if b {
                    f.write_str(s)?;
                }
            }
            f.write_str(";")?;
        }
        if flags.underline != Underline::None {
            write!(
                f,
                "text-decoration-style: {};",
                match flags.underline {
                    Underline::None | Underline::Single => "solid",
                    Underline::Double => "double",
                    Underline::Curly => "wavy",
                    Underline::Dotted => "dotted",
                    Underline::Dashed => "dashed",
                }
            )?;
        }
        for (b, s) in [
            (flags.bold, "font-weight: bold;"),
            (flags.italic, "font-style: italic;"),
            (flags.faint, "opacity: 0.5;"),
            (flags.invisible, "visibility: hidden;"),
            (flags.inverse, "filter: invert(100%);"),
        ] {
            if b {
                f.write_str(s)?;
            }
        }
        Ok(())
    }
}
/// Page-local deduplicated style storage.
pub type Set = crate::ref_counted_set::RefCountedSet<Style>;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_empty() {
        let mut style = Style::default();
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_bold() {
        let mut style = Style::default();
        style.flags.bold = true;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[1m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_faint() {
        let mut style = Style::default();
        style.flags.faint = true;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[2m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_italic() {
        let mut style = Style::default();
        style.flags.italic = true;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[3m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_blink() {
        let mut style = Style::default();
        style.flags.blink = true;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[5m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_inverse() {
        let mut style = Style::default();
        style.flags.inverse = true;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[7m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_invisible() {
        let mut style = Style::default();
        style.flags.invisible = true;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[8m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_strikethrough() {
        let mut style = Style::default();
        style.flags.strikethrough = true;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[9m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_overline() {
        let mut style = Style::default();
        style.flags.overline = true;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[53m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_underline_single() {
        let mut style = Style::default();
        style.flags.underline = Underline::Single;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[4m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_underline_double() {
        let mut style = Style::default();
        style.flags.underline = Underline::Double;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[4:2m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_underline_curly() {
        let mut style = Style::default();
        style.flags.underline = Underline::Curly;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[4:3m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_underline_dotted() {
        let mut style = Style::default();
        style.flags.underline = Underline::Dotted;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[4:4m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_underline_dashed() {
        let mut style = Style::default();
        style.flags.underline = Underline::Dashed;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[4:5m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_fg_palette() {
        let mut style = Style::default();
        style.fg_color = Color::Palette(42);
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[38;5;42m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_fg_rgb() {
        let mut style = Style::default();
        style.fg_color = Color::Rgb(Rgb {
            r: 255,
            g: 128,
            b: 64,
        });
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[38;2;255;128;64m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_bg_palette() {
        let mut style = Style::default();
        style.bg_color = Color::Palette(7);
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[48;5;7m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_bg_rgb() {
        let mut style = Style::default();
        style.bg_color = Color::Rgb(Rgb {
            r: 32,
            g: 64,
            b: 96,
        });
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[48;2;32;64;96m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_underline_color_palette() {
        let mut style = Style::default();
        style.underline_color = Color::Palette(15);
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[58;5;15m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_underline_color_rgb() {
        let mut style = Style::default();
        style.underline_color = Color::Rgb(Rgb {
            r: 200,
            g: 100,
            b: 50,
        });
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[58;2;200;100;50m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_multiple_flags() {
        let mut style = Style::default();
        style.flags.bold = true;
        style.flags.italic = true;
        style.flags.underline = Underline::Single;
        let mut formatter = style.formatter_vt();
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[1m\x1b[3m\x1b[4m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_all_flags() {
        let mut style = Style::default();
        style.flags.bold = true;
        style.flags.italic = true;
        style.flags.faint = true;
        style.flags.blink = true;
        style.flags.inverse = true;
        style.flags.invisible = true;
        style.flags.strikethrough = true;
        style.flags.overline = true;
        style.flags.underline = Underline::Curly;
        let mut formatter = style.formatter_vt();
        assert_eq!(
            formatter.to_string(),
            "\x1b[0m\x1b[1m\x1b[2m\x1b[3m\x1b[5m\x1b[7m\x1b[8m\x1b[9m\x1b[53m\x1b[4:3m"
        );
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_combined_colors_and_flags() {
        let mut style = Style::default();
        style.flags.bold = true;
        style.flags.italic = true;
        style.flags.underline = Underline::Double;
        style.fg_color = Color::Rgb(Rgb { r: 255, g: 0, b: 0 });
        style.bg_color = Color::Palette(8);
        style.underline_color = Color::Rgb(Rgb { r: 0, g: 255, b: 0 });
        let mut formatter = style.formatter_vt();
        assert_eq!(
            formatter.to_string(),
            "\x1b[0m\x1b[1m\x1b[3m\x1b[4:2m\x1b[38;2;255;0;0m\x1b[48;5;8m\x1b[58;2;0;255;0m"
        );
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_all_colors_rgb() {
        let mut style = Style::default();
        style.fg_color = Color::Rgb(Rgb {
            r: 10,
            g: 20,
            b: 30,
        });
        style.bg_color = Color::Rgb(Rgb {
            r: 40,
            g: 50,
            b: 60,
        });
        style.underline_color = Color::Rgb(Rgb {
            r: 70,
            g: 80,
            b: 90,
        });
        let mut formatter = style.formatter_vt();
        assert_eq!(
            formatter.to_string(),
            "\x1b[0m\x1b[38;2;10;20;30m\x1b[48;2;40;50;60m\x1b[58;2;70;80;90m"
        );
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_all_colors_palette() {
        let mut style = Style::default();
        style.fg_color = Color::Palette(1);
        style.bg_color = Color::Palette(2);
        style.underline_color = Color::Palette(3);
        let mut formatter = style.formatter_vt();
        assert_eq!(
            formatter.to_string(),
            "\x1b[0m\x1b[38;5;1m\x1b[48;5;2m\x1b[58;5;3m"
        );
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_palette_with_palette_set_emits_rgb() {
        let mut style = Style::default();
        style.fg_color = Color::Palette(1);
        let mut formatter = style.formatter_vt();
        formatter.palette = Some(&ghostty_vt::color::DEFAULT);
        assert_eq!(formatter.to_string(), "\x1b[0m\x1b[38;2;204;102;102m");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_vt_formatting_all_palette_colors_with_palette_set() {
        let mut style = Style::default();
        style.fg_color = Color::Palette(1);
        style.bg_color = Color::Palette(2);
        style.underline_color = Color::Palette(3);
        let mut formatter = style.formatter_vt();
        formatter.palette = Some(&ghostty_vt::color::DEFAULT);
        assert_eq!(
            formatter.to_string(),
            "\x1b[0m\x1b[38;2;204;102;102m\x1b[48;2;181;189;104m\x1b[58;2;240;198;116m"
        );
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_html_formatting_basic_bold() {
        let mut style = Style::default();
        style.flags.bold = true;
        let mut formatter = style.formatter_html();
        assert_eq!(formatter.to_string(), "font-weight: bold;");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_html_formatting_fg_color_rgb() {
        let mut style = Style::default();
        style.fg_color = Color::Rgb(Rgb {
            r: 255,
            g: 128,
            b: 64,
        });
        let mut formatter = style.formatter_html();
        assert_eq!(formatter.to_string(), "color: rgb(255, 128, 64);");
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_html_formatting_bg_color_palette() {
        let mut style = Style::default();
        style.bg_color = Color::Palette(7);
        let mut formatter = style.formatter_html();
        assert_eq!(
            formatter.to_string(),
            "background-color: var(--vt-palette-7);"
        );
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_html_formatting_combined_colors_and_flags() {
        let mut style = Style::default();
        style.flags.bold = true;
        style.flags.italic = true;
        style.fg_color = Color::Rgb(Rgb { r: 255, g: 0, b: 0 });
        style.bg_color = Color::Rgb(Rgb { r: 0, g: 0, b: 255 });
        let mut formatter = style.formatter_html();
        assert!(formatter.to_string().contains("color: rgb(255, 0, 0);"));
        assert!(
            formatter
                .to_string()
                .contains("background-color: rgb(0, 0, 255);")
        );
        assert!(formatter.to_string().contains("font-weight: bold;"));
        assert!(formatter.to_string().contains("font-style: italic;"));
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_html_formatting_single_decoration_line() {
        let mut style = Style::default();
        style.flags.underline = Underline::Single;
        let mut formatter = style.formatter_html();
        assert!(
            formatter
                .to_string()
                .contains("text-decoration-line: underline;")
        );
        assert!(
            formatter
                .to_string()
                .contains("text-decoration-style: solid;")
        );
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_html_formatting_multiple_decoration_lines() {
        let mut style = Style::default();
        style.flags.strikethrough = true;
        style.flags.overline = true;
        style.flags.underline = Underline::Curly;
        let mut formatter = style.formatter_html();
        assert!(
            formatter
                .to_string()
                .contains("text-decoration-line: underline line-through overline;")
        );
        assert!(
            formatter
                .to_string()
                .contains("text-decoration-style: wavy;")
        );
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_html_formatting_palette_with_palette_set_emits_rgb() {
        let mut style = Style::default();
        style.bg_color = Color::Palette(7);
        let mut formatter = style.formatter_html();
        formatter.palette = Some(&ghostty_vt::color::DEFAULT);
        assert_eq!(
            formatter.to_string(),
            "background-color: rgb(197, 200, 198);"
        );
    }
    #[test]
    #[allow(unused_mut, clippy::field_reassign_with_default)]
    fn style_html_formatting_all_palette_colors_with_palette_set() {
        let mut style = Style::default();
        style.fg_color = Color::Palette(1);
        style.bg_color = Color::Palette(2);
        style.underline_color = Color::Palette(3);
        let mut formatter = style.formatter_html();
        formatter.palette = Some(&ghostty_vt::color::DEFAULT);
        assert_eq!(
            formatter.to_string(),
            "color: rgb(204, 102, 102);background-color: rgb(181, 189, 104);text-decoration-color: rgb(240, 198, 116);"
        );
    }
    #[test]
    fn set_basic_usage() {
        let mut set = Set::init(16).unwrap();
        let a = Style {
            flags: Flags {
                bold: true,
                ..Flags::default()
            },
            ..Style::default()
        };
        let b = Style {
            flags: Flags {
                italic: true,
                ..Flags::default()
            },
            ..Style::default()
        };
        let id = set.add(a).unwrap();
        assert!(id > 0);
        assert_eq!(set.add(a).unwrap(), id);
        assert!(set.get(id).unwrap().flags.bold);
        let id2 = set.add(b).unwrap();
        assert!(set.get(id2).unwrap().flags.italic);
        assert_eq!(set.ref_count(id), 2);
        assert_eq!(set.ref_count(id2), 1);
        set.release(id).unwrap();
        set.release(id2).unwrap();
        assert_eq!(set.ref_count(id), 1);
        assert_eq!(set.ref_count(id2), 0);
        set.release(id).unwrap();
        assert_eq!(set.ref_count(id), 0);
    }
    #[test]
    fn set_capacities() {
        assert!(Set::init(16384).is_ok());
    }
}
