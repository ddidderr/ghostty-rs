//! Types nested in `Config.zig` (lines ~8566-10146): enums, packed structs
//! and small structs used as config field values.

#![allow(clippy::format_push_string, clippy::single_match_else)]

use std::fmt;

use ghostty_input::binding::Action;
use ghostty_os::osbytes::EncodedOs;

use crate::{
    c_value::{CValue, QuickTerminalSizeC},
    cli_args::{ArgError, ConfigField, parse_auto_struct_with, parse_bool, trim_ws},
    config_enum,
    config_packed,
    formatter::EntryFormatter,
};

/// Write `name = <s>\n` (Zig: `formatEntry([]const u8, s)`).
fn fmt_str(f: &mut EntryFormatter<'_>, s: &str) -> fmt::Result {
    writeln!(f.writer, "{} = {}", f.name, s)
}

config_enum! {
    /// Options for copy on select behavior.
    pub enum CopyOnSelect {
        /// Disables copy on select entirely.
        False => "false",

        /// Copy on select is enabled, but goes to the selection clipboard.
        /// This is not supported on platforms such as macOS. This is the default.
        True => "true",

        /// Copy on select is enabled and goes to both the system clipboard
        /// and the selection clipboard (for Linux).
        Clipboard => "clipboard",
    }
}

config_enum! {
    /// Options for right-click actions.
    pub enum RightClickAction {
        /// No action is taken on right-click.
        Ignore => "ignore",

        /// Pastes from the system clipboard.
        Paste => "paste",

        /// Copies the selected text to the system clipboard.
        Copy => "copy",

        /// Copies the selected text to the system clipboard and
        /// pastes the clipboard if no text is selected.
        CopyOrPaste => "copy-or-paste",

        /// Shows a context menu with options.
        ContextMenu => "context-menu",
    }
}

config_enum! {
    /// Shell integration values
    pub enum ShellIntegration {
        None => "none",
        Detect => "detect",
        Bash => "bash",
        Elvish => "elvish",
        Fish => "fish",
        Nushell => "nushell",
        Zsh => "zsh",
    }
}

config_packed! {
    /// Shell integration features
    pub struct ShellIntegrationFeatures {
        "cursor" => cursor: true,
        "sudo" => sudo: false,
        "title" => title: true,
        "ssh-env" => ssh_env: false,
        "ssh-terminfo" => ssh_terminfo: false,
        "path" => path: true,
    }
}

config_packed! {
    pub struct SplitPreserveZoom {
        "navigation" => navigation: false,
    }
}

use ghostty_input::command::Command;

/// Parse a command from `title:...,description:...,action:...`
/// (Zig: `parseAutoStruct(inputpkg.Command, alloc, input, null)`).
pub fn parse_command_auto(input: &[u8]) -> Result<Command, ArgError> {
    let mut action: Option<Action> = None;
    let mut title: Option<EncodedOs> = None;
    let mut description = EncodedOs::new();
    crate::cli_args::parse_auto_struct_bytes_with(input, |k, v| {
        match k {
            "action" => {
                action =
                    Some(Action::parse_bytes(v).map_err(|e| ArgError::Other(format!("{e:?}")))?);
            }
            "title" => title = Some(EncodedOs::from_bytes(v)),
            "description" => description = EncodedOs::from_bytes(v),
            _ => return Ok(false),
        }
        Ok(true)
    })?;
    Ok(Command {
        action: action.ok_or(ArgError::InvalidValue)?,
        title: title.ok_or(ArgError::InvalidValue)?,
        description,
    })
}

fn default_commands() -> Vec<Command> {
    ghostty_input::command::defaults().to_vec()
}

