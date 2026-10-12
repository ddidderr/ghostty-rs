//! Terminal colors (ZIG/src/terminal/color.zig).

use crate::x11_color;

/// The default palette.
pub const DEFAULT: Palette = default_palette();

const fn default_palette() -> Palette {
    let mut result = [Rgb { r: 0, g: 0, b: 0 }; 256];

    // Named values
    let mut i: usize = 0;
    while i < 16 {
        result[i] = match Name(i as u8).default() {
            Ok(c) => c,
            Err(_) => unreachable!(),
        };
        i += 1;
    }

    // Cube
    assert!(i == 16);
    let mut r: u8 = 0;
    while r < 6 {
        let mut g: u8 = 0;
        while g < 6 {
            let mut b: u8 = 0;
            while b < 6 {
                result[i] = Rgb {
                    r: if r == 0 { 0 } else { r * 40 + 55 },
                    g: if g == 0 { 0 } else { g * 40 + 55 },
                    b: if b == 0 { 0 } else { b * 40 + 55 },
                };
                i += 1;
                b += 1;
            }
            g += 1;
        }
        r += 1;
    }

    // Gray ramp
    assert!(i == 232);
    while i < 256 {
        let value = ((i as u8 - 232) * 10) + 8;
        result[i] = Rgb {
            r: value,
            g: value,
            b: value,
        };
        i += 1;
    }

    result
}

/// Palette is the 256 color palette.
pub type Palette = [Rgb; 256];

/// Mask that can be used to set which palette indexes were set.
/// (Zig `std.StaticBitSet(256)`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaletteMask([u64; 4]);

impl PaletteMask {
    pub const fn init_empty() -> Self {
        Self([0; 4])
    }

    pub fn is_set(&self, idx: usize) -> bool {
        self.0[idx / 64] & (1 << (idx % 64)) != 0
    }

    pub fn set(&mut self, idx: usize) {
        self.0[idx / 64] |= 1 << (idx % 64);
    }

    pub fn unset(&mut self, idx: usize) {
        self.0[idx / 64] &= !(1 << (idx % 64));
    }

    pub fn count(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Iterate over set indexes in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (0..256).filter(|&i| self.is_set(i))
    }
}

/// Generate the 256-color palette from the user's base16 theme colors,
/// terminal background, and terminal foreground.
///
/// Motivation: The default 256-color palette uses fixed, fully-saturated
/// colors that clash with custom base16 themes, have poor readability in
/// dark shades (the first non-black shade jumps to 37% intensity instead
/// of the expected 20%), and exhibit inconsistent perceived brightness
/// across hues of the same shade (e.g., blue appears darker than green).
/// By generating the extended palette from the user's chosen colors,
/// programs can use the richer 256-color range without requiring their
/// own theme configuration, and light/dark switching works automatically.
///
/// The 216-color cube (indices 16–231) is built via trilinear
/// interpolation in CIELAB space over the 8 base colors. The base16
/// palette maps to the 8 corners of a 6×6×6 RGB cube as follows:
///
///   R=0 edge: bg      → base[1] (red)
///   R=5 edge: base[6] → fg
///   G=0 edge: bg/base[6] (via R) → base[2]/base[4] (green/blue via R)
///   G=5 edge: base[1]/fg (via R) → base[3]/base[5] (yellow/magenta via R)
///
/// For each R slice, four corner colors (c0–c3) are interpolated along
/// the R axis, then for each G row two edge colors (c4–c5) are
/// interpolated along G, and finally each B cell is interpolated along B
/// to produce the final color. CIELAB interpolation ensures perceptually
/// uniform brightness transitions across different hues.
///
/// The 24-step grayscale ramp (indices 232–255) is a simple linear
/// interpolation in CIELAB from the background to the foreground,
/// excluding pure black and white (available in the cube at (0,0,0)
/// and (5,5,5)). The interpolation parameter runs from 1/25 to 24/25.
///
/// Fill `skip` with user-defined color indexes to avoid replacing them.
pub fn generate_256_color(
    base: Palette,
    skip: PaletteMask,
    bg: Rgb,
    fg: Rgb,
    harmonious: bool,
) -> Palette {
    // Convert the background, foreground, and 8 base theme colors into
    // CIELAB space so that all interpolation is perceptually uniform.
    let base8_lab: [Lab; 8] = {
        let mut base8 = [
            Lab::from_rgb(bg),
            Lab::from_rgb(base[1]),
            Lab::from_rgb(base[2]),
            Lab::from_rgb(base[3]),
            Lab::from_rgb(base[4]),
            Lab::from_rgb(base[5]),
            Lab::from_rgb(base[6]),
            Lab::from_rgb(fg),
        ];

        // For light themes (where the foreground is darker than the
        // background), the cube's dark-to-light orientation is inverted
        // relative to the base color mapping. When `harmonious` is false,
        // swap bg and fg so the cube still runs from black (16) to
        // white (231).
        let is_light_theme = base8[7].l < base8[0].l;
        let invert = is_light_theme && !harmonious;
        if invert {
            base8.swap(0, 7);
        }

        base8
    };

    // Start from the base palette so indices 0-15 are preserved as-is.
    let mut result = base;

    // Build the 216-color cube (indices 16-231) via trilinear interpolation
    // in CIELAB. The three nested loops correspond to the R, G, and B axes
    // of a 6x6x6 cube.
    let mut idx: usize = 16;
    for ri in 0..6 {
        // R-axis corners: blend base colors along the red dimension.
        let tr = ri as f32 / 5.0;
        let c0 = Lab::lerp(tr, base8_lab[0], base8_lab[1]);
        let c1 = Lab::lerp(tr, base8_lab[2], base8_lab[3]);
        let c2 = Lab::lerp(tr, base8_lab[4], base8_lab[5]);
        let c3 = Lab::lerp(tr, base8_lab[6], base8_lab[7]);
        for gi in 0..6 {
            // G-axis edges: blend the R-interpolated corners along green.
            let tg = gi as f32 / 5.0;
            let c4 = Lab::lerp(tg, c0, c1);
            let c5 = Lab::lerp(tg, c2, c3);
            for bi in 0..6 {
                // B-axis: final interpolation along blue, then convert back to RGB.
                if !skip.is_set(idx) {
                    let c6 = Lab::lerp(bi as f32 / 5.0, c4, c5);
                    result[idx] = c6.to_rgb();
                }

                idx += 1;
            }
        }
    }

    // Build the 24-step grayscale ramp (indices 232-255) by linearly
    // interpolating in CIELAB from background to foreground. The parameter
    // runs from 1/25 to 24/25, excluding the endpoints which are already
    // available in the cube at (0,0,0) and (5,5,5).
    for i in 0..24 {
        let t = (i + 1) as f32 / 25.0;
        if !skip.is_set(idx) {
            let c = Lab::lerp(t, base8_lab[0], base8_lab[7]);
            result[idx] = c.to_rgb();
        }
        idx += 1;
    }

    result
}

