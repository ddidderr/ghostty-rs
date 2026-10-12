//! Config field types that were nested in `Config.zig` (part A): simple
//! enums, working directory, colors, palette, repeatable strings, word
//! chars, font variation / codepoint maps, font style, links.

use crate::cli_args::{ArgError, ConfigField, parse_int_base0, trim_ws};
use crate::clipboard_codepoint_map::{ClipboardCodepointMap, Entry as ClipboardEntry, Replacement};
use crate::font_types::{CodepointMap, CodepointMapEntry, Descriptor, Variation, VariationId};
use crate::formatter::EntryFormatter;
use crate::{config_enum, config_packed};
use ghostty_vt::color::{DEFAULT as DEFAULT_PALETTE, Palette as TermPalette, PaletteMask, Rgb};
use std::fmt;

/// Write `name = value\n` for a string-ish value.
fn write_str(f: &mut EntryFormatter<'_>, value: &str) -> fmt::Result {
    writeln!(f.writer, "{} = {}", f.name, value)
}

/// Write the `void` form of an entry (`name = \n`).
fn write_void(f: &mut EntryFormatter<'_>) -> fmt::Result {
    writeln!(f.writer, "{} = ", f.name)
}

// ---------------------------------------------------------------------
// Cursor style: the canonical type is `ghostty_terminal::cursor::Style`
// (ZIG/src/terminal/cursor.zig `Style`); config only adds the field impl.

pub use ghostty_terminal::cursor::Style as CursorStyle;

impl ConfigField for CursorStyle {
    const VALID_VALUES: Option<&'static [&'static str]> =
        Some(&["bar", "block", "underline", "block_hollow"]);

    fn parse_cli(&mut self, value: Option<&str>) -> Result<(), ArgError> {
        let v = value.ok_or(ArgError::ValueRequired)?;
        *self = match v {
            "bar" => CursorStyle::Bar,
            "block" => CursorStyle::Block,
            "underline" => CursorStyle::Underline,
            "block_hollow" => CursorStyle::BlockHollow,
            _ => return Err(ArgError::InvalidValue),
        };
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        let name = match self {
            CursorStyle::Bar => "bar",
            CursorStyle::Block => "block",
            CursorStyle::Underline => "underline",
            CursorStyle::BlockHollow => "block_hollow",
        };
        writeln!(f.writer, "{} = {}", f.name, name)
    }
}

// ---------------------------------------------------------------------
// Simple enums

config_enum! {
    pub enum ConfirmCloseSurface {
        False => "false",
        True => "true",
        Always => "always",
    }
}

config_enum! {
    /// Valid values for custom-shader-animation
    pub enum CustomShaderAnimation {
        False => "false",
        True => "true",
        Always => "always",
    }
}

config_enum! {
    /// Valid values for macos-non-native-fullscreen
    pub enum NonNativeFullscreen {
        False => "false",
        True => "true",
        VisibleMenu => "visible-menu",
        PaddedNotch => "padded-notch",
    }
}

config_enum! {
    /// Valid values for fullscreen config option
    pub enum Fullscreen {
        False => "false",
        True => "true",
        NonNative => "non-native",
        NonNativeVisibleMenu => "non-native-visible-menu",
        NonNativePaddedNotch => "non-native-padded-notch",
    }
}

config_enum! {
    pub enum WindowPaddingColor {
        Background => "background",
        Extend => "extend",
        ExtendAlways => "extend-always",
    }
}

config_enum! {
    pub enum WindowSubtitle {
        False => "false",
        WorkingDirectory => "working-directory",
    }
}

config_enum! {
    pub enum LinkPreviews {
        False => "false",
        True => "true",
        Osc8 => "osc8",
    }
}

// ---------------------------------------------------------------------
// WorkingDirectory

/// See working-directory
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WorkingDirectory {
    /// Resolve to the current user's home directory during config finalize.
    Home,

    /// Inherit the working directory from the launching process.
    #[default]
    Inherit,

    /// Use an explicit working directory path. This may be not be
    /// expanded until finalize is called.
    Path(String),
}

impl ConfigField for WorkingDirectory {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        let mut input = input.trim_matches(|c: char| {
            // std.ascii.whitespace
            matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
        });
        if input.is_empty() {
            return Err(ArgError::ValueRequired);
        }

        // Match path.zig behavior for quoted values.
        if input.len() >= 2 && input.starts_with('"') && input.ends_with('"') {
            input = &input[1..input.len() - 1];
        }

        *self = match input {
            "home" => WorkingDirectory::Home,
            "inherit" => WorkingDirectory::Inherit,
            _ => WorkingDirectory::Path(input.to_string()),
        };
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        match self {
            WorkingDirectory::Home => write_str(f, "home"),
            WorkingDirectory::Inherit => write_str(f, "inherit"),
            WorkingDirectory::Path(p) => write_str(f, p),
        }
    }
}

impl WorkingDirectory {
    /// Expand tilde paths in .path values.
    pub fn finalize(&mut self) {
        let WorkingDirectory::Path(path) = self else {
            return;
        };

        if !path.starts_with("~/") {
            return;
        }

        let expanded = match ghostty_os::expand_home(path) {
            Ok(e) => e,
            Err(err) => {
                log::warn!(
                    "error expanding home directory for working-directory path={path}: {err}"
                );
                return;
            }
        };

        if expanded == *path {
            return;
        }
        *self = WorkingDirectory::Path(expanded);
    }

    #[must_use]
    pub fn value(&self) -> Option<&str> {
        match self {
            WorkingDirectory::Path(p) => Some(p),
            WorkingDirectory::Home | WorkingDirectory::Inherit => None,
        }
    }
}