/// Escape a string like Zig's `std.zig.fmtString`: `\n \r \t \\ \"` are
/// escaped by name and every other byte outside printable ASCII is written
/// as `\xNN` (so multi-byte UTF-8 is escaped per byte).
fn zig_fmt_string(s: &EncodedOs) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for b in s.to_bytes() {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b' '..=b'~' => out.push(b as char),
            _ => {
                let _ = write!(out, "\\x{b:02x}");
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RepeatableCommand {
    pub value: Vec<Command>,
}

impl RepeatableCommand {
    /// Reset to the default command list (Zig: `init`).
    pub fn init(&mut self) {
        self.value = default_commands();
    }
}

impl ConfigField for RepeatableCommand {
    fn c_value(&self) -> CValue {
        CValue::Commands(
            self.value
                .iter()
                .map(ghostty_input::command::Command::cval)
                .collect(),
        )
    }

    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        // Unset or empty input clears the list
        let input = input.unwrap_or(b"");
        if input.is_empty() {
            self.value.clear();
            return Ok(());
        }

        let cmd = parse_command_auto(input)?;
        self.value.push(cmd);
        Ok(())
    }

    /// Zig: `init` -- resets to the default command list.
    fn reset(&mut self, _default: Self) {
        self.value = default_commands();
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        if self.value.is_empty() {
            return writeln!(f.writer, "{} = ", f.name);
        }

        for item in &self.value {
            let mut s = format!("title:\"{}\"", zig_fmt_string(&item.title));
            if !item.description.is_empty() {
                s.push_str(&format!(
                    ",description:\"{}\"",
                    zig_fmt_string(&item.description)
                ));
            }
            s.push_str(&format!(",action:\"{}\"", item.action));
            fmt_str(f, &s)?;
        }
        Ok(())
    }
}

config_enum! {
    /// OSC 4, 10, 11, and 12 default color reporting format.
    pub enum OSCColorReportFormat {
        None => "none",
        Bit8 => "8-bit",
        Bit16 => "16-bit",
    }
}

config_enum! {
    /// The default window theme.
    pub enum WindowTheme {
        Auto => "auto",
        System => "system",
        Light => "light",
        Dark => "dark",
        Ghostty => "ghostty",
    }
}

config_enum! {
    /// See window-colorspace
    pub enum WindowColorspace {
        Srgb => "srgb",
        DisplayP3 => "display-p3",
    }
}

config_enum! {
    /// See macos-window-buttons
    pub enum MacWindowButtons {
        Visible => "visible",
        Hidden => "hidden",
    }
}

config_enum! {
    /// See macos-titlebar-style
    pub enum MacTitlebarStyle {
        Native => "native",
        Transparent => "transparent",
        Tabs => "tabs",
        Hidden => "hidden",
    }
}

config_enum! {
    /// See macos-titlebar-proxy-icon
    pub enum MacTitlebarProxyIcon {
        Visible => "visible",
        Hidden => "hidden",
    }
}

config_enum! {
    /// See macos-hidden
    pub enum MacHidden {
        Never => "never",
        Always => "always",
    }
}

config_enum! {
    /// See macos-icon
    ///
    /// Note: future versions of Ghostty can support a custom icon with
    /// path by changing this to a tagged union, which doesn't change our
    /// format at all.
    pub enum MacAppIcon {
        Official => "official",
        Blueprint => "blueprint",
        Chalkboard => "chalkboard",
        Microchip => "microchip",
        Glass => "glass",
        Holographic => "holographic",
        Paper => "paper",
        Retro => "retro",
        Xray => "xray",
        Custom => "custom",
        CustomStyle => "custom-style",
    }
}

config_enum! {
    /// See macos-icon-frame
    pub enum MacAppIconFrame {
        Aluminum => "aluminum",
        Beige => "beige",
        Plastic => "plastic",
        Chrome => "chrome",
    }
}

config_enum! {
    /// See macos-shortcuts
    pub enum MacShortcuts {
        Allow => "allow",
        Deny => "deny",
        Ask => "ask",
    }
}

config_enum! {
    /// See gtk-single-instance
    pub enum GtkSingleInstance {
        False => "false",
        True => "true",
        Detect => "detect",
    }
}

impl GtkSingleInstance {
    /// Zig: `pub const default: GtkSingleInstance = .detect;`
    pub const DEFAULT: GtkSingleInstance = GtkSingleInstance::Detect;
}

config_enum! {
    /// See gtk-tabs-location
    pub enum GtkTabsLocation {
        Top => "top",
        Bottom => "bottom",
    }
}

config_enum! {
    /// See gtk-toolbar-style
    pub enum GtkToolbarStyle {
        Flat => "flat",
        Raised => "raised",
        RaisedBorder => "raised-border",
    }
}

config_enum! {
    /// See gtk-titlebar-style
    pub enum GtkTitlebarStyle {
        Native => "native",
        Tabs => "tabs",
    }
}

config_packed! {
    /// See app-notifications
    pub struct AppNotifications {
        "clipboard-copy" => clipboard_copy: true,
        "config-reload" => config_reload: true,
    }
}

config_packed! {
    /// See bell-features
    pub struct BellFeatures {
        "system" => system: false,
        "audio" => audio: false,
        "attention" => attention: true,
        "title" => title: true,
        "border" => border: false,
    }
}

config_enum! {
    /// See mouse-shift-capture
    pub enum MouseShiftCapture {
        False => "false",
        True => "true",
        Always => "always",
        Never => "never",
    }
}

/// See mouse-scroll-multiplier
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseScrollMultiplier {
    pub precision: f64,
    pub discrete: f64,
}

impl Default for MouseScrollMultiplier {
    fn default() -> Self {
        Self {
            precision: 1.0,
            discrete: 3.0,
        }
    }
}

impl MouseScrollMultiplier {
    /// Zig: `pub const default`.
    pub const DEFAULT: MouseScrollMultiplier = MouseScrollMultiplier {
        precision: 1.0,
        discrete: 3.0,
    };
}

impl ConfigField for MouseScrollMultiplier {
    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let input = crate::cli_args::text_value(input)?;
        let input = input.ok_or(ArgError::ValueRequired)?;
        // Unset fields inherit from the current value.
        let mut result = *self;
        let r = parse_auto_struct_with(input, |k, v| {
            match k {
                "precision" => result.precision.parse_cli(Some(v))?,
                "discrete" => result.discrete.parse_cli(Some(v))?,
                _ => return Ok(false),
            }
            Ok(true)
        });
        match r {
            Ok(()) => *self = result,
            Err(ArgError::InvalidValue) => {
                let v: f64 = input.parse().map_err(|_| ArgError::InvalidValue)?;
                *self = MouseScrollMultiplier {
                    precision: v,
                    discrete: v,
                };
            }
            Err(e) => return Err(e),
        }
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        fmt_str(
            f,
            &format!("precision:{},discrete:{}", self.precision, self.discrete),
        )
    }
}

config_enum! {
    /// How to treat requests to write to or read from the clipboard
    pub enum ClipboardAccess {
        Allow => "allow",
        Deny => "deny",
        Ask => "ask",
    }
}

config_enum! {
    /// See window-save-state
    pub enum WindowSaveState {
        Default => "default",
        Never => "never",
        Always => "always",
    }
}

config_enum! {
    /// See window-new-tab-position
    pub enum WindowNewTabPosition {
        Current => "current",
        End => "end",
    }
}

config_enum! {
    /// See macos-dock-drop-behavior
    pub enum MacOSDockDropBehavior {
        NewTab => "new-tab",
        NewWindow => "new-window",
    }
}

config_enum! {
    /// See window-show-tab-bar
    pub enum WindowShowTabBar {
        Always => "always",
        Auto => "auto",
        Never => "never",
    }
}

config_enum! {
    /// See resize-overlay
    pub enum ResizeOverlay {
        Always => "always",
        Never => "never",
        AfterFirst => "after-first",
    }
}

config_enum! {
    /// See resize-overlay-position
    pub enum ResizeOverlayPosition {
        Center => "center",
        TopLeft => "top-left",
        TopCenter => "top-center",
        TopRight => "top-right",
        BottomLeft => "bottom-left",
        BottomCenter => "bottom-center",
        BottomRight => "bottom-right",
    }
}

config_enum! {
    /// See quick-terminal-position
    pub enum QuickTerminalPosition {
        Top => "top",
        Bottom => "bottom",
        Left => "left",
        Right => "right",
        Center => "center",
    }
}

config_enum! {
    /// See quick-terminal-layer
    pub enum QuickTerminalLayer {
        Overlay => "overlay",
        Top => "top",
        Bottom => "bottom",
        Background => "background",
    }
}

/// A size of the quick terminal along one axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QuickTerminalSizeValue {
    Percentage(f32),
    Pixels(u32),
}

impl QuickTerminalSizeValue {
    #[must_use]
    pub fn to_pixels(self, parent_dimensions: u32) -> u32 {
        match self {
            Self::Percentage(v) => {
                let dim = parent_dimensions as f32;
                (v / 100.0 * dim) as u32
            }
            Self::Pixels(v) => v,
        }
    }