/// A palette that can have its colors changed and reset. Purposely built
/// for terminal color operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicPalette {
    /// The current palette including any user modifications.
    pub current: Palette,

    /// The original/default palette values.
    pub original: Palette,

    /// A bitset where each bit represents whether the corresponding
    /// palette index has been modified from its default value.
    pub mask: PaletteMask,
}

impl DynamicPalette {
    pub const DEFAULT: DynamicPalette = DynamicPalette::init(DEFAULT);

    /// Initialize a dynamic palette with a default palette.
    pub const fn init(def: Palette) -> DynamicPalette {
        DynamicPalette {
            current: def,
            original: def,
            mask: PaletteMask::init_empty(),
        }
    }

    /// Set a custom color at the given palette index.
    pub fn set(&mut self, idx: u8, color: Rgb) {
        self.current[idx as usize] = color;
        self.mask.set(idx as usize);
    }

    /// Reset the color at the given palette index to its original value.
    pub fn reset(&mut self, idx: u8) {
        self.current[idx as usize] = self.original[idx as usize];
        self.mask.unset(idx as usize);
    }

    /// Reset all colors to their original values.
    pub fn reset_all(&mut self) {
        *self = Self::init(self.original);
    }

    /// Change the default palette, but preserve the changed values.
    pub fn change_default(&mut self, def: Palette) {
        self.original = def;

        // Fast path, the palette is usually not changed.
        if self.mask.count() == 0 {
            self.current = self.original;
            return;
        }

        // There are usually less set than unset, so iterate over the changed
        // values and override them.
        let mut current = def;
        for idx in self.mask.iter() {
            current[idx] = self.current[idx];
        }
        self.current = current;
    }
}

/// RGB value that can be changed and reset. This can also be totally unset
/// in every way, in which case the caller can determine their own ultimate
/// default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DynamicRgb {
    pub override_: Option<Rgb>,
    pub default: Option<Rgb>,
}

impl DynamicRgb {
    pub const UNSET: DynamicRgb = DynamicRgb {
        override_: None,
        default: None,
    };

    pub fn init(def: Rgb) -> DynamicRgb {
        DynamicRgb {
            override_: None,
            default: Some(def),
        }
    }

    pub fn get(&self) -> Option<Rgb> {
        self.override_.or(self.default)
    }

    pub fn set(&mut self, color: Rgb) {
        self.override_ = Some(color);
    }

    pub fn reset(&mut self) {
        self.override_ = self.default;
    }
}

/// Color names in the standard 8 or 16 color palette. Remainders (any other
/// `u8`) are valid unnamed values in the 256 color palette, so this is a
/// newtype rather than an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Name(pub u8);

/// Error returned by [`Name::default`] for unnamed palette values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoDefaultValue;

impl Name {
    pub const BLACK: Name = Name(0);
    pub const RED: Name = Name(1);
    pub const GREEN: Name = Name(2);
    pub const YELLOW: Name = Name(3);
    pub const BLUE: Name = Name(4);
    pub const MAGENTA: Name = Name(5);
    pub const CYAN: Name = Name(6);
    pub const WHITE: Name = Name(7);

    pub const BRIGHT_BLACK: Name = Name(8);
    pub const BRIGHT_RED: Name = Name(9);
    pub const BRIGHT_GREEN: Name = Name(10);
    pub const BRIGHT_YELLOW: Name = Name(11);
    pub const BRIGHT_BLUE: Name = Name(12);
    pub const BRIGHT_MAGENTA: Name = Name(13);
    pub const BRIGHT_CYAN: Name = Name(14);
    pub const BRIGHT_WHITE: Name = Name(15);