// ---------------------------------------------------------------------
// Colors

/// Color represents a color using RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    /// Convert this to the terminal RGB struct
    #[must_use]
    pub fn to_terminal_rgb(self) -> Rgb {
        Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }

    /// Zig: `Color.parseCLI(input)` (static form returning a new value).
    pub fn from_cli(input: Option<&str>) -> Result<Color, ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        // Trim any whitespace before processing
        let trimmed = trim_ws(input);

        if let Some(rgb) = ghostty_vt::x11_color::get(trimmed) {
            return Ok(Color {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
            });
        }

        Color::from_hex(trimmed)
    }

    /// Format the color as a string.
    #[must_use]
    pub fn format_buf(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// fromHex parses a color from a hex value such as #RRGGBB. The "#"
    /// is optional.
    pub fn from_hex(input: &str) -> Result<Color, ArgError> {
        // Trim the beginning '#' if it exists
        let trimmed = input.strip_prefix('#').unwrap_or(input).as_bytes();
        if trimmed.len() != 6 && trimmed.len() != 3 {
            return Err(ArgError::InvalidValue);
        }

        // Expand short hex values to full hex values
        let rgb: Vec<u8> = if trimmed.len() == 3 {
            vec![
                trimmed[0], trimmed[0], trimmed[1], trimmed[1], trimmed[2], trimmed[2],
            ]
        } else {
            trimmed.to_vec()
        };

        let digit = |c: u8| -> Result<u8, ArgError> {
            (c as char)
                .to_digit(16)
                .map(|d| d as u8)
                .ok_or_else(|| ArgError::other("InvalidCharacter"))
        };
        let byte =
            |i: usize| -> Result<u8, ArgError> { Ok(digit(rgb[i])? * 16 + digit(rgb[i + 1])?) };

        Ok(Color {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
        })
    }
}

impl ConfigField for Color {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        *self = Color::from_cli(input)?;
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        write_str(f, &self.format_buf())
    }
}

/// Represents color values that can also reference special color
/// values such as "cell-foreground" or "cell-background".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalColor {
    Color(Color),
    CellForeground,
    CellBackground,
}

impl Default for TerminalColor {
    fn default() -> Self {
        TerminalColor::Color(Color::default())
    }
}

impl TerminalColor {
    /// Zig: `TerminalColor.parseCLI(input)`.
    pub fn from_cli(input: Option<&str>) -> Result<TerminalColor, ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        match input {
            "cell-foreground" => Ok(TerminalColor::CellForeground),
            "cell-background" => Ok(TerminalColor::CellBackground),
            _ => Ok(TerminalColor::Color(Color::from_cli(Some(input))?)),
        }
    }

    #[must_use]
    pub fn to_terminal_rgb(self) -> Option<Rgb> {
        match self {
            TerminalColor::Color(v) => Some(v.to_terminal_rgb()),
            TerminalColor::CellForeground | TerminalColor::CellBackground => None,
        }
    }
}

impl ConfigField for TerminalColor {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        *self = TerminalColor::from_cli(input)?;
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        match self {
            TerminalColor::Color(c) => c.format_entry(f),
            TerminalColor::CellForeground => write_str(f, "cell-foreground"),
            TerminalColor::CellBackground => write_str(f, "cell-background"),
        }
    }
}

/// Represents color values that can be used for bold. See `bold-color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoldColor {
    Color(Color),
    Bright,
}

impl Default for BoldColor {
    fn default() -> Self {
        BoldColor::Color(Color::default())
    }
}

impl BoldColor {
    /// Zig: `BoldColor.parseCLI(input)`.
    pub fn from_cli(input: Option<&str>) -> Result<BoldColor, ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        if input == "bright" {
            return Ok(BoldColor::Bright);
        }
        Ok(BoldColor::Color(Color::from_cli(Some(input))?))
    }
}

impl ConfigField for BoldColor {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        *self = BoldColor::from_cli(input)?;
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        match self {
            BoldColor::Color(c) => c.format_entry(f),
            BoldColor::Bright => write_str(f, "bright"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ColorList {
    pub colors: Vec<Color>,
}

impl ConfigField for ColorList {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        if input.is_empty() {
            return Err(ArgError::ValueRequired);
        }

        // Always reset on parse
        *self = ColorList::default();

        // Split the input by commas and parse each color
        let mut count = 0usize;
        for raw in input.split(',').filter(|s| !s.is_empty()) {
            count += 1;
            if count > 64 {
                return Err(ArgError::InvalidValue);
            }

            // Trim whitespace from each color value
            let trimmed = trim_ws(raw);
            let color = Color::from_cli(Some(trimmed))?;
            self.colors.push(color);
        }

        // If no colors were parsed, we need to return an error
        if self.colors.is_empty() {
            return Err(ArgError::InvalidValue);
        }
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        // If no items, we want to render an empty field.
        if self.colors.is_empty() {
            return write_void(f);
        }
        let parts: Vec<String> = self.colors.iter().map(|c| c.format_buf()).collect();
        write_str(f, &parts.join(","))
    }
}

/// Palette is the 256 color palette for 256-color mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    /// The actual value that is updated as we parse.
    pub value: TermPalette,

    /// Keep track of which indexes were manually set by the user.
    pub mask: PaletteMask,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            value: DEFAULT_PALETTE,
            mask: PaletteMask::init_empty(),
        }
    }
}

impl ConfigField for Palette {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let value = input.ok_or(ArgError::ValueRequired)?;
        let eql_idx = value.find('=').ok_or(ArgError::InvalidValue)?;