    pub fn parse(input: &str) -> Result<Self, ArgError> {
        if input.is_empty() {
            return Err(ArgError::ValueRequired);
        }

        if let Some(num) = input.strip_suffix("px") {
            return Ok(Self::Pixels(
                num.parse::<u32>().map_err(|_| ArgError::InvalidValue)?,
            ));
        }

        if let Some(num) = input.strip_suffix('%') {
            let percentage = num.parse::<f32>().map_err(|_| ArgError::InvalidValue)?;
            if percentage < 0.0 {
                return Err(ArgError::InvalidValue);
            }
            return Ok(Self::Percentage(percentage));
        }

        Err(ArgError::other("MissingUnit"))
    }

    fn format(self, out: &mut String) {
        match self {
            Self::Percentage(v) => out.push_str(&format!("{v}%")),
            Self::Pixels(v) => out.push_str(&format!("{v}px")),
        }
    }
}

/// See quick-terminal-size
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct QuickTerminalSize {
    pub primary: Option<QuickTerminalSizeValue>,
    pub secondary: Option<QuickTerminalSizeValue>,
}

/// Dimensions in pixels (Zig: `QuickTerminalSize.Dimensions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickTerminalDimensions {
    pub width: u32,
    pub height: u32,
}

impl QuickTerminalSize {
    #[must_use]
    pub fn calculate(
        &self,
        position: QuickTerminalPosition,
        dims: QuickTerminalDimensions,
    ) -> QuickTerminalDimensions {
        let p = self.primary;
        let s = self.secondary;
        match position {
            QuickTerminalPosition::Left | QuickTerminalPosition::Right => QuickTerminalDimensions {
                width: p.map_or(400, |v| v.to_pixels(dims.width)),
                height: s.map_or(dims.height, |v| v.to_pixels(dims.height)),
            },
            QuickTerminalPosition::Top | QuickTerminalPosition::Bottom => QuickTerminalDimensions {
                width: s.map_or(dims.width, |v| v.to_pixels(dims.width)),
                height: p.map_or(400, |v| v.to_pixels(dims.height)),
            },
            QuickTerminalPosition::Center => {
                if dims.width >= dims.height {
                    QuickTerminalDimensions {
                        width: p.map_or(800, |v| v.to_pixels(dims.width)),
                        height: s.map_or(400, |v| v.to_pixels(dims.height)),
                    }
                } else {
                    QuickTerminalDimensions {
                        width: s.map_or(400, |v| v.to_pixels(dims.width)),
                        height: p.map_or(800, |v| v.to_pixels(dims.height)),
                    }
                }
            }
        }
    }
}

impl ConfigField for QuickTerminalSize {
    fn c_value(&self) -> CValue {
        let conv = |v: Option<QuickTerminalSizeValue>| match v {
            None => QuickTerminalSizeC::None,
            Some(QuickTerminalSizeValue::Percentage(p)) => QuickTerminalSizeC::Percentage(p),
            Some(QuickTerminalSizeValue::Pixels(p)) => QuickTerminalSizeC::Pixels(p),
        };
        CValue::QuickTerminalSize {
            primary: conv(self.primary),
            secondary: conv(self.secondary),
        }
    }

    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let input = crate::cli_args::text_value(input)?;
        let input = input.ok_or(ArgError::ValueRequired)?;
        let mut it = input.split(',');

        let primary = trim_ws(it.next().ok_or(ArgError::ValueRequired)?);
        self.primary = Some(QuickTerminalSizeValue::parse(primary)?);

        self.secondary = match it.next() {
            Some(s) => Some(QuickTerminalSizeValue::parse(trim_ws(s))?),
            None => None,
        };

        if it.next().is_some() {
            return Err(ArgError::other("TooManyArguments"));
        }
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        let Some(primary) = self.primary else {
            return Ok(());
        };
        let mut s = String::new();
        primary.format(&mut s);
        if let Some(secondary) = self.secondary {
            s.push(',');
            secondary.format(&mut s);
        }
        fmt_str(f, &s)
    }
}

config_enum! {
    /// See quick-terminal-screen
    pub enum QuickTerminalScreen {
        Main => "main",
        Mouse => "mouse",
        MacosMenuBar => "macos-menu-bar",
    }
}

config_enum! {
    /// See quick-terminal-space-behavior
    pub enum QuickTerminalSpaceBehavior {
        Remain => "remain",
        Move => "move",
    }
}

config_enum! {
    /// See quick-terminal-keyboard-interactivity
    pub enum QuickTerminalKeyboardInteractivity {
        None => "none",
        OnDemand => "on-demand",
        Exclusive => "exclusive",
    }
}

config_enum! {
    /// See grapheme-width-method
    pub enum GraphemeWidthMethod {
        Legacy => "legacy",
        Unicode => "unicode",
    }
}

config_enum! {
    /// See alpha-blending
    pub enum AlphaBlending {
        Native => "native",
        Linear => "linear",
        LinearCorrected => "linear-corrected",
    }
}

impl AlphaBlending {
    #[must_use]
    pub fn is_linear(&self) -> bool {
        match self {
            AlphaBlending::Native => false,
            AlphaBlending::Linear | AlphaBlending::LinearCorrected => true,
        }
    }
}

config_enum! {
    /// See background-image-position
    pub enum BackgroundImagePosition {
        TopLeft => "top-left",
        TopCenter => "top-center",
        TopRight => "top-right",
        CenterLeft => "center-left",
        CenterCenter => "center-center",
        CenterRight => "center-right",
        BottomLeft => "bottom-left",
        BottomCenter => "bottom-center",
        BottomRight => "bottom-right",
        Center => "center",
    }
}

config_enum! {
    /// See background-image-fit
    pub enum BackgroundImageFit {
        Contain => "contain",
        Cover => "cover",
        Stretch => "stretch",
        None => "none",
    }
}