    /// Default colors for tagged values.
    pub const fn default(self) -> Result<Rgb, NoDefaultValue> {
        let (r, g, b) = match self.0 {
            0 => (0x1D, 0x1F, 0x21),
            1 => (0xCC, 0x66, 0x66),
            2 => (0xB5, 0xBD, 0x68),
            3 => (0xF0, 0xC6, 0x74),
            4 => (0x81, 0xA2, 0xBE),
            5 => (0xB2, 0x94, 0xBB),
            6 => (0x8A, 0xBE, 0xB7),
            7 => (0xC5, 0xC8, 0xC6),

            8 => (0x66, 0x66, 0x66),
            9 => (0xD5, 0x4E, 0x53),
            10 => (0xB9, 0xCA, 0x4A),
            11 => (0xE7, 0xC5, 0x47),
            12 => (0x7A, 0xA6, 0xDA),
            13 => (0xC3, 0x97, 0xD8),
            14 => (0x70, 0xC0, 0xB1),
            15 => (0xEA, 0xEA, 0xEA),

            _ => return Err(NoDefaultValue),
        };
        Ok(Rgb { r, g, b })
    }
}

/// The "special colors" as denoted by xterm. These can be set via
/// OSC 5 or via OSC 4 by adding the palette length to it.
///
/// https://invisible-island.net/xterm/ctlseqs/ctlseqs.html
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Special {
    Bold = 0,
    Underline = 1,
    Blink = 2,
    Reverse = 3,
    Italic = 4,
}

impl Special {
    pub fn osc4(self) -> u16 {
        // "The special colors can also be set by adding the maximum
        // number of colors (e.g., 88 or 256) to these codes in an
        // OSC 4  control" - xterm ctlseqs
        self as u16 + 256
    }

    /// Convert from the raw value (Zig `@enumFromInt` / `intToEnum`).
    pub fn from_u8(v: u8) -> Option<Special> {
        Some(match v {
            0 => Special::Bold,
            1 => Special::Underline,
            2 => Special::Blink,
            3 => Special::Reverse,
            4 => Special::Italic,
            _ => return None,
        })
    }
}

/// The "dynamic colors" as denoted by xterm. These can be set via
/// OSC 10 through 19.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Dynamic {
    Foreground = 10,
    Background = 11,
    Cursor = 12,
    PointerForeground = 13,
    PointerBackground = 14,
    TektronixForeground = 15,
    TektronixBackground = 16,
    HighlightBackground = 17,
    TektronixCursor = 18,
    HighlightForeground = 19,
}

impl Dynamic {
    /// Convert from the raw value (10..=19).
    pub fn from_u8(v: u8) -> Option<Dynamic> {
        Some(match v {
            10 => Dynamic::Foreground,
            11 => Dynamic::Background,
            12 => Dynamic::Cursor,
            13 => Dynamic::PointerForeground,
            14 => Dynamic::PointerBackground,
            15 => Dynamic::TektronixForeground,
            16 => Dynamic::TektronixBackground,
            17 => Dynamic::HighlightBackground,
            18 => Dynamic::TektronixCursor,
            19 => Dynamic::HighlightForeground,
            _ => return None,
        })
    }

    /// The next dynamic color sequentially. This is required because
    /// specifying colors sequentially without their index will automatically
    /// use the next dynamic color.
    ///
    /// "Each successive parameter changes the next color in the list.  The
    /// value of Ps tells the starting point in the list."
    pub fn next(self) -> Option<Dynamic> {
        Dynamic::from_u8(self as u8 + 1)
    }
}

/// Error returned when parsing a color specification fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidFormat;

impl std::fmt::Display for InvalidFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid color format")
    }
}

impl std::error::Error for InvalidFormat {}

/// RGB
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Alias matching the Zig name.
pub type RGB = Rgb;

/// Zig `std.fmt.parseUnsigned(u16, value, 16)`.
fn parse_unsigned_hex_u16(value: &[u8]) -> Option<u16> {
    ghostty_zigfmt::parse_unsigned::<u16>(value, 16).ok()
}

impl Rgb {
    pub fn eql(self, other: Rgb) -> bool {
        self.r == other.r && self.g == other.g && self.b == other.b
    }

    /// Calculates the contrast ratio between two colors. The contrast
    /// ration is a value between 1 and 21 where 1 is the lowest contrast
    /// and 21 is the highest contrast.
    ///
    /// https://www.w3.org/TR/WCAG20/#contrast-ratiodef
    pub fn contrast(self, other: Rgb) -> f64 {
        // pair[0] = lighter, pair[1] = darker
        let self_lum = self.luminance();
        let other_lum = other.luminance();
        let pair = if self_lum > other_lum {
            [self_lum, other_lum]
        } else {
            [other_lum, self_lum]
        };

        (pair[0] + 0.05) / (pair[1] + 0.05)
    }