        // Parse the key part (trim whitespace)
        let key_str = trim_ws(&value[..eql_idx]);
        let key = match parse_int_base0(key_str) {
            None => return Err(ArgError::other("InvalidCharacter")),
            Some(n) if n < 0 => return Err(ArgError::other("InvalidCharacter")),
            Some(n) if n > 255 => return Err(ArgError::other("Overflow")),
            Some(n) => n as usize,
        };

        // Parse the color part (Color parsing will handle whitespace)
        let rgb = Color::from_cli(Some(&value[eql_idx + 1..]))?;
        self.value[key] = rgb.to_terminal_rgb();
        self.mask.set(key);
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        for (k, v) in self.value.iter().enumerate() {
            write_str(f, &format!("{}=#{:02x}{:02x}{:02x}", k, v.r, v.g, v.b))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------
// RepeatableString

/// RepeatableString is a string value that can be repeated to accumulate
/// a list of strings. This isn't called "StringList" because I find that
/// sometimes leads to confusion that it _accepts_ a list such as
/// comma-separated values.
#[derive(Debug, Clone, Default)]
pub struct RepeatableString {
    pub list: Vec<String>,

    /// If true, then the next value will clear the list and start over
    /// rather than append. This is a bit of a hack but is here to make
    /// the font-family set of configurations work with CLI parsing.
    pub overwrite_next: bool,
}

/// Compare if two of our value are equal (Zig `equal`: only the items
/// are compared, not `overwrite_next`).
impl PartialEq for RepeatableString {
    fn eq(&self, other: &Self) -> bool {
        self.list == other.list
    }
}

impl RepeatableString {
    /// The number of items in the list
    #[must_use]
    pub fn count(&self) -> usize {
        self.list.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

impl ConfigField for RepeatableString {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let value = input.ok_or(ArgError::ValueRequired)?;

        // Empty value resets the list
        if value.is_empty() {
            self.list.clear();
            return Ok(());
        }

        // If we're overwriting then we clear before appending
        if self.overwrite_next {
            self.list.clear();
            self.overwrite_next = false;
        }

        self.list.push(value.to_string());
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        // If no items, we want to render an empty field.
        if self.list.is_empty() {
            return write_void(f);
        }
        for v in &self.list {
            write_str(f, v)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------
// SelectionWordChars

/// SelectionWordChars stores the parsed codepoints for word boundary
/// characters used during text selection. The string is parsed once
/// during configuration and stored as codepoints for efficient
/// lookup during selection operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionWordChars {
    /// The parsed codepoints. Always includes null (U+0000) at index 0.
    pub codepoints: Vec<u32>,
}

impl SelectionWordChars {
    /// Default boundary characters: ` \t'"│`|:;,()[]{}<>$`
    pub const DEFAULT_CODEPOINTS: [u32; 20] = [
        0,           // null
        ' ' as u32,  // space
        '\t' as u32, // tab
        '\'' as u32, // single quote
        '"' as u32,  // double quote
        '│' as u32,  // U+2502 box drawing
        '`' as u32,  // backtick
        '|' as u32,  // pipe
        ':' as u32,  // colon
        ';' as u32,  // semicolon
        ',' as u32,  // comma
        '(' as u32,  // left paren
        ')' as u32,  // right paren
        '[' as u32,  // left bracket
        ']' as u32,  // right bracket
        '{' as u32,  // left brace
        '}' as u32,  // right brace
        '<' as u32,  // less than
        '>' as u32,  // greater than
        '$' as u32,  // dollar
    ];
}

impl Default for SelectionWordChars {
    fn default() -> Self {
        Self {
            codepoints: Self::DEFAULT_CODEPOINTS.to_vec(),
        }
    }
}

impl ConfigField for SelectionWordChars {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let value = input.ok_or(ArgError::ValueRequired)?;

        // Always include null as first boundary
        let mut list = vec![0u32];
        let mut it = crate::string::codepoint_iterator(value);
        while let Some(cp) = it.next().map_err(|_| ArgError::InvalidValue)? {
            list.push(cp);
        }
        self.codepoints = list;
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        // Convert codepoints back to UTF-8 string for display, skipping the
        // null character at index 0.
        let mut s = String::new();
        for &cp in self.codepoints.iter().skip(1) {
            let Some(ch) = char::from_u32(cp) else {
                continue;
            };
            if s.len() + ch.len_utf8() > 4096 {
                break;
            }
            s.push(ch);
        }
        write_str(f, &s)
    }
}

// ---------------------------------------------------------------------
// RepeatableFontVariation

/// FontVariation is a repeatable configuration value that sets a single
/// font variation value. Font variations are configurations for what
/// are often called "variable fonts." The font files usually end in
/// "-VF.ttf."
///
/// The value for this is in the format of `id=value` where `id` is the
/// 4-character font variation axis identifier and `value` is the
/// floating point value for that axis. For more details on font variations
/// see the MDN font-variation-settings documentation since this copies that
/// behavior almost exactly:
///
/// https://developer.mozilla.org/en-US/docs/Web/CSS/font-variation-settings
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RepeatableFontVariation {
    pub list: Vec<Variation>,
}

impl ConfigField for RepeatableFontVariation {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        let eql_idx = input.find('=').ok_or(ArgError::InvalidValue)?;
        let key = trim_ws(&input[..eql_idx]);
        let value = trim_ws(&input[eql_idx + 1..]);
        let key: &[u8; 4] = key
            .as_bytes()
            .try_into()
            .map_err(|_| ArgError::InvalidValue)?;
        self.list.push(Variation {
            id: VariationId::init(key),
            value: value.parse::<f64>().map_err(|_| ArgError::InvalidValue)?,
        });
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        if self.list.is_empty() {
            return write_void(f);
        }
        for v in &self.list {
            let id = v.id.str();
            write_str(f, &format!("{}={}", String::from_utf8_lossy(&id), v.value))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Codepoint maps

/// Parses the list of Unicode codepoint ranges. Valid syntax:
///
///   "" (empty returns null)
///   U+1234
///   U+1234-5678
///   U+1234,U+5678
///   U+1234,U+5678-U+9ABC
///   U+1234-5678,U+5678
///
/// etc.
pub(crate) struct UnicodeRangeParser<'a> {
    input: &'a [u8],
    i: usize,
}

impl<'a> UnicodeRangeParser<'a> {
    pub(crate) fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            i: 0,
        }
    }

    pub(crate) fn next_range(&mut self) -> Result<Option<[u32; 2]>, ArgError> {
        // Once we're EOF then we're done without an error.
        if self.eof() {
            return Ok(None);
        }

        // One codepoint no matter what
        let start = self.parse_codepoint()?;
        if self.eof() {
            return Ok(Some([start, start]));
        }

        // We're allowed to have any whitespace here
        self.consume_whitespace();

        // Otherwise we expect either a range or a comma
        match self.input.get(self.i) {
            // Comma means we have another codepoint but in a different
            // range so we return our current codepoint.
            Some(b',') => {
                self.advance();
                self.consume_whitespace();
                if self.eof() {
                    return Err(ArgError::InvalidValue);
                }
                Ok(Some([start, start]))
            }

            // Hyphen means we have a range.
            Some(b'-') => {
                self.advance();
                self.consume_whitespace();
                if self.eof() {
                    return Err(ArgError::InvalidValue);
                }
                let end = self.parse_codepoint()?;
                self.consume_whitespace();
                if !self.eof() && self.input[self.i] != b',' {
                    return Err(ArgError::InvalidValue);
                }
                self.advance();
                self.consume_whitespace();
                if start > end {
                    return Err(ArgError::InvalidValue);
                }
                Ok(Some([start, end]))
            }

            // Note: the Zig code indexes input[i] here, which can be past
            // the end after trailing whitespace; that is safety-checked UB
            // in Zig, we report an invalid value instead.
            _ => Err(ArgError::InvalidValue),
        }
    }

    fn consume_whitespace(&mut self) {
        while !self.eof() {
            match self.input[self.i] {
                b' ' | b'\t' => self.advance(),
                _ => return,
            }
        }
    }

    fn parse_codepoint(&mut self) -> Result<u32, ArgError> {
        if self.input.get(self.i) != Some(&b'U') {
            return Err(ArgError::InvalidValue);
        }
        self.advance();
        if self.eof() {
            return Err(ArgError::InvalidValue);
        }
        if self.input[self.i] != b'+' {
            return Err(ArgError::InvalidValue);
        }
        self.advance();
        if self.eof() {
            return Err(ArgError::InvalidValue);
        }

        let start_i = self.i;
        loop {
            let current = self.input[self.i];
            if !current.is_ascii_hexdigit() {
                break;
            }

            // Advance but break on EOF
            self.advance();
            if self.eof() {
                break;
            }
        }

        // If we didn't consume a single character, we have an error.
        if start_i == self.i {
            return Err(ArgError::InvalidValue);
        }

        parse_u21_hex(&self.input[start_i..self.i]).ok_or(ArgError::InvalidValue)
    }

    fn advance(&mut self) {
        self.i += 1;
    }

    fn eof(&self) -> bool {
        self.i >= self.input.len()
    }
}

/// `std.fmt.parseInt(u21, s, 16)` for plain hex digit strings.
fn parse_u21_hex(s: &[u8]) -> Option<u32> {
    let s = std::str::from_utf8(s).ok()?;
    let v = u32::from_str_radix(s, 16).ok()?;
    (v <= 0x1f_ffff).then_some(v)
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RepeatableCodepointMap {
    pub map: CodepointMap,
}

impl ConfigField for RepeatableCodepointMap {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        let eql_idx = input.find('=').ok_or(ArgError::InvalidValue)?;
        let key = trim_ws(&input[..eql_idx]);
        let value = trim_ws(&input[eql_idx + 1..]);

        let mut p = UnicodeRangeParser::new(key);
        while let Some(range) = p.next_range()? {
            self.map.add(CodepointMapEntry {
                range,
                descriptor: Descriptor {
                    family: Some(value.to_string()),
                    monospace: false, // we allow any font
                    ..Descriptor::default()
                },
            });
        }
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        if self.map.list.is_empty() {
            return write_void(f);
        }

        for e in &self.map.list {
            let family = e.descriptor.family.as_deref().unwrap_or("");
            if e.range[0] == e.range[1] {
                write_str(f, &format!("U+{:04X}={}", e.range[0], family))?;
            } else {
                write_str(
                    f,
                    &format!("U+{:04X}-U+{:04X}={}", e.range[0], e.range[1], family),
                )?;
            }
        }
        Ok(())
    }
}

/// See "clipboard-codepoint-map" for documentation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RepeatableClipboardCodepointMap {
    pub map: ClipboardCodepointMap,
}

impl ConfigField for RepeatableClipboardCodepointMap {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        let eql_idx = input.find('=').ok_or(ArgError::InvalidValue)?;
        let key = trim_ws(&input[..eql_idx]);
        let value = trim_ws(&input[eql_idx + 1..]);

        // Parse the replacement value - either a codepoint or string
        let replacement = if let Some(cp_str) = value.strip_prefix("U+") {
            // Parse as codepoint
            let cp = parse_u21_hex(cp_str.as_bytes()).ok_or(ArgError::InvalidValue)?;
            Replacement::Codepoint(cp)
        } else {
            // `value` is a &str so it is already valid UTF-8.
            Replacement::String(value.to_string())
        };

        let mut p = UnicodeRangeParser::new(key);
        while let Some(range) = p.next_range()? {
            self.map.add(ClipboardEntry {
                range,
                replacement: replacement.clone(),
            });
        }
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        if self.map.list.is_empty() {
            return write_void(f);
        }

        for e in &self.map.list {
            let value_str = match &e.replacement {
                Replacement::Codepoint(cp) => format!("U+{cp:04X}"),
                Replacement::String(s) => s.clone(),
            };

            if e.range[0] == e.range[1] {
                write_str(f, &format!("U+{:04X}={}", e.range[0], value_str))?;
            } else {
                write_str(
                    f,
                    &format!("U+{:04X}-U+{:04X}={}", e.range[0], e.range[1], value_str),
                )?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Font style

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FontStyle {
    /// Use the default font style that font discovery finds.
    #[default]
    Default,

    /// Disable this font style completely. This will fall back to using
    /// the regular font when this style is encountered.
    False,

    /// A specific named font style to use for this style.
    Name(String),
}

impl FontStyle {
    /// Returns the string name value that can be used with a font
    /// descriptor.
    #[must_use]
    pub fn name_value(&self) -> Option<&str> {
        match self {
            FontStyle::Default | FontStyle::False => None,
            FontStyle::Name(n) => Some(n),
        }
    }
}

impl ConfigField for FontStyle {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let value = input.ok_or(ArgError::ValueRequired)?;
        *self = match value {
            "default" => FontStyle::Default,
            "false" => FontStyle::False,
            _ => FontStyle::Name(value.to_string()),
        };
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        match self {
            FontStyle::Default => write_str(f, "default"),
            FontStyle::False => write_str(f, "false"),
            FontStyle::Name(n) => write_str(f, n),
        }
    }
}

config_packed! {
    /// See `font-synthetic-style` for documentation.
    pub struct FontSyntheticStyle {
        "bold" => bold: true,
        "italic" => italic: true,
        "bold-italic" => bold_italic: true,
    }
}

config_packed! {
    /// See "font-shaping-break" for documentation
    pub struct FontShapingBreak {
        "cursor" => cursor: true,
    }
}

// ---------------------------------------------------------------------
// RepeatableLink

/// See "link" for documentation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RepeatableLink {
    pub links: Vec<ghostty_input::Link>,
}

impl ConfigField for RepeatableLink {
    fn parse_cli(&mut self, _input: Option<&str>) -> Result<(), ArgError> {
        Err(ArgError::other("NotImplemented"))
    }

    fn format_entry(&self, _f: &mut EntryFormatter<'_>) -> fmt::Result {
        // This currently can't be set so we don't format anything.
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::cli_args::ConfigField;

    fn fe<T: ConfigField>(v: &T) -> String {
        let mut s = String::new();
        let mut f = EntryFormatter::new("a", &mut s);
        v.format_entry(&mut f).unwrap();
        s
    }

    fn parse<T: ConfigField>(v: &mut T, s: &str) -> Result<(), ArgError> {
        v.parse_cli(Some(s))
    }

    mod working_directory {
        use super::*;

        #[test]
        fn working_directory_parse_cli() {
            let mut wd = WorkingDirectory::Inherit;
            parse(&mut wd, "inherit").unwrap();
            assert_eq!(wd, WorkingDirectory::Inherit);
            parse(&mut wd, "home").unwrap();
            assert_eq!(wd, WorkingDirectory::Home);
            parse(&mut wd, "~/projects/ghostty").unwrap();
            assert_eq!(wd, WorkingDirectory::Path("~/projects/ghostty".into()));
            parse(&mut wd, "\"/tmp path\"").unwrap();
            assert_eq!(wd, WorkingDirectory::Path("/tmp path".into()));
        }

        #[test]
        fn working_directory_finalize() {
            let mut wd = WorkingDirectory::Path("~/projects/ghostty".into());
            wd.finalize();
            let expected = ghostty_os::expand_home("~/projects/ghostty")
                .unwrap_or_else(|_| "~/projects/ghostty".to_string());
            assert_eq!(wd.value().unwrap(), expected);
        }
    }

    mod color {
        use super::*;

        #[test]
        fn from_hex() {
            let c = |r, g, b| Color { r, g, b };
            assert_eq!(Color::from_hex("#000000").unwrap(), c(0, 0, 0));
            assert_eq!(Color::from_hex("#0A0B0C").unwrap(), c(10, 11, 12));
            assert_eq!(Color::from_hex("0A0B0C").unwrap(), c(10, 11, 12));
            assert_eq!(Color::from_hex("FFFFFF").unwrap(), c(255, 255, 255));
            assert_eq!(Color::from_hex("FFF").unwrap(), c(255, 255, 255));
            assert_eq!(Color::from_hex("#345").unwrap(), c(51, 68, 85));
        }

        #[test]
        fn parse_cli_from_name() {
            assert_eq!(
                Color::from_cli(Some("black")).unwrap(),
                Color { r: 0, g: 0, b: 0 }
            );
        }

        #[test]
        fn format_config() {
            let color = Color {
                r: 10,
                g: 11,
                b: 12,
            };
            assert_eq!(fe(&color), "a = #0a0b0c\n");
        }

        #[test]
        fn parse_cli_with_whitespace() {
            assert_eq!(
                Color::from_cli(Some(" #AABBCC   ")).unwrap(),
                Color {
                    r: 0xAA,
                    g: 0xBB,
                    b: 0xCC
                }
            );
            assert_eq!(
                Color::from_cli(Some("  black ")).unwrap(),
                Color { r: 0, g: 0, b: 0 }
            );
        }
    }

    mod terminal_color {
        use super::*;

        #[test]
        fn parse_cli() {
            assert_eq!(
                TerminalColor::from_cli(Some("#4e2a84")).unwrap(),
                TerminalColor::Color(Color {
                    r: 78,
                    g: 42,
                    b: 132
                })
            );
            assert_eq!(
                TerminalColor::from_cli(Some("black")).unwrap(),
                TerminalColor::Color(Color { r: 0, g: 0, b: 0 })
            );
            assert_eq!(
                TerminalColor::from_cli(Some("cell-foreground")).unwrap(),
                TerminalColor::CellForeground
            );
            assert_eq!(
                TerminalColor::from_cli(Some("cell-background")).unwrap(),
                TerminalColor::CellBackground
            );
            assert_eq!(
                TerminalColor::from_cli(Some("a")),
                Err(ArgError::InvalidValue)
            );
        }

        #[test]
        fn format_config() {
            assert_eq!(fe(&TerminalColor::CellForeground), "a = cell-foreground\n");
        }
    }

    mod bold_color {
        use super::*;

        #[test]
        fn parse_cli() {
            assert_eq!(
                BoldColor::from_cli(Some("#4e2a84")).unwrap(),
                BoldColor::Color(Color {
                    r: 78,
                    g: 42,
                    b: 132
                })
            );
            assert_eq!(
                BoldColor::from_cli(Some("black")).unwrap(),
                BoldColor::Color(Color { r: 0, g: 0, b: 0 })
            );
            assert_eq!(
                BoldColor::from_cli(Some("bright")).unwrap(),
                BoldColor::Bright
            );
            assert_eq!(BoldColor::from_cli(Some("a")), Err(ArgError::InvalidValue));
        }

        #[test]
        fn format_config() {
            assert_eq!(fe(&BoldColor::Bright), "a = bright\n");
        }
    }

    mod color_list {
        use super::*;

        #[test]
        fn parse_cli() {
            let mut p = ColorList::default();
            parse(&mut p, "black,white").unwrap();
            assert_eq!(p.colors.len(), 2);

            // Test whitespace handling
            parse(&mut p, "black, white").unwrap();
            assert_eq!(p.colors.len(), 2);
            parse(&mut p, "black , white").unwrap();
            assert_eq!(p.colors.len(), 2);
            parse(&mut p, " black , white ").unwrap();
            assert_eq!(p.colors.len(), 2);

            // Error cases
            assert_eq!(p.parse_cli(None), Err(ArgError::ValueRequired));
            assert_eq!(parse(&mut p, " "), Err(ArgError::InvalidValue));
        }

        #[test]
        fn format() {
            let mut p = ColorList::default();
            parse(&mut p, "black,white").unwrap();
            assert_eq!(fe(&p), "a = #000000,#ffffff\n");
        }

        #[test]
        fn clone() {
            let mut source = ColorList::default();
            parse(&mut source, "#ff0000,#00ff00,#0000ff").unwrap();
            let cloned = source.clone();
            assert_eq!(source, cloned);
        }
    }

    mod palette {
        use super::*;

        #[test]
        fn parse_cli() {
            let mut p = Palette::default();
            parse(&mut p, "0=#AABBCC").unwrap();
            assert_eq!(p.value[0].r, 0xAA);
            assert_eq!(p.value[0].g, 0xBB);
            assert_eq!(p.value[0].b, 0xCC);
            assert!(p.mask.is_set(0));
            assert!(!p.mask.is_set(1));
        }

        #[test]
        fn parse_cli_base() {
            let mut p = Palette::default();
            parse(&mut p, "0b1=#014589").unwrap();
            parse(&mut p, "0o7=#234567").unwrap();
            parse(&mut p, "0xF=#ABCDEF").unwrap();

            assert_eq!(
                (p.value[1].r, p.value[1].g, p.value[1].b),
                (0x01, 0x45, 0x89)
            );
            assert_eq!(
                (p.value[7].r, p.value[7].g, p.value[7].b),
                (0x23, 0x45, 0x67)
            );
            assert_eq!(
                (p.value[15].r, p.value[15].g, p.value[15].b),
                (0xAB, 0xCD, 0xEF)
            );

            assert!(p.mask.is_set(1));
            assert!(p.mask.is_set(7));
            assert!(p.mask.is_set(15));
            assert!(!p.mask.is_set(0));
            assert!(!p.mask.is_set(2));
        }

        #[test]
        fn parse_cli_overflow() {
            let mut p = Palette::default();
            assert_eq!(
                parse(&mut p, "256=#AABBCC"),
                Err(ArgError::other("Overflow"))
            );
            // Mask should remain empty since parsing failed.
            assert_eq!(p.mask.count(), 0);
        }

        #[test]
        fn format_config() {
            let list = Palette::default();
            assert_eq!(&fe(&list)[0..14], "a = 0=#1d1f21\n");
        }

        #[test]
        fn parse_cli_with_whitespace() {
            let mut p = Palette::default();
            parse(&mut p, "0 =  #AABBCC").unwrap();
            parse(&mut p, " 1= #DDEEFF    ").unwrap();
            parse(&mut p, "  2  =  #123456 ").unwrap();

            assert_eq!(
                (p.value[0].r, p.value[0].g, p.value[0].b),
                (0xAA, 0xBB, 0xCC)
            );
            assert_eq!(
                (p.value[1].r, p.value[1].g, p.value[1].b),
                (0xDD, 0xEE, 0xFF)
            );
            assert_eq!(
                (p.value[2].r, p.value[2].g, p.value[2].b),
                (0x12, 0x34, 0x56)
            );

            assert!(p.mask.is_set(0));
            assert!(p.mask.is_set(1));
            assert!(p.mask.is_set(2));
            assert!(!p.mask.is_set(3));
        }
    }

    mod repeatable_string {
        use super::*;

        #[test]
        fn parse_cli() {
            let mut list = RepeatableString::default();
            parse(&mut list, "A").unwrap();
            parse(&mut list, "B").unwrap();
            assert_eq!(list.list.len(), 2);

            parse(&mut list, "").unwrap();
            assert_eq!(list.list.len(), 0);
        }

        #[test]
        fn parse_cli_overwrite() {
            let mut list = RepeatableString::default();
            parse(&mut list, "A").unwrap();

            // Set our overwrite flag
            list.overwrite_next = true;

            parse(&mut list, "B").unwrap();
            assert_eq!(list.list.len(), 1);
            parse(&mut list, "C").unwrap();
            assert_eq!(list.list.len(), 2);
        }

        #[test]
        fn format_config_empty() {
            assert_eq!(fe(&RepeatableString::default()), "a = \n");
        }

        #[test]
        fn format_config_single_item() {
            let mut list = RepeatableString::default();
            parse(&mut list, "A").unwrap();
            assert_eq!(fe(&list), "a = A\n");
        }

        #[test]
        fn format_config_multiple_items() {
            let mut list = RepeatableString::default();
            parse(&mut list, "A").unwrap();
            parse(&mut list, "B").unwrap();
            assert_eq!(fe(&list), "a = A\na = B\n");
        }

        #[test]
        fn equality_ignores_overwrite_next() {
            let mut a = RepeatableString::default();
            let b = RepeatableString::default();
            a.overwrite_next = true;
            assert_eq!(a, b);
        }
    }

    mod selection_word_chars {
        use super::*;

        #[test]
        fn parse_cli() {
            let mut chars = SelectionWordChars::default();
            parse(&mut chars, " \t;,").unwrap();
            assert_eq!(
                chars.codepoints,
                vec![0, ' ' as u32, '\t' as u32, ';' as u32, ',' as u32]
            );
        }

        #[test]
        fn parse_cli_escape_sequences() {
            let mut chars = SelectionWordChars::default();
            parse(&mut chars, " \\t;,").unwrap();
            assert_eq!(
                chars.codepoints,
                vec![0, ' ' as u32, '\t' as u32, ';' as u32, ',' as u32]
            );
        }

        #[test]
        fn parse_cli_backslash_escape() {
            let mut chars = SelectionWordChars::default();
            parse(&mut chars, "\\\\;").unwrap();
            assert_eq!(chars.codepoints, vec![0, '\\' as u32, ';' as u32]);
        }

        #[test]
        fn parse_cli_unicode_escape() {
            let mut chars = SelectionWordChars::default();
            parse(&mut chars, "\\u{2502};").unwrap();
            assert_eq!(chars.codepoints, vec![0, '│' as u32, ';' as u32]);
        }

        #[test]
        fn format_roundtrip() {
            let chars = SelectionWordChars::default();
            assert_eq!(fe(&chars), "a =  \t'\"│`|:;,()[]{}<>$\n");
        }
    }

    mod font_variation {
        use super::*;

        #[allow(clippy::trivially_copy_pass_by_ref)]
        fn v(id: &[u8; 4], value: f64) -> Variation {
            Variation {
                id: VariationId::init(id),
                value,
            }
        }

        #[test]
        fn parse_cli() {
            let mut list = RepeatableFontVariation::default();
            parse(&mut list, "wght=200").unwrap();
            parse(&mut list, "slnt=-15").unwrap();
            assert_eq!(list.list, vec![v(b"wght", 200.0), v(b"slnt", -15.0)]);
        }

        #[test]
        fn parse_cli_with_whitespace() {
            let mut list = RepeatableFontVariation::default();
            parse(&mut list, "wght =200").unwrap();
            parse(&mut list, "slnt= -15").unwrap();
            assert_eq!(list.list, vec![v(b"wght", 200.0), v(b"slnt", -15.0)]);
        }

        #[test]
        fn format_config_single() {
            let mut list = RepeatableFontVariation::default();
            parse(&mut list, "wght = 200").unwrap();
            assert_eq!(fe(&list), "a = wght=200\n");
        }
    }

    mod codepoint_map {
        use super::*;

        fn fam(map: &RepeatableCodepointMap, i: usize) -> (&[u32; 2], &str) {
            let e = &map.map.list[i];
            (&e.range, e.descriptor.family.as_deref().unwrap())
        }

        #[test]
        fn parse_cli() {
            let mut list = RepeatableCodepointMap::default();
            parse(&mut list, "U+ABCD=Comic Sans").unwrap();
            parse(&mut list, "U+0001 - U+0005=Verdana").unwrap();
            parse(&mut list, "U+0006-U+0009, U+ABCD=Courier").unwrap();

            assert_eq!(list.map.list.len(), 4);
            assert_eq!(fam(&list, 0), (&[0xABCD, 0xABCD], "Comic Sans"));
            assert_eq!(fam(&list, 1), (&[1, 5], "Verdana"));
            assert_eq!(fam(&list, 2), (&[6, 9], "Courier"));
            assert_eq!(fam(&list, 3), (&[0xABCD, 0xABCD], "Courier"));
        }

        #[test]
        fn format_config_single() {
            let mut list = RepeatableCodepointMap::default();
            parse(&mut list, "U+ABCD=Comic Sans").unwrap();
            assert_eq!(fe(&list), "a = U+ABCD=Comic Sans\n");
        }

        #[test]
        fn format_config_range() {
            let mut list = RepeatableCodepointMap::default();
            parse(&mut list, "U+0001 - U+0005=Verdana").unwrap();
            assert_eq!(fe(&list), "a = U+0001-U+0005=Verdana\n");
        }

        #[test]
        fn format_config_multiple() {
            let mut list = RepeatableCodepointMap::default();
            parse(&mut list, "U+0006-U+0009, U+ABCD=Courier").unwrap();
            assert_eq!(fe(&list), "a = U+0006-U+0009=Courier\na = U+ABCD=Courier\n");
        }

        #[test]
        fn invalid_ranges() {
            let mut list = RepeatableCodepointMap::default();
            assert_eq!(parse(&mut list, "U+5-U+1=X"), Err(ArgError::InvalidValue));
            assert_eq!(parse(&mut list, "U+1,=X"), Err(ArgError::InvalidValue));
            assert_eq!(parse(&mut list, "U+=X"), Err(ArgError::InvalidValue));
            assert_eq!(parse(&mut list, "nope"), Err(ArgError::InvalidValue));
        }
    }

    mod clipboard_codepoint_map {
        use super::*;

        #[test]
        fn parse_cli_codepoint_replacement() {
            let mut list = RepeatableClipboardCodepointMap::default();
            parse(&mut list, "U+2500=U+002D").unwrap();
            assert_eq!(list.map.list.len(), 1);
            let e = &list.map.list[0];
            assert_eq!(e.range, [0x2500, 0x2500]);
            assert_eq!(e.replacement, Replacement::Codepoint(0x002D));
        }

        #[test]
        fn parse_cli_string_replacement() {
            let mut list = RepeatableClipboardCodepointMap::default();
            parse(&mut list, "U+03A3=SUM").unwrap();
            assert_eq!(list.map.list.len(), 1);
            let e = &list.map.list[0];
            assert_eq!(e.range, [0x03A3, 0x03A3]);
            assert_eq!(e.replacement, Replacement::String("SUM".into()));
        }

        #[test]
        fn parse_cli_range_replacement() {
            let mut list = RepeatableClipboardCodepointMap::default();
            parse(&mut list, "U+2500-U+2503=|").unwrap();
            assert_eq!(list.map.list.len(), 1);
            let e = &list.map.list[0];
            assert_eq!(e.range, [0x2500, 0x2503]);
            assert_eq!(e.replacement, Replacement::String("|".into()));
        }

        #[test]
        fn format_config_codepoint() {
            let mut list = RepeatableClipboardCodepointMap::default();
            parse(&mut list, "U+2500=U+002D").unwrap();
            assert_eq!(fe(&list), "a = U+2500=U+002D\n");
        }

        #[test]
        fn format_config_string() {
            let mut list = RepeatableClipboardCodepointMap::default();
            parse(&mut list, "U+03A3=SUM").unwrap();
            assert_eq!(fe(&list), "a = U+03A3=SUM\n");
        }
    }

    mod font_style {
        use super::*;

        #[test]
        fn parse_cli() {
            let mut p = FontStyle::Default;
            parse(&mut p, "default").unwrap();
            assert_eq!(p, FontStyle::Default);
            parse(&mut p, "false").unwrap();
            assert_eq!(p, FontStyle::False);
            parse(&mut p, "bold").unwrap();
            assert_eq!(p, FontStyle::Name("bold".into()));
        }

        #[test]
        fn format_config_default() {
            let mut p = FontStyle::Default;
            parse(&mut p, "default").unwrap();
            assert_eq!(fe(&p), "a = default\n");
        }

        #[test]
        fn format_config_false() {
            let mut p = FontStyle::Default;
            parse(&mut p, "false").unwrap();
            assert_eq!(fe(&p), "a = false\n");
        }

        #[test]
        fn format_config_named() {
            let mut p = FontStyle::Default;
            parse(&mut p, "bold").unwrap();
            assert_eq!(fe(&p), "a = bold\n");
        }
    }

    #[test]
    fn packed_and_enum_formats() {
        assert_eq!(
            fe(&FontSyntheticStyle::default()),
            "a = bold,italic,bold-italic\n"
        );
        let mut s = FontSyntheticStyle::default();
        parse(&mut s, "no-bold-italic").unwrap();
        assert!(!s.bold_italic && s.bold);
        assert_eq!(fe(&FontShapingBreak::default()), "a = cursor\n");
        assert_eq!(
            fe(&Fullscreen::NonNativePaddedNotch),
            "a = non-native-padded-notch\n"
        );
        let mut c = CursorStyle::default();
        parse(&mut c, "block_hollow").unwrap();
        assert_eq!(c, CursorStyle::BlockHollow);
    }

    #[test]
    fn repeatable_link_not_implemented() {
        let mut l = RepeatableLink::default();
        assert_eq!(parse(&mut l, "x"), Err(ArgError::other("NotImplemented")));
        assert_eq!(fe(&l), "");
    }
}