config_packed! {
    /// See freetype-load-flag
    pub struct FreetypeLoadFlags {
        // The defaults here at the time of writing this match the defaults
        // for Freetype itself. Ghostty hasn't made any opinionated changes
        // to these defaults. (Strictly speaking, `light` isn't FreeType's
        // own default, but appears to be the effective default with most
        // Fontconfig-aware software using FreeType, so until Ghostty
        // implements Fontconfig support we default to `light`.)
        "hinting" => hinting: true,
        "force-autohint" => force_autohint: false,
        "monochrome" => monochrome: false,
        "autohint" => autohint: true,
        "light" => light: true,
    }
}

config_enum! {
    /// See linux-cgroup
    pub enum LinuxCgroup {
        Never => "never",
        Always => "always",
        SingleInstance => "single-instance",
    }
}

config_enum! {
    /// See async-backend
    pub enum AsyncBackend {
        Auto => "auto",
        Epoll => "epoll",
        IoUring => "io_uring",
    }
}

config_enum! {
    /// See auto-updates
    pub enum AutoUpdate {
        Off => "off",
        Check => "check",
        Download => "download",
    }
}

/// See background-blur
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundBlur {
    #[default]
    False,
    True,
    MacosGlassRegular,
    MacosGlassClear,
    Radius(u8),
}

impl BackgroundBlur {
    #[must_use]
    pub fn enabled(&self) -> bool {
        match self {
            BackgroundBlur::False => false,
            BackgroundBlur::True => true,
            BackgroundBlur::Radius(v) => *v > 0,
            // We treat these as true because they both imply some blur!
            // This has the effect of making the standard blur happen on
            // Linux.
            BackgroundBlur::MacosGlassRegular | BackgroundBlur::MacosGlassClear => true,
        }
    }
}

impl ConfigField for BackgroundBlur {
    fn c_value(&self) -> CValue {
        // Sentinel values for the macOS glass styles, like Zig's `cval`.
        CValue::Short(match self {
            BackgroundBlur::False => 0,
            BackgroundBlur::True => 20,
            BackgroundBlur::Radius(v) => i16::from(*v),
            BackgroundBlur::MacosGlassRegular => -1,
            BackgroundBlur::MacosGlassClear => -2,
        })
    }

    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let input = crate::cli_args::text_value(input)?;
        let Some(input) = input else {
            // Emulate behavior for bools
            *self = BackgroundBlur::True;
            return Ok(());
        };

        // Try to parse normal bools
        if let Ok(b) = parse_bool(input) {
            *self = if b {
                BackgroundBlur::True
            } else {
                BackgroundBlur::False
            };
            return Ok(());
        }

        // Try to parse enums (only void variants can be parsed this way)
        match input {
            "macos-glass-regular" => {
                *self = BackgroundBlur::MacosGlassRegular;
                return Ok(());
            }
            "macos-glass-clear" => {
                *self = BackgroundBlur::MacosGlassClear;
                return Ok(());
            }
            _ => {}
        }

        let mut r = 0u8;
        r.parse_cli(Some(input))
            .map_err(|_| ArgError::InvalidValue)?;
        *self = BackgroundBlur::Radius(r);
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        match self {
            BackgroundBlur::False => f.format_entry(&false),
            BackgroundBlur::True => f.format_entry(&true),
            BackgroundBlur::Radius(v) => f.format_entry(v),
            BackgroundBlur::MacosGlassRegular => fmt_str(f, "macos-glass-regular"),
            BackgroundBlur::MacosGlassClear => fmt_str(f, "macos-glass-clear"),
        }
    }
}

/// See window-decoration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum WindowDecoration {
    #[default]
    Auto,
    Client,
    Server,
    None,
}

impl WindowDecoration {
    pub const NAMES: &'static [&'static str] = &["auto", "client", "server", "none"];

    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            WindowDecoration::Auto => "auto",
            WindowDecoration::Client => "client",
            WindowDecoration::Server => "server",
            WindowDecoration::None => "none",
        }
    }

    pub fn parse(input: Option<&str>) -> Result<WindowDecoration, ArgError> {
        let Some(input) = input else {
            return Ok(WindowDecoration::Auto);
        };

        if let Ok(b) = parse_bool(input) {
            return Ok(if b {
                WindowDecoration::Auto
            } else {
                WindowDecoration::None
            });
        }
        match input {
            "auto" => Ok(WindowDecoration::Auto),
            "client" => Ok(WindowDecoration::Client),
            "server" => Ok(WindowDecoration::Server),
            "none" => Ok(WindowDecoration::None),
            _ => Err(ArgError::InvalidValue),
        }
    }
}

impl ConfigField for WindowDecoration {
    const VALID_VALUES: Option<&'static [&'static str]> = Some(Self::NAMES);

    fn c_value(&self) -> CValue {
        CValue::EnumName(self.as_str())
    }

    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let input = crate::cli_args::text_value(input)?;
        *self = Self::parse(input)?;
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        fmt_str(f, self.as_str())
    }
}

/// See theme
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Theme {
    pub light: EncodedOs,
    pub dark: EncodedOs,
}

impl ConfigField for Theme {
    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        if input.is_empty() {
            return Err(ArgError::ValueRequired);
        }

        // If there is a comma, equal sign, or colon, then we assume that
        // we're parsing a light/dark mode theme pair. Note that "=" isn't
        // actually valid for setting a light/dark mode pair but I anticipate
        // it'll be a common typo.
        if input.iter().any(|b| matches!(b, b',' | b'=' | b':')) {
            let mut light: Option<EncodedOs> = None;
            let mut dark: Option<EncodedOs> = None;
            crate::cli_args::parse_auto_struct_bytes_with(input, |k, v| {
                match k {
                    "light" => light = Some(EncodedOs::from_bytes(v)),
                    "dark" => dark = Some(EncodedOs::from_bytes(v)),
                    _ => return Ok(false),
                }
                Ok(true)
            })?;
            // Both fields are required.
            *self = Theme {
                light: light.ok_or(ArgError::InvalidValue)?,
                dark: dark.ok_or(ArgError::InvalidValue)?,
            };
            return Ok(());
        }

        // Trim our value and set the value to the specified value directly.
        let trimmed = crate::cli_args::trim_bytes(input);
        *self = Theme {
            light: EncodedOs::from_bytes(trimmed),
            dark: EncodedOs::from_bytes(trimmed),
        };
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        if self.light == self.dark {
            return fmt_str(f, self.light.as_encoded());
        }
        fmt_str(
            f,
            &format!(
                "light:{},dark:{}",
                self.light.as_encoded(),
                self.dark.as_encoded()
            ),
        )
    }
}