    /// Calculates luminance based on the W3C formula. This returns a
    /// normalized value between 0 and 1 where 0 is black and 1 is white.
    ///
    /// https://www.w3.org/TR/WCAG20/#relativeluminancedef
    pub fn luminance(self) -> f64 {
        let r_lum = component_luminance(self.r);
        let g_lum = component_luminance(self.g);
        let b_lum = component_luminance(self.b);
        0.2126 * r_lum + 0.7152 * g_lum + 0.0722 * b_lum
    }

    /// Calculates "perceived luminance" which is better for determining
    /// light vs dark.
    ///
    /// Source: https://www.w3.org/TR/AERT/#color-contrast
    pub fn perceived_luminance(self) -> f64 {
        let r = self.r as f64;
        let g = self.g as f64;
        let b = self.b as f64;
        0.299 * (r / 255.0) + 0.587 * (g / 255.0) + 0.114 * (b / 255.0)
    }

    /// Parse a color from a floating point intensity value.
    ///
    /// The value should be between 0.0 and 1.0, inclusive.
    fn from_intensity(value: &[u8]) -> Result<u8, InvalidFormat> {
        let i: f64 = ghostty_zigfmt::parse_float::<f64>(value).map_err(|_| InvalidFormat)?;
        if !(0.0..=1.0).contains(&i) && !i.is_nan() {
            return Err(InvalidFormat);
        }

        Ok((i * 255.0) as u8)
    }

    /// Parse a color from a string of hexadecimal digits
    ///
    /// The string can contain 1, 2, 3, or 4 characters and represents the color
    /// value scaled in 4, 8, 12, or 16 bits, respectively.
    fn from_hex(value: &[u8]) -> Result<u8, InvalidFormat> {
        if value.is_empty() || value.len() > 4 {
            return Err(InvalidFormat);
        }

        let color = parse_unsigned_hex_u16(value).ok_or(InvalidFormat)?;

        let divisor: usize = match value.len() {
            1 => 0xf,
            2 => 0xff,
            3 => 0xfff,
            4 => 0xffff,
            _ => unreachable!(),
        };

        Ok((color as usize * 255 / divisor) as u8)
    }

    /// Parse a color specification.
    ///
    /// Any of the following forms are accepted:
    ///
    /// 1. rgb:<red>/<green>/<blue>
    ///
    ///    <red>, <green>, <blue> := h | hh | hhh | hhhh
    ///
    ///    where `h` is a single hexadecimal digit.
    ///
    /// 2. rgbi:<red>/<green>/<blue>
    ///
    ///    where <red>, <green>, and <blue> are floating point values between
    ///    0.0 and 1.0 (inclusive).
    ///
    /// 3. #rgb, #rrggbb, #rrrgggbbb #rrrrggggbbbb
    ///
    ///    where `r`, `g`, and `b` are a single hexadecimal digit.
    ///    These specify a color with 4, 8, 12, and 16 bits of precision
    ///    per color channel.
    pub fn parse(value: &str) -> Result<Rgb, InvalidFormat> {
        Self::parse_bytes(value.as_bytes())
    }

    /// Byte-slice variant of [`Rgb::parse`].
    pub fn parse_bytes(value: &[u8]) -> Result<Rgb, InvalidFormat> {
        if value.is_empty() {
            return Err(InvalidFormat);
        }

        if value[0] == b'#' {
            return match value.len() {
                4 => Ok(Rgb {
                    r: Rgb::from_hex(&value[1..2])?,
                    g: Rgb::from_hex(&value[2..3])?,
                    b: Rgb::from_hex(&value[3..4])?,
                }),
                7 => Ok(Rgb {
                    r: Rgb::from_hex(&value[1..3])?,
                    g: Rgb::from_hex(&value[3..5])?,
                    b: Rgb::from_hex(&value[5..7])?,
                }),
                10 => Ok(Rgb {
                    r: Rgb::from_hex(&value[1..4])?,
                    g: Rgb::from_hex(&value[4..7])?,
                    b: Rgb::from_hex(&value[7..10])?,
                }),
                13 => Ok(Rgb {
                    r: Rgb::from_hex(&value[1..5])?,
                    g: Rgb::from_hex(&value[5..9])?,
                    b: Rgb::from_hex(&value[9..13])?,
                }),
                _ => Err(InvalidFormat),
            };
        }

        // Check for X11 named colors. We allow whitespace around the edges
        // of the color because Kitty allows whitespace. This is not part of
        // any spec I could find.
        let trimmed = {
            let mut s = value;
            while let [b' ', rest @ ..] = s {
                s = rest;
            }
            while let [rest @ .., b' '] = s {
                s = rest;
            }
            s
        };
        if let Some(rgb) = x11_color::get_bytes(trimmed) {
            return Ok(rgb);
        }

        if value.len() < "rgb:a/a/a".len() || &value[0..3] != b"rgb" {
            return Err(InvalidFormat);
        }

        let mut i: usize = 3;

        let use_intensity = if value[i] == b'i' {
            i += 1;
            true
        } else {
            false
        };

        if value[i] != b':' {
            return Err(InvalidFormat);
        }

        i += 1;

        let component = |slice: &[u8]| {
            if use_intensity {
                Rgb::from_intensity(slice)
            } else {
                Rgb::from_hex(slice)
            }
        };

        let r = {
            let end = value[i..]
                .iter()
                .position(|&c| c == b'/')
                .map(|p| p + i)
                .ok_or(InvalidFormat)?;
            let slice = &value[i..end];
            i += slice.len() + 1;
            component(slice)?
        };

        let g = {
            let end = value[i..]
                .iter()
                .position(|&c| c == b'/')
                .map(|p| p + i)
                .ok_or(InvalidFormat)?;
            let slice = &value[i..end];
            i += slice.len() + 1;
            component(slice)?
        };

        let b = component(&value[i..])?;

        Ok(Rgb { r, g, b })
    }
}