/// A duration, stored in nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Duration {
    /// Duration in nanoseconds
    pub duration: u64,
}

const NS_PER_US: u64 = 1000;
const NS_PER_MS: u64 = 1000 * NS_PER_US;
const NS_PER_S: u64 = 1000 * NS_PER_MS;
const NS_PER_MIN: u64 = 60 * NS_PER_S;
const NS_PER_HOUR: u64 = 60 * NS_PER_MIN;
const NS_PER_DAY: u64 = 24 * NS_PER_HOUR;
const NS_PER_WEEK: u64 = 7 * NS_PER_DAY;

/// Duration units. The order is important as the first factor that matches
/// will be the default unit that is used for formatting.
pub const DURATION_UNITS: &[(&str, u64)] = &[
    ("y", 365 * NS_PER_DAY),
    ("w", NS_PER_WEEK),
    ("d", NS_PER_DAY),
    ("h", NS_PER_HOUR),
    ("m", NS_PER_MIN),
    ("s", NS_PER_S),
    ("ms", NS_PER_MS),
    ("µs", NS_PER_US),
    ("us", NS_PER_US),
    ("ns", 1),
];

/// `std.fmt.parseUnsigned(u64, s, 10)`.
fn parse_unsigned_u64(s: &[u8]) -> Option<u64> {
    if s.is_empty() || !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut acc: u64 = 0;
    for &c in s {
        acc = acc.checked_mul(10)?.checked_add(u64::from(c - b'0'))?;
    }
    Some(acc)
}

impl Duration {
    #[must_use]
    pub fn round(self, to: u64) -> Duration {
        Duration {
            duration: self.duration / to * to,
        }
    }

    #[must_use]
    pub fn lte(self, other: Duration) -> bool {
        self.duration <= other.duration
    }

    pub fn parse(input: Option<&str>) -> Result<Duration, ArgError> {
        let mut remaining = input.ok_or(ArgError::ValueRequired)?.as_bytes();

        let mut value: Option<u64> = None;
        while !remaining.is_empty() {
            // Skip over whitespace before the number
            while let Some(&c) = remaining.first() {
                if matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) {
                    remaining = &remaining[1..];
                } else {
                    break;
                }
            }

            // There was whitespace at the end, that's OK
            if remaining.is_empty() {
                break;
            }

            // Find the longest number
            let number: u64 = {
                let mut prev_number: Option<u64> = None;
                let mut prev_remaining: Option<&[u8]> = None;
                let mut result: Option<Option<u64>> = None;
                for index in 1..=remaining.len() {
                    if let Some(n) = parse_unsigned_u64(&remaining[..index]) {
                        prev_number = Some(n);
                        prev_remaining = Some(&remaining[index..]);
                    } else {
                        if let Some(prev) = prev_remaining {
                            remaining = prev;
                        }
                        result = Some(prev_number);
                        break;
                    }
                }
                match result {
                    Some(r) => r,
                    None => {
                        if let Some(prev) = prev_remaining {
                            remaining = prev;
                        }
                        prev_number
                    }
                }
            }
            .ok_or(ArgError::InvalidValue)?;

            // A number without a unit is invalid unless the number is
            // exactly zero. In that case, the unit is unambiguous since
            // its all the same.
            if remaining.is_empty() {
                if number == 0 {
                    value = Some(0);
                    break;
                }
                return Err(ArgError::InvalidValue);
            }

            // Find the longest matching unit. Needs to be the longest matching
            // to distinguish 'm' from 'ms'.
            let factor: u64 = {
                let mut prev_factor: Option<u64> = None;
                let mut prev_index: Option<usize> = None;
                for index in 1..=remaining.len() {
                    let next = DURATION_UNITS
                        .iter()
                        .find(|(name, _)| name.as_bytes() == &remaining[..index])
                        .map(|(_, f)| *f);
                    if let Some(next) = next {
                        prev_factor = Some(next);
                        prev_index = Some(index);
                    }
                }
                if let Some(index) = prev_index {
                    remaining = &remaining[index..];
                }
                prev_factor
            }
            .ok_or(ArgError::InvalidValue)?;

            // Add our time value to the total. Avoid overflow with saturating math.
            let diff = number.saturating_mul(factor);
            value = Some(value.unwrap_or(0).saturating_add(diff));
        }

        match value {
            Some(v) => Ok(Duration { duration: v }),
            None => Err(ArgError::ValueRequired),
        }
    }

    /// Format like Zig's `Duration.format`, e.g. `1m 30s`.
    pub fn format(&self, writer: &mut dyn fmt::Write) -> fmt::Result {
        let mut value = self.duration;
        let mut i = 0usize;
        for (name, factor) in DURATION_UNITS {
            if value >= *factor {
                if i > 0 {
                    writer.write_str(" ")?;
                }
                let remainder = value % factor;
                let quotient = (value - remainder) / factor;
                write!(writer, "{quotient}{name}")?;
                value = remainder;
                i += 1;
            }
        }
        Ok(())
    }

    /// Convenience function to convert to milliseconds since many OS and
    /// library timing functions operate on that timescale.
    #[must_use]
    pub fn as_milliseconds(&self) -> u32 {
        u32::try_from(self.duration / NS_PER_MS).unwrap_or(u32::MAX)
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.format(f)
    }
}

impl ConfigField for Duration {
    fn c_value(&self) -> CValue {
        CValue::Usize(self.as_milliseconds() as usize)
    }

    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let input = crate::cli_args::text_value(input)?;
        *self = Duration::parse(input)?;
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        fmt_str(f, &self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowPadding {
    pub top_left: u32,
    pub bottom_right: u32,
}

impl WindowPadding {
    pub fn parse(input: Option<&str>) -> Result<WindowPadding, ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        let p = |s: &str| s.parse::<u32>().map_err(|_| ArgError::InvalidValue);

        if let Some(idx) = input.find(',') {
            let left = p(trim_ws(&input[..idx]))?;
            let right = p(trim_ws(&input[idx + 1..]))?;
            Ok(WindowPadding {
                top_left: left,
                bottom_right: right,
            })
        } else {
            let value = p(trim_ws(input))?;
            Ok(WindowPadding {
                top_left: value,
                bottom_right: value,
            })
        }
    }
}

impl ConfigField for WindowPadding {
    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let input = crate::cli_args::text_value(input)?;
        *self = WindowPadding::parse(input)?;
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        if self.top_left == self.bottom_right {
            fmt_str(f, &self.top_left.to_string())
        } else {
            fmt_str(f, &format!("{},{}", self.top_left, self.bottom_right))
        }
    }
}