/// Calculates single-component luminance based on the W3C formula.
///
/// Expects sRGB color space which at the time of writing we don't
/// generally use but it's a good enough approximation until we fix that.
/// https://www.w3.org/TR/WCAG20/#relativeluminancedef
fn component_luminance(c: u8) -> f64 {
    let normalized = c as f64 / 255.0;
    if normalized <= 0.03928 {
        return normalized / 12.92;
    }
    ((normalized + 0.055) / 1.055).powf(2.4)
}

/// LAB color space
#[derive(Debug, Clone, Copy, PartialEq)]
struct Lab {
    l: f32,
    a: f32,
    b: f32,
}

#[allow(clippy::excessive_precision)]
impl Lab {
    /// RGB to LAB
    fn from_rgb(rgb: Rgb) -> Lab {
        // Step 1: Normalize sRGB channels from [0, 255] to [0.0, 1.0].
        let mut r = rgb.r as f32 / 255.0;
        let mut g = rgb.g as f32 / 255.0;
        let mut b = rgb.b as f32 / 255.0;

        // Step 2: Apply the inverse sRGB companding (gamma correction) to
        // convert from sRGB to linear RGB.
        r = if r > 0.04045 {
            ((r + 0.055) / 1.055).powf(2.4)
        } else {
            r / 12.92
        };
        g = if g > 0.04045 {
            ((g + 0.055) / 1.055).powf(2.4)
        } else {
            g / 12.92
        };
        b = if b > 0.04045 {
            ((b + 0.055) / 1.055).powf(2.4)
        } else {
            b / 12.92
        };

        // Step 3: Convert linear RGB to CIE XYZ using the sRGB to XYZ
        // transformation matrix (D65 illuminant). The X and Z values are
        // normalized by the D65 white point reference values (Xn=0.95047,
        // Zn=1.08883; Yn=1.0 is implicit).
        let mut x = (r * 0.4124564 + g * 0.3575761 + b * 0.1804375) / 0.95047;
        let mut y = r * 0.2126729 + g * 0.7151522 + b * 0.0721750;
        let mut z = (r * 0.0193339 + g * 0.1191920 + b * 0.9503041) / 1.08883;

        // Step 4: Apply the CIE f(t) nonlinear transform to each XYZ
        // component.
        x = if x > 0.008856 {
            x.cbrt()
        } else {
            7.787 * x + 16.0 / 116.0
        };
        y = if y > 0.008856 {
            y.cbrt()
        } else {
            7.787 * y + 16.0 / 116.0
        };
        z = if z > 0.008856 {
            z.cbrt()
        } else {
            7.787 * z + 16.0 / 116.0
        };

        // Step 5: Compute the final CIELAB values from the transformed XYZ.
        Lab {
            l: 116.0 * y - 16.0,
            a: 500.0 * (x - y),
            b: 200.0 * (y - z),
        }
    }

    /// LAB to RGB
    fn to_rgb(self) -> Rgb {
        // Step 1: Recover the intermediate f(Y), f(X), f(Z) values from
        // L*a*b* by inverting the CIELAB formulas.
        let y = (self.l + 16.0) / 116.0;
        let x = self.a / 500.0 + y;
        let z = y - self.b / 200.0;

        // Step 2: Apply the inverse CIE f(t) transform to get back to XYZ.
        let x3 = x * x * x;
        let y3 = y * y * y;
        let z3 = z * z * z;
        let xf = (if x3 > 0.008856 {
            x3
        } else {
            (x - 16.0 / 116.0) / 7.787
        }) * 0.95047;
        let yf = if y3 > 0.008856 {
            y3
        } else {
            (y - 16.0 / 116.0) / 7.787
        };
        let zf = (if z3 > 0.008856 {
            z3
        } else {
            (z - 16.0 / 116.0) / 7.787
        }) * 1.08883;

        // Step 3: Convert CIE XYZ back to linear RGB.
        let mut r = xf * 3.2404542 - yf * 1.5371385 - zf * 0.4985314;
        let mut g = -xf * 0.9692660 + yf * 1.8760108 + zf * 0.0415560;
        let mut b = xf * 0.0556434 - yf * 0.2040259 + zf * 1.0572252;

        // Step 4: Apply sRGB companding (gamma correction).
        r = if r > 0.0031308 {
            1.055 * r.powf(1.0 / 2.4) - 0.055
        } else {
            12.92 * r
        };
        g = if g > 0.0031308 {
            1.055 * g.powf(1.0 / 2.4) - 0.055
        } else {
            12.92 * g
        };
        b = if b > 0.0031308 {
            1.055 * b.powf(1.0 / 2.4) - 0.055
        } else {
            12.92 * b
        };

        // Step 5: Clamp to [0.0, 1.0], scale to [0, 255], and round to
        // the nearest integer to produce the final 8-bit sRGB values.
        Rgb {
            r: (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            g: (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            b: (b.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        }
    }

    /// Linearly interpolate between two LAB colors component-wise.
    /// `t` is the interpolation factor in [0, 1]: t=0 returns `a`,
    /// t=1 returns `b`, and values in between blend proportionally.
    fn lerp(t: f32, a: Lab, b: Lab) -> Lab {
        Lab {
            l: a.l + t * (b.l - a.l),
            a: a.a + t * (b.a - a.a),
            b: a.b + t * (b.b - a.b),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    // Zig `std.fmt.parseUnsigned` / `parseFloat` differences from Rust's.
    #[test]
    fn zig_parse_hex_u16_underscore_and_sign() {
        assert_eq!(Some(0x1f), parse_unsigned_hex_u16(b"1_f"));
        assert_eq!(Some(0x1f), parse_unsigned_hex_u16(b"1__f"));
        assert_eq!(None, parse_unsigned_hex_u16(b"+f"));
        assert_eq!(None, parse_unsigned_hex_u16(b"-0"));
        assert_eq!(None, parse_unsigned_hex_u16(b"_1"));
        assert_eq!(None, parse_unsigned_hex_u16(b"f_"));
        assert_eq!(None, parse_unsigned_hex_u16(b"0x1"));
    }

    #[test]
    fn zig_parse_float_intensity() {
        // hex floats and `_` separators are valid Zig floats.
        assert_eq!(Ok(127), Rgb::from_intensity(b"0x1p-1"));
        assert_eq!(Ok(127), Rgb::from_intensity(b"0.5_0"));
        assert!(Rgb::from_intensity(b"0_.5").is_err());
        assert_eq!(Ok(127), Rgb::from_intensity(b"+.5"));
        assert_eq!(Ok(255), Rgb::from_intensity(b"1."));
        assert!(Rgb::from_intensity(b"1.5").is_err());
        assert!(Rgb::from_intensity(b"").is_err());
    }

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    #[test]
    fn special_osc4() {
        assert_eq!(256, Special::Bold.osc4());
        assert_eq!(257, Special::Underline.osc4());
        assert_eq!(258, Special::Blink.osc4());
        assert_eq!(259, Special::Reverse.osc4());
        assert_eq!(260, Special::Italic.osc4());
    }

    #[test]
    fn dynamic_next() {
        use Dynamic::*;
        assert_eq!(Some(Background), Foreground.next());
        assert_eq!(Some(Cursor), Background.next());
        assert_eq!(Some(PointerForeground), Cursor.next());
        assert_eq!(Some(PointerBackground), PointerForeground.next());
        assert_eq!(Some(TektronixForeground), PointerBackground.next());
        assert_eq!(Some(TektronixBackground), TektronixForeground.next());
        assert_eq!(Some(HighlightBackground), TektronixBackground.next());
        assert_eq!(Some(TektronixCursor), HighlightBackground.next());
        assert_eq!(Some(HighlightForeground), TektronixCursor.next());
        assert_eq!(None, HighlightForeground.next());
    }

    #[test]
    fn palette_default() {
        for i in 0..16u8 {
            assert_eq!(Name(i).default().unwrap(), DEFAULT[i as usize]);
        }
        // Spot checks of the cube and ramp.
        assert_eq!(DEFAULT[16], rgb(0, 0, 0));
        assert_eq!(DEFAULT[17], rgb(0, 0, 95));
        assert_eq!(DEFAULT[231], rgb(255, 255, 255));
        assert_eq!(DEFAULT[232], rgb(8, 8, 8));
        assert_eq!(DEFAULT[255], rgb(238, 238, 238));
    }

    #[test]
    fn rgb_parse() {
        assert_eq!(rgb(255, 0, 0), Rgb::parse("rgbi:1.0/0/0").unwrap());
        assert_eq!(rgb(127, 160, 0), Rgb::parse("rgb:7f/a0a0/0").unwrap());
        assert_eq!(rgb(255, 255, 255), Rgb::parse("rgb:f/ff/fff").unwrap());
        assert_eq!(rgb(255, 255, 255), Rgb::parse("#ffffff").unwrap());
        assert_eq!(rgb(255, 255, 255), Rgb::parse("#fff").unwrap());
        assert_eq!(rgb(255, 255, 255), Rgb::parse("#fffffffff").unwrap());
        assert_eq!(rgb(255, 255, 255), Rgb::parse("#ffffffffffff").unwrap());
        assert_eq!(rgb(255, 0, 16), Rgb::parse("#ff0010").unwrap());

        assert_eq!(rgb(0, 0, 0), Rgb::parse("black").unwrap());
        assert_eq!(rgb(255, 0, 0), Rgb::parse("red").unwrap());
        assert_eq!(rgb(0, 255, 0), Rgb::parse("green").unwrap());
        assert_eq!(rgb(0, 0, 255), Rgb::parse("blue").unwrap());
        assert_eq!(rgb(255, 255, 255), Rgb::parse("white").unwrap());

        assert_eq!(rgb(124, 252, 0), Rgb::parse("LawnGreen").unwrap());
        assert_eq!(rgb(0, 250, 154), Rgb::parse("medium spring green").unwrap());
        assert_eq!(rgb(34, 139, 34), Rgb::parse(" Forest Green ").unwrap());

        // Invalid format
        for bad in [
            "rgb;",
            "rgb:",
            ":a/a/a",
            "a/a/a",
            "rgb:a/a/a/",
            "rgb:00000///",
            "rgb:000/",
            "rgbi:a/a/a",
            "rgb:0.5/0.0/1.0",
            "rgb:not/hex/zz",
            "#",
            "#ff",
            "#ffff",
            "#fffff",
            "#gggggg",
        ] {
            assert_eq!(Err(InvalidFormat), Rgb::parse(bad), "{bad}");
        }
    }

    #[test]
    fn dynamic_palette_init() {
        let p = DynamicPalette::init(DEFAULT);
        assert_eq!(DEFAULT, p.current);
        assert_eq!(DEFAULT, p.original);
        assert_eq!(0, p.mask.count());
    }

    #[test]
    fn dynamic_palette_set() {
        let mut p = DynamicPalette::init(DEFAULT);
        let new_color = rgb(255, 0, 0);

        p.set(0, new_color);
        assert_eq!(new_color, p.current[0]);
        assert!(p.mask.is_set(0));
        assert_eq!(1, p.mask.count());

        assert_eq!(DEFAULT[0], p.original[0]);
    }

    #[test]
    fn dynamic_palette_reset() {
        let mut p = DynamicPalette::init(DEFAULT);
        p.set(0, rgb(255, 0, 0));
        assert!(p.mask.is_set(0));

        p.reset(0);
        assert_eq!(DEFAULT[0], p.current[0]);
        assert!(!p.mask.is_set(0));
        assert_eq!(0, p.mask.count());
    }

    #[test]
    fn dynamic_palette_reset_all() {
        let mut p = DynamicPalette::init(DEFAULT);
        let c = rgb(255, 0, 0);
        p.set(0, c);
        p.set(5, c);
        p.set(10, c);
        assert_eq!(3, p.mask.count());

        p.reset_all();
        assert_eq!(DEFAULT, p.current);
        assert_eq!(DEFAULT, p.original);
        assert_eq!(0, p.mask.count());
    }

    #[test]
    fn dynamic_palette_change_default_with_no_changes() {
        let mut p = DynamicPalette::init(DEFAULT);
        let mut new_palette = DEFAULT;
        new_palette[0] = rgb(100, 100, 100);

        p.change_default(new_palette);
        assert_eq!(new_palette, p.original);
        assert_eq!(new_palette, p.current);
        assert_eq!(0, p.mask.count());
    }

    #[test]
    fn dynamic_palette_change_default_preserves_changes() {
        let mut p = DynamicPalette::init(DEFAULT);
        let custom = rgb(255, 0, 0);

        p.set(5, custom);
        assert!(p.mask.is_set(5));

        let mut new_palette = DEFAULT;
        new_palette[0] = rgb(100, 100, 100);
        new_palette[5] = rgb(50, 50, 50);

        p.change_default(new_palette);

        assert_eq!(new_palette, p.original);
        assert_eq!(new_palette[0], p.current[0]);
        assert_eq!(custom, p.current[5]);
        assert!(p.mask.is_set(5));
        assert_eq!(1, p.mask.count());
    }

    #[test]
    fn dynamic_palette_change_default_with_multiple_changes() {
        let mut p = DynamicPalette::init(DEFAULT);
        let red = rgb(255, 0, 0);
        let green = rgb(0, 255, 0);
        let blue = rgb(0, 0, 255);

        p.set(1, red);
        p.set(2, green);
        p.set(3, blue);

        let mut new_palette = DEFAULT;
        new_palette[0] = rgb(50, 50, 50);
        new_palette[1] = rgb(60, 60, 60);

        p.change_default(new_palette);

        assert_eq!(new_palette[0], p.current[0]);
        assert_eq!(red, p.current[1]);
        assert_eq!(green, p.current[2]);
        assert_eq!(blue, p.current[3]);
        assert_eq!(3, p.mask.count());
    }

    fn approx(expected: f32, actual: f32) {
        assert!((expected - actual).abs() <= 0.5, "{expected} vs {actual}");
    }

    #[test]
    fn lab_from_rgb() {
        let white = Lab::from_rgb(rgb(255, 255, 255));
        approx(100.0, white.l);
        approx(0.0, white.a);
        approx(0.0, white.b);

        let black = Lab::from_rgb(rgb(0, 0, 0));
        approx(0.0, black.l);
        approx(0.0, black.a);
        approx(0.0, black.b);

        let red = Lab::from_rgb(rgb(255, 0, 0));
        approx(53.23, red.l);
        approx(80.11, red.a);
        approx(67.22, red.b);

        let green = Lab::from_rgb(rgb(0, 128, 0));
        approx(46.23, green.l);
        approx(-51.70, green.a);
        approx(49.90, green.b);

        let blue = Lab::from_rgb(rgb(0, 0, 255));
        approx(32.30, blue.l);
        approx(79.20, blue.a);
        approx(-107.86, blue.b);
    }

    #[test]
    fn generate256_color_base16_preserved() {
        let bg = rgb(0, 0, 0);
        let fg = rgb(255, 255, 255);
        let palette = generate_256_color(DEFAULT, PaletteMask::init_empty(), bg, fg, false);
        for i in 0..16 {
            assert_eq!(DEFAULT[i], palette[i]);
        }
    }

    #[test]
    fn generate256_color_cube_corners_match_base_colors() {
        let bg = rgb(0, 0, 0);
        let fg = rgb(255, 255, 255);
        let palette = generate_256_color(DEFAULT, PaletteMask::init_empty(), bg, fg, false);
        assert_eq!(bg, palette[16]);
        assert_eq!(fg, palette[231]);
    }

    #[test]
    fn generate256_color_cube_corners_black_white_with_harmonious_false() {
        let black = rgb(0, 0, 0);
        let white = rgb(255, 255, 255);

        let dark = generate_256_color(DEFAULT, PaletteMask::init_empty(), black, white, false);
        assert_eq!(black, dark[16]);
        assert_eq!(white, dark[231]);

        let light = generate_256_color(DEFAULT, PaletteMask::init_empty(), white, black, false);
        assert_eq!(black, light[16]);
        assert_eq!(white, light[231]);
    }

    #[test]
    fn generate256_color_light_theme_cube_corners_with_harmonious_true() {
        let white = rgb(255, 255, 255);
        let black = rgb(0, 0, 0);
        let palette = generate_256_color(DEFAULT, PaletteMask::init_empty(), white, black, true);
        assert_eq!(white, palette[16]);
        assert_eq!(black, palette[231]);
    }

    #[test]
    fn generate256_color_grayscale_ramp_monotonic_luminance() {
        let bg = rgb(0, 0, 0);
        let fg = rgb(255, 255, 255);
        let palette = generate_256_color(DEFAULT, PaletteMask::init_empty(), bg, fg, false);
        let mut prev_lum = 0.0;
        for c in &palette[232..256] {
            let lum = c.luminance();
            assert!(lum >= prev_lum);
            prev_lum = lum;
        }
    }

    #[test]
    fn generate256_color_skip_mask_preserves_original_colors() {
        let bg = rgb(0, 0, 0);
        let fg = rgb(255, 255, 255);

        let mut skip = PaletteMask::init_empty();
        skip.set(20);
        skip.set(100);
        skip.set(240);

        let palette = generate_256_color(DEFAULT, skip, bg, fg, false);
        assert_eq!(DEFAULT[20], palette[20]);
        assert_eq!(DEFAULT[100], palette[100]);
        assert_eq!(DEFAULT[240], palette[240]);

        assert!(!palette[21].eql(DEFAULT[21]));
    }

    #[test]
    fn generate256_color_dark_theme_harmonious_has_no_effect() {
        let bg = rgb(0, 0, 0);
        let fg = rgb(255, 255, 255);
        let normal = generate_256_color(DEFAULT, PaletteMask::init_empty(), bg, fg, false);
        let harmonious = generate_256_color(DEFAULT, PaletteMask::init_empty(), bg, fg, true);
        for i in 16..256 {
            assert_eq!(normal[i], harmonious[i]);
        }
    }

    #[test]
    fn generate256_color_light_theme_harmonious_skips_inversion() {
        let bg = rgb(255, 255, 255);
        let fg = rgb(0, 0, 0);
        let inverted = generate_256_color(DEFAULT, PaletteMask::init_empty(), bg, fg, false);
        let harmonious = generate_256_color(DEFAULT, PaletteMask::init_empty(), bg, fg, true);

        assert_eq!(bg, harmonious[16]);
        assert!(!inverted[16].eql(bg));

        let differ = (16..232)
            .filter(|&i| !inverted[i].eql(harmonious[i]))
            .count();
        assert!(differ > 0);
    }

    #[test]
    fn generate256_color_light_theme_harmonious_grayscale_ramp() {
        let bg = rgb(255, 255, 255);
        let fg = rgb(0, 0, 0);

        {
            let palette = generate_256_color(DEFAULT, PaletteMask::init_empty(), bg, fg, false);
            let mut prev_lum = 0.0;
            for c in &palette[232..256] {
                let lum = c.luminance();
                assert!(lum >= prev_lum);
                prev_lum = lum;
            }
        }

        {
            let palette = generate_256_color(DEFAULT, PaletteMask::init_empty(), bg, fg, true);
            let mut prev_lum = 1.0;
            for c in &palette[232..256] {
                let lum = c.luminance();
                assert!(lum <= prev_lum);
                prev_lum = lum;
            }
        }
    }

    #[test]
    fn lab_to_rgb() {
        let cases = [
            rgb(255, 255, 255),
            rgb(0, 0, 0),
            rgb(255, 0, 0),
            rgb(0, 128, 0),
            rgb(0, 0, 255),
            rgb(128, 128, 128),
            rgb(64, 224, 208),
        ];
        for expected in cases {
            let actual = Lab::from_rgb(expected).to_rgb();
            assert_eq!(expected, actual);
        }
    }
}