config_enum! {
    /// See scrollbar
    pub enum Scrollbar {
        System => "system",
        Never => "never",
    }
}

config_packed! {
    /// See scroll-to-bottom
    pub struct ScrollToBottom {
        "keystroke" => keystroke: true,
        "output" => output: false,
    }
}

config_enum! {
    /// See notify-on-command-finish
    pub enum NotifyOnCommandFinish {
        Never => "never",
        Unfocused => "unfocused",
        Always => "always",
    }
}

config_packed! {
    /// See notify-on-command-finish-action
    pub struct NotifyOnCommandFinishAction {
        "bell" => bell: true,
        "notify" => notify: false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::approx_constant)]
mod tests {
    use super::*;
    use crate::formatter::format_entry;

    fn fe<T: ConfigField>(name: &str, v: &T) -> String {
        let mut s = String::new();
        format_entry(name, v, &mut s).unwrap();
        s
    }

    fn parse_rc(list: &mut RepeatableCommand, s: &str) {
        list.parse_cli(Some(s)).unwrap();
    }

    #[test]
    fn repeatable_command_parse_cli() {
        let mut list = RepeatableCommand::default();
        parse_rc(&mut list, "title:Foo,action:ignore");
        parse_rc(
            &mut list,
            "title:Bar,description:bobr,action:text:ale bydle",
        );
        parse_rc(
            &mut list,
            "title:Quux,description:boo,action:increase_font_size:2.5",
        );
        parse_rc(
            &mut list,
            "title:Baz,description:Raspberry Pie,action:set_font_size:3.14",
        );

        assert_eq!(list.value.len(), 4);

        assert_eq!(list.value[0].action, Action::Ignore);
        assert_eq!(list.value[0].title, "Foo");

        assert_eq!(list.value[1].action, Action::Text("ale bydle".into()));
        assert_eq!(list.value[1].title, "Bar");
        assert_eq!(list.value[1].description, "bobr");

        assert_eq!(list.value[2].action, Action::IncreaseFontSize(2.5));
        assert_eq!(list.value[2].title, "Quux");
        assert_eq!(list.value[2].description, "boo");

        assert_eq!(list.value[3].action, Action::SetFontSize(3.14));
        assert_eq!(list.value[3].title, "Baz");
        assert_eq!(list.value[3].description, "Raspberry Pie");

        list.parse_cli(Some("")).unwrap();
        assert_eq!(list.value.len(), 0);
    }

    #[test]
    fn repeatable_command_cval() {
        let mut list = RepeatableCommand::default();
        list.parse_cli(Some("title:Foo,action:ignore")).unwrap();
        list.parse_cli(Some("title:Bar,description:bobr,action:text:ale bydle"))
            .unwrap();
        assert_eq!(list.value.len(), 2);

        let CValue::Commands(cv) = list.c_value() else {
            panic!("expected commands");
        };
        assert_eq!(cv.len(), 2);

        // First entry
        assert_eq!(cv[0].title, "Foo");
        assert_eq!(cv[0].action_key, "ignore");
        assert_eq!(cv[0].action, "ignore");

        // Second entry
        assert_eq!(cv[1].title, "Bar");
        assert_eq!(cv[1].description, "bobr");
        assert_eq!(cv[1].action_key, "text");
        assert_eq!(cv[1].action, "text:ale bydle");
    }

    #[test]
    fn repeatable_command_cval_cleared() {
        let mut list = RepeatableCommand::default();
        list.parse_cli(Some("title:Foo,action:ignore")).unwrap();
        let CValue::Commands(cv) = list.c_value() else {
            panic!("expected commands");
        };
        assert_eq!(cv.len(), 1);

        list.parse_cli(Some("")).unwrap();
        let CValue::Commands(cv) = list.c_value() else {
            panic!("expected commands");
        };
        assert_eq!(cv.len(), 0);
    }

    #[test]
    fn repeatable_command_format_config_empty() {
        let list = RepeatableCommand::default();
        assert_eq!(fe("a", &list), "a = \n");
    }

    #[test]
    fn repeatable_command_format_config_single_item() {
        let mut list = RepeatableCommand::default();
        parse_rc(&mut list, "title:Bobr, action:text:Bober");
        assert_eq!(fe("a", &list), "a = title:\"Bobr\",action:\"text:Bober\"\n");
    }

    #[test]
    fn repeatable_command_format_config_multiple_items() {
        let mut list = RepeatableCommand::default();
        parse_rc(&mut list, "title:Bobr, action:text:kurwa");
        parse_rc(
            &mut list,
            "title:Ja,   description: pierdole,  action:text:jakie bydle",
        );
        assert_eq!(
            fe("a", &list),
            "a = title:\"Bobr\",action:\"text:kurwa\"\na = title:\"Ja\",description:\"pierdole\",action:\"text:jakie bydle\"\n"
        );
    }

    #[test]
    fn repeatable_command_parse_cli_commas() {
        {
            let mut list = RepeatableCommand::default();
            parse_rc(&mut list, "title:\"Bo,br\",action:\"text:kur,wa\"");
            assert_eq!(list.value.len(), 1);
            let item = &list.value[0];
            assert_eq!(item.title, "Bo,br");
            assert_eq!(item.description, "");
            assert_eq!(item.action, Action::Text("kur,wa".into()));
        }
        {
            let mut list = RepeatableCommand::default();
            parse_rc(
                &mut list,
                "title:\"Bo,br\",description:\"abc,def\",action:text:kurwa",
            );
            assert_eq!(list.value.len(), 1);
            let item = &list.value[0];
            assert_eq!(item.title, "Bo,br");
            assert_eq!(item.description, "abc,def");
            assert_eq!(item.action, Action::Text("kurwa".into()));
        }
    }

    #[test]
    fn mouse_scroll_multiplier_parse() {
        let eps = 0.00001;
        let base = MouseScrollMultiplier {
            precision: 0.1,
            discrete: 3.0,
        };
        let near = |a: f64, b: f64| (a - b).abs() < eps;

        let mut args = base;
        args.parse_cli(Some("3")).unwrap();
        assert!(near(args.precision, 3.0) && near(args.discrete, 3.0));

        args = base;
        args.parse_cli(Some("precision:1")).unwrap();
        assert!(near(args.precision, 1.0) && near(args.discrete, 3.0));

        args = base;
        args.parse_cli(Some("discrete:5")).unwrap();
        assert!(near(args.precision, 0.1) && near(args.discrete, 5.0));

        args = base;
        args.parse_cli(Some("precision:3,discrete:7")).unwrap();
        assert!(near(args.precision, 3.0) && near(args.discrete, 7.0));

        args = base;
        args.parse_cli(Some("discrete:8,precision:6")).unwrap();
        assert!(near(args.precision, 6.0) && near(args.discrete, 8.0));

        args = MouseScrollMultiplier::DEFAULT;
        for bad in [
            "foo:1",
            "precision:bar",
            "precision:1,discrete:3,foo:5",
            "precision:1,,discrete:3",
            ",precision:1,discrete:3",
        ] {
            assert_eq!(
                args.parse_cli(Some(bad)),
                Err(ArgError::InvalidValue),
                "{bad}"
            );
        }
    }

    #[test]
    fn format_entry_mouse_scroll_multiplier() {
        let args = MouseScrollMultiplier {
            precision: 1.5,
            discrete: 2.5,
        };
        assert_eq!(
            fe("mouse-scroll-multiplier", &args),
            "mouse-scroll-multiplier = precision:1.5,discrete:2.5\n"
        );
    }

    #[test]
    fn parse_quick_terminal_size() {
        use QuickTerminalSizeValue::{Percentage, Pixels};
        let mut v = QuickTerminalSize::default();

        v.parse_cli(Some("50%")).unwrap();
        assert_eq!(v.primary, Some(Percentage(50.0)));
        assert_eq!(v.secondary, None);

        v.parse_cli(Some("200px")).unwrap();
        assert_eq!(v.primary, Some(Pixels(200)));
        assert_eq!(v.secondary, None);

        v.parse_cli(Some("50%,200px")).unwrap();
        assert_eq!(v.primary, Some(Percentage(50.0)));
        assert_eq!(v.secondary, Some(Pixels(200)));

        assert_eq!(v.parse_cli(None), Err(ArgError::ValueRequired));
        assert_eq!(v.parse_cli(Some("")), Err(ArgError::ValueRequired));
        assert_eq!(v.parse_cli(Some("69px,")), Err(ArgError::ValueRequired));
        assert_eq!(
            v.parse_cli(Some("69px,42%,69px")),
            Err(ArgError::other("TooManyArguments"))
        );

        assert_eq!(
            v.parse_cli(Some("420")),
            Err(ArgError::other("MissingUnit"))
        );
        assert_eq!(
            v.parse_cli(Some("bobr")),
            Err(ArgError::other("MissingUnit"))
        );
        assert_eq!(v.parse_cli(Some("bobr%")), Err(ArgError::InvalidValue));
        assert_eq!(v.parse_cli(Some("-32%")), Err(ArgError::InvalidValue));
        assert_eq!(v.parse_cli(Some("-69px")), Err(ArgError::InvalidValue));
    }

    #[test]
    fn calculate_quick_terminal_size() {
        use QuickTerminalPosition::{Center, Left, Top};
        use QuickTerminalSizeValue::{Percentage, Pixels};
        let d = |width, height| QuickTerminalDimensions { width, height };
        let land = d(2560, 1600);
        let port = d(1600, 2560);

        let size = QuickTerminalSize::default();
        assert_eq!(size.calculate(Top, land), d(2560, 400));
        assert_eq!(size.calculate(Left, land), d(400, 1600));
        assert_eq!(size.calculate(Center, land), d(800, 400));
        assert_eq!(size.calculate(Center, port), d(400, 800));

        let size = QuickTerminalSize {
            primary: Some(Percentage(20.0)),
            secondary: None,
        };
        assert_eq!(size.calculate(Top, land), d(2560, 320));
        assert_eq!(size.calculate(Left, land), d(512, 1600));
        assert_eq!(size.calculate(Center, land), d(512, 400));
        assert_eq!(size.calculate(Center, port), d(400, 512));

        let size = QuickTerminalSize {
            primary: Some(Pixels(600)),
            secondary: None,
        };
        assert_eq!(size.calculate(Top, land), d(2560, 600));
        assert_eq!(size.calculate(Left, land), d(600, 1600));
        assert_eq!(size.calculate(Center, land), d(600, 400));
        assert_eq!(size.calculate(Center, port), d(400, 600));

        let size = QuickTerminalSize {
            primary: Some(Percentage(69.0)),
            secondary: Some(Pixels(420)),
        };
        assert_eq!(size.calculate(Top, land), d(420, 1104));
        assert_eq!(size.calculate(Left, land), d(1766, 420));
        assert_eq!(size.calculate(Center, land), d(1766, 420));
        assert_eq!(size.calculate(Center, port), d(420, 1766));
    }

    #[test]
    fn parse_background_blur() {
        let mut v = BackgroundBlur::default();

        v.parse_cli(None).unwrap();
        assert_eq!(v, BackgroundBlur::True);
        v.parse_cli(Some("true")).unwrap();
        assert_eq!(v, BackgroundBlur::True);
        v.parse_cli(Some("false")).unwrap();
        assert_eq!(v, BackgroundBlur::False);
        v.parse_cli(Some("42")).unwrap();
        assert_eq!(v, BackgroundBlur::Radius(42));
        v.parse_cli(Some("macos-glass-regular")).unwrap();
        assert_eq!(v, BackgroundBlur::MacosGlassRegular);
        v.parse_cli(Some("macos-glass-clear")).unwrap();
        assert_eq!(v, BackgroundBlur::MacosGlassClear);

        assert_eq!(v.parse_cli(Some("")), Err(ArgError::InvalidValue));
        assert_eq!(v.parse_cli(Some("aaaa")), Err(ArgError::InvalidValue));
        assert_eq!(v.parse_cli(Some("420")), Err(ArgError::InvalidValue));
    }

    #[test]
    fn parse_window_decoration() {
        assert_eq!(WindowDecoration::parse(None), Ok(WindowDecoration::Auto));
        assert_eq!(
            WindowDecoration::parse(Some("true")),
            Ok(WindowDecoration::Auto)
        );
        assert_eq!(
            WindowDecoration::parse(Some("false")),
            Ok(WindowDecoration::None)
        );
        assert_eq!(
            WindowDecoration::parse(Some("server")),
            Ok(WindowDecoration::Server)
        );
        assert_eq!(
            WindowDecoration::parse(Some("client")),
            Ok(WindowDecoration::Client)
        );
        assert_eq!(
            WindowDecoration::parse(Some("auto")),
            Ok(WindowDecoration::Auto)
        );
        assert_eq!(
            WindowDecoration::parse(Some("none")),
            Ok(WindowDecoration::None)
        );
        assert_eq!(
            WindowDecoration::parse(Some("")),
            Err(ArgError::InvalidValue)
        );
        assert_eq!(
            WindowDecoration::parse(Some("aaaa")),
            Err(ArgError::InvalidValue)
        );
    }

    #[test]
    fn parse_theme() {
        // Single
        let mut v = Theme::default();
        v.parse_cli(Some("foo")).unwrap();
        assert_eq!((v.light.as_encoded(), v.dark.as_encoded()), ("foo", "foo"));

        // Single whitespace
        let mut v = Theme::default();
        v.parse_cli(Some("  foo  ")).unwrap();
        assert_eq!((v.light.as_encoded(), v.dark.as_encoded()), ("foo", "foo"));

        // Light/dark
        let mut v = Theme::default();
        v.parse_cli(Some(" light:foo,  dark : bar  ")).unwrap();
        assert_eq!((v.light.as_encoded(), v.dark.as_encoded()), ("foo", "bar"));

        let mut v = Theme::default();
        assert_eq!(v.parse_cli(None), Err(ArgError::ValueRequired));
        assert_eq!(v.parse_cli(Some("")), Err(ArgError::ValueRequired));
        assert_eq!(v.parse_cli(Some("light:foo")), Err(ArgError::InvalidValue));
        assert_eq!(v.parse_cli(Some("dark:foo")), Err(ArgError::InvalidValue));
    }

    #[test]
    fn parse_window_padding() {
        let wp = |a, b| WindowPadding {
            top_left: a,
            bottom_right: b,
        };
        assert_eq!(WindowPadding::parse(Some("100")), Ok(wp(100, 100)));
        assert_eq!(WindowPadding::parse(Some("100,200")), Ok(wp(100, 200)));
        // Trim whitespace
        assert_eq!(WindowPadding::parse(Some(" 100 , 200 ")), Ok(wp(100, 200)));

        assert_eq!(WindowPadding::parse(None), Err(ArgError::ValueRequired));
        assert_eq!(WindowPadding::parse(Some("")), Err(ArgError::InvalidValue));
        assert_eq!(WindowPadding::parse(Some("a")), Err(ArgError::InvalidValue));
    }

    fn pd(s: &str) -> Result<u64, ArgError> {
        Duration::parse(Some(s)).map(|d| d.duration)
    }

    #[test]
    fn parse_duration() {
        for (name, _) in DURATION_UNITS {
            assert_eq!(pd(&format!("0{name}")), Ok(0));
        }
        for (name, factor) in DURATION_UNITS {
            assert_eq!(pd(&format!("1{name}")), Ok(*factor));
        }

        assert_eq!(pd("0"), Ok(0));
        assert_eq!(pd("100ns"), Ok(100));
        assert_eq!(pd("1µs"), Ok(1000));
        assert_eq!(pd("1µs1ns"), Ok(1001));
        assert_eq!(pd("1µs 1ns"), Ok(1001));
        assert_eq!(pd(" 1µs1ns"), Ok(1001));
        assert_eq!(pd("1µs1ns "), Ok(1001));
        assert_eq!(pd("30s"), Ok(30 * NS_PER_S));
        assert_eq!(pd("584y 49w 23h 34m 33s 709ms 551µs 615ns"), Ok(u64::MAX));

        // Overflow
        assert_eq!(pd("600y"), Ok(u64::MAX));

        // Repeated units
        assert_eq!(pd("100ns100ns"), Ok(200));

        assert_eq!(Duration::parse(None), Err(ArgError::ValueRequired));
        assert_eq!(pd(""), Err(ArgError::ValueRequired));
        assert_eq!(pd("1"), Err(ArgError::InvalidValue));
        assert_eq!(pd("s"), Err(ArgError::InvalidValue));
        assert_eq!(pd("1x"), Err(ArgError::InvalidValue));
        assert_eq!(pd("1 "), Err(ArgError::InvalidValue));
    }

    #[test]
    fn test_format() {
        for (name, factor) in DURATION_UNITS {
            let d = Duration { duration: *factor };
            let expected = if *name == "us" {
                "1µs".to_string()
            } else {
                format!("1{name}")
            };
            assert_eq!(d.to_string(), expected);
        }
    }

    #[test]
    fn test_entry_formatter() {
        let p = Duration { duration: u64::MAX };
        assert_eq!(fe("a", &p), "a = 584y 49w 23h 34m 33s 709ms 551µs 615ns\n");
    }

    #[test]
    fn extra_enum_and_packed_formatting() {
        assert_eq!(fe("a", &BackgroundBlur::Radius(5)), "a = 5\n");
        assert_eq!(fe("a", &BackgroundBlur::True), "a = true\n");
        assert_eq!(
            fe("a", &BackgroundBlur::MacosGlassClear),
            "a = macos-glass-clear\n"
        );
        assert_eq!(
            fe("a", &ScrollToBottom::default()),
            "a = keystroke,no-output\n"
        );
        assert_eq!(fe("a", &QuickTerminalSize::default()), "");
        let mut q = QuickTerminalSize::default();
        q.parse_cli(Some("50%, 200px")).unwrap();
        assert_eq!(fe("a", &q), "a = 50%,200px\n");
        assert_eq!(
            fe(
                "a",
                &WindowPadding {
                    top_left: 1,
                    bottom_right: 2
                }
            ),
            "a = 1,2\n"
        );
        let mut c = CopyOnSelect::default();
        assert_eq!(c.parse_cli(Some("x")), Err(ArgError::InvalidValue));
        c.parse_cli(Some("clipboard")).unwrap();
        assert_eq!(c, CopyOnSelect::Clipboard);
    }
}
