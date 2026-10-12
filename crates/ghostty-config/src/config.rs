//! The configuration struct (port of `config/Config.zig`).
//!
//! Every option is declared once in the [`config_struct!`](crate::config_struct)
//! invocation below with its exact name, type, default value and doc comment.
//! The doc comments are user-visible (`ghostty +show-config --docs`).

use std::collections::HashSet;

use crate::{
    cli_args::{self, ArgError, ArgIter, CliTarget},
    command::Command,
    conditional::{self, Conditional},
    config_enum,
    diagnostics::{Diagnostic, DiagnosticList},
    io::RepeatableReadableIO,
    key::Key,
    keybinds::Keybinds,
    macros::ConfigStruct,
    metric_modifier::MetricModifier,
    path::{Path, RepeatablePath},
    repeatable_string_map::RepeatableStringMap,
    types_a::*,
    types_b::*,
};

const NS_PER_S: u64 = 1_000_000_000;
const NS_PER_MS: u64 = 1_000_000;

config_enum! {
    /// The release channel of the build (Zig: `build_config.ReleaseChannel`).
    pub enum ReleaseChannel {
        Stable => "stable",
        Tip => "tip",
    }
}

crate::config_struct! {
    pub struct Config {
        /// Set Ghostty's graphical user interface language to a language other than the
        /// system default language. For example:
        ///
        ///     language = de
        ///
        /// will force the strings in Ghostty's graphical user interface to be in German
        /// rather than the system default.
        ///
        /// This will not affect the language used by programs run _within_ Ghostty.
        /// Those will continue to use the default system language. There are also many
        /// non-GUI elements in Ghostty that are not translated - this setting will have
        /// no effect on those.
        ///
        /// Warning: This setting cannot be reloaded at runtime. To change the language
        /// you must fully restart Ghostty.
        ///
        /// GTK only.
        /// Available since 1.3.0.
        "language" => language: Option<String> = None,

        /// The font families to use.
        ///
        /// You can generate the list of valid values using the CLI:
        ///
        ///     ghostty +list-fonts
        ///
        /// This configuration can be repeated multiple times to specify preferred
        /// fallback fonts when the requested codepoint is not available in the primary
        /// font. This is particularly useful for multiple languages, symbolic fonts,
        /// etc.
        ///
        /// Notes on emoji specifically: On macOS, Ghostty by default will always use
        /// Apple Color Emoji and on Linux will always use Noto Emoji. You can
        /// override this behavior by specifying a font family here that contains
        /// emoji glyphs.
        ///
        /// The specific styles (bold, italic, bold italic) do not need to be
        /// explicitly set. If a style is not set, then the regular style (font-family)
        /// will be searched for stylistic variants. If a stylistic variant is not
        /// found, Ghostty will use the regular style. This prevents falling back to a
        /// different font family just to get a style such as bold. This also applies
        /// if you explicitly specify a font family for a style. For example, if you
        /// set `font-family-bold = FooBar` and "FooBar" cannot be found, Ghostty will
        /// use whatever font is set for `font-family` for the bold style.
        ///
        /// Finally, some styles may be synthesized if they are not supported.
        /// For example, if a font does not have an italic style and no alternative
        /// italic font is specified, Ghostty will synthesize an italic style by
        /// applying a slant to the regular style. If you want to disable these
        /// synthesized styles then you can use the `font-style` configurations
        /// as documented below.
        ///
        /// You can disable styles completely by using the `font-style` set of
        /// configurations. See the documentation for `font-style` for more information.
        ///
        /// If you want to overwrite a previous set value rather than append a fallback,
        /// specify the value as `""` (empty string) to reset the list and then set the
        /// new values. For example:
        ///
        ///     font-family = ""
        ///     font-family = "My Favorite Font"
        ///
        /// Setting any of these as CLI arguments will automatically clear the
        /// values set in configuration files so you don't need to specify
        /// `--font-family=""` before setting a new value. You only need to specify
        /// this within config files if you want to clear previously set values in
        /// configuration files or on the CLI if you want to clear values set on the
        /// CLI.
        "font-family" => font_family: RepeatableString = RepeatableString::default(),

        "font-family-bold" => font_family_bold: RepeatableString = RepeatableString::default(),

        "font-family-italic" => font_family_italic: RepeatableString = RepeatableString::default(),

        "font-family-bold-italic" => font_family_bold_italic: RepeatableString = RepeatableString::default(),

        /// The named font style to use for each of the requested terminal font styles.
        /// This looks up the style based on the font style string advertised by the
        /// font itself. For example, "Iosevka Heavy" has a style of "Heavy".
        ///
        /// You can also use these fields to completely disable a font style. If you set
        /// the value of the configuration below to literal `false` then that font style
        /// will be disabled. If the running program in the terminal requests a disabled
        /// font style, the regular font style will be used instead.
        ///
        /// These are only valid if its corresponding font-family is also specified. If
        /// no font-family is specified, then the font-style is ignored unless you're
        /// disabling the font style.
        "font-style" => font_style: FontStyle = FontStyle::Default,

        "font-style-bold" => font_style_bold: FontStyle = FontStyle::Default,

        "font-style-italic" => font_style_italic: FontStyle = FontStyle::Default,

        "font-style-bold-italic" => font_style_bold_italic: FontStyle = FontStyle::Default,

        /// Control whether Ghostty should synthesize a style if the requested style is
        /// not available in the specified font-family.
        ///
        /// Ghostty can synthesize bold, italic, and bold italic styles if the font
        /// does not have a specific style. For bold, this is done by drawing an
        /// outline around the glyph of varying thickness. For italic, this is done by
        /// applying a slant to the glyph. For bold italic, both of these are applied.
        ///
        /// Synthetic styles are not perfect and will generally not look as good
        /// as a font that has the style natively. However, they are useful to
        /// provide styled text when the font does not have the style.
        ///
        /// Set this to "false" or "true" to disable or enable synthetic styles
        /// completely. You can disable specific styles using "no-bold", "no-italic",
        /// and "no-bold-italic". You can disable multiple styles by separating them
        /// with a comma. For example, "no-bold,no-italic".
        ///
        /// Available style keys are: `bold`, `italic`, `bold-italic`.
        ///
        /// If synthetic styles are disabled, then the regular style will be used
        /// instead if the requested style is not available. If the font has the
        /// requested style, then the font will be used as-is since the style is
        /// not synthetic.
        ///
        /// Warning: An easy mistake is to disable `bold` or `italic` but not
        /// `bold-italic`. Disabling only `bold` or `italic` will NOT disable either
        /// in the `bold-italic` style. If you want to disable `bold-italic`, you must
        /// explicitly disable it. You cannot partially disable `bold-italic`.
        ///
        /// By default, synthetic styles are enabled.
        "font-synthetic-style" => font_synthetic_style: FontSyntheticStyle = FontSyntheticStyle::default(),

        /// Apply a font feature. To enable multiple font features you can repeat
        /// this multiple times or use a comma-separated list of feature settings.
        ///
        /// The syntax for feature settings is as follows, where `feat` is a feature:
        ///
        ///   * Enable features with e.g. `feat`, `+feat`, `feat on`, `feat=1`.
        ///   * Disabled features with e.g. `-feat`, `feat off`, `feat=0`.
        ///   * Set a feature value with e.g. `feat=2`, `feat = 3`, `feat 4`.
        ///   * Feature names may be wrapped in quotes, meaning this config should be
        ///     syntactically compatible with the `font-feature-settings` CSS property.
        ///
        /// The syntax is fairly loose, but invalid settings will be silently ignored.
        ///
        /// The font feature will apply to all fonts rendered by Ghostty. A future
        /// enhancement will allow targeting specific faces.
        ///
        /// To disable programming ligatures, use `-calt` since this is the typical
        /// feature name for programming ligatures. To look into what font features
        /// your font has and what they do, use a font inspection tool such as
        /// [fontdrop.info](https://fontdrop.info).
        ///
        /// To generally disable most ligatures, use `-calt, -liga, -dlig`.
        "font-feature" => font_feature: RepeatableString = RepeatableString::default(),

        /// Font size in points. This value can be a non-integer and the nearest integer
        /// pixel size will be selected. If you have a high dpi display where 1pt = 2px
        /// then you can get an odd numbered pixel size by specifying a half point.
        ///
        /// For example, 13.5pt @ 2px/pt = 27px
        ///
        /// Changing this configuration at runtime will only affect existing
        /// terminals that have NOT manually adjusted their font size in some way
        /// (e.g. increased or decreased the font size). Terminals that have manually
        /// adjusted their font size will retain their manually adjusted size.
        /// Otherwise, the font size of existing terminals will be updated on
        /// reload.
        ///
        /// On Linux with GTK, font size is scaled according to both display-wide and
        /// text-specific scaling factors, which are often managed by your desktop
        /// environment (e.g. the GNOME display scale and large text settings).
        "font-size" => font_size: f32 = 12.0,

        /// A repeatable configuration to set one or more font variations values for
        /// a variable font. A variable font is a single font, usually with a filename
        /// ending in `-VF.ttf` or `-VF.otf` that contains one or more configurable axes
        /// for things such as weight, slant, etc. Not all fonts support variations;
        /// only fonts that explicitly state they are variable fonts will work.
        ///
        /// The format of this is `id=value` where `id` is the axis identifier. An axis
        /// identifier is always a 4 character string, such as `wght`. To get the list
        /// of supported axes, look at your font documentation or use a font inspection
        /// tool.
        ///
        /// Invalid ids and values are usually ignored. For example, if a font only
        /// supports weights from 100 to 700, setting `wght=800` will do nothing (it
        /// will not be clamped to 700). You must consult your font's documentation to
        /// see what values are supported.
        ///
        /// Common axes are: `wght` (weight), `slnt` (slant), `ital` (italic), `opsz`
        /// (optical size), `wdth` (width), `GRAD` (gradient), etc.
        "font-variation" => font_variation: RepeatableFontVariation = RepeatableFontVariation::default(),

        "font-variation-bold" => font_variation_bold: RepeatableFontVariation = RepeatableFontVariation::default(),

        "font-variation-italic" => font_variation_italic: RepeatableFontVariation = RepeatableFontVariation::default(),

        "font-variation-bold-italic" => font_variation_bold_italic: RepeatableFontVariation = RepeatableFontVariation::default(),

        /// Force one or a range of Unicode codepoints to map to a specific named font.
        /// This is useful if you want to support special symbols or if you want to use
        /// specific glyphs that render better for your specific font.
        ///
        /// The syntax is `codepoint=fontname` where `codepoint` is either a single
        /// codepoint or a range. Codepoints must be specified as full Unicode
        /// hex values, such as `U+ABCD`. Codepoints ranges are specified as
        /// `U+ABCD-U+DEFG`. You can specify multiple ranges for the same font separated
        /// by commas, such as `U+ABCD-U+DEFG,U+1234-U+5678=fontname`. The font name is
        /// the same value as you would use for `font-family`.
        ///
        /// This configuration can be repeated multiple times to specify multiple
        /// codepoint mappings.
        ///
        /// Changing this configuration at runtime will only affect new terminals,
        /// i.e. new windows, tabs, etc.
        "font-codepoint-map" => font_codepoint_map: RepeatableCodepointMap = RepeatableCodepointMap::default(),

        /// Map specific Unicode codepoints to replacement values when copying text
        /// to clipboard.
        ///
        /// This configuration allows you to replace specific Unicode characters with
        /// other characters or strings when copying terminal content to the clipboard.
        /// This is useful for converting special terminal symbols to more compatible
        /// characters for pasting into other applications.
        ///
        /// The syntax is similar to `font-codepoint-map`:
        /// - Single codepoint: `U+1234=U+ABCD` or `U+1234=replacement_text`
        /// - Codepoint range: `U+1234-U+5678=U+ABCD`
        ///
        /// Examples:
        /// - `clipboard-codepoint-map = U+2500=U+002D` (box drawing horizontal → hyphen)
        /// - `clipboard-codepoint-map = U+2502=U+007C` (box drawing vertical → pipe)
        /// - `clipboard-codepoint-map = U+03A3=SUM` (Greek sigma → "SUM")
        ///
        /// This configuration can be repeated multiple times to specify multiple
        /// mappings. Later entries take priority over earlier ones for overlapping
        /// ranges.
        ///
        /// Note: This only applies to text copying operations, not URL copying.
        "clipboard-codepoint-map" => clipboard_codepoint_map: RepeatableClipboardCodepointMap = RepeatableClipboardCodepointMap::default(),

        /// Draw fonts with a thicker stroke, if supported.
        /// This is currently only supported on macOS.
        "font-thicken" => font_thicken: bool = false,

        /// Strength of thickening when `font-thicken` is enabled.
        ///
        /// Valid values are integers between `0` and `255`. `0` does not correspond to
        /// *no* thickening, rather it corresponds to the lightest available thickening.
        ///
        /// Has no effect when `font-thicken` is set to `false`.
        ///
        /// This is currently only supported on macOS.
        "font-thicken-strength" => font_thicken_strength: u8 = 255,

        /// Locations to break font shaping into multiple runs.
        ///
        /// A "run" is a contiguous segment of text that is shaped together. "Shaping"
        /// is the process of converting text (codepoints) into glyphs (renderable
        /// characters). This is how ligatures are formed, among other things.
        /// For example, if a coding font turns "!=" into a single glyph, then it
        /// must see "!" and "=" next to each other in a single run. When a run
        /// is broken, the text is shaped separately. To continue our example, if
        /// "!" is at the end of one run and "=" is at the start of the next run,
        /// then the ligature will not be formed.
        ///
        /// Ghostty breaks runs at certain points to improve readability or usability.
        /// For example, Ghostty by default will break runs under the cursor so that
        /// text editing can see the individual characters rather than a ligature.
        /// This configuration lets you configure this behavior.
        ///
        /// Combine values with a comma to set multiple options. Prefix an
        /// option with "no-" to disable it. Enabling and disabling options
        /// can be done at the same time.
        ///
        /// Available options:
        ///
        ///   * `cursor` - Break runs under the cursor.
        ///
        /// Available since: 1.2.0
        "font-shaping-break" => font_shaping_break: FontShapingBreak = FontShapingBreak::default(),

        /// What color space to use when performing alpha blending.
        ///
        /// This affects the appearance of text and of any images with transparency.
        /// Additionally, custom shaders will receive colors in the configured space.
        ///
        /// On macOS the default is `native`, on all other platforms the default is
        /// `linear-corrected`.
        ///
        /// Valid values:
        ///
        /// * `native` - Perform alpha blending in the native color space for the OS.
        ///   On macOS this corresponds to Display P3, and on Linux it's sRGB.
        ///
        /// * `linear` - Perform alpha blending in linear space. This will eliminate
        ///   the darkening artifacts around the edges of text that are very visible
        ///   when certain color combinations are used (e.g. red / green), but makes
        ///   dark text look much thinner than normal and light text much thicker.
        ///   This is also sometimes known as "gamma correction".
        ///
        /// * `linear-corrected` - Same as `linear`, but with a correction step applied
        ///   for text that makes it look nearly or completely identical to `native`,
        ///   but without any of the darkening artifacts.
        ///
        /// Available since: 1.1.0
        "alpha-blending" => alpha_blending: AlphaBlending = AlphaBlending::LinearCorrected,

        /// All of the configurations behavior adjust various metrics determined by the
        /// font. The values can be integers (1, -1, etc.) or a percentage (20%, -15%,
        /// etc.). In each case, the values represent the amount to change the original
        /// value.
        ///
        /// For example, a value of `1` increases the value by 1; it does not set it to
        /// literally 1. A value of `20%` increases the value by 20%. And so on.
        ///
        /// There is little to no validation on these values so the wrong values (e.g.
        /// `-100%`) can cause the terminal to be unusable. Use with caution and reason.
        ///
        /// Some values are clamped to minimum or maximum values. This can make it
        /// appear that certain values are ignored. For example, many `*-thickness`
        /// adjustments cannot go below 1px.
        ///
        /// `adjust-cell-height` has some additional behaviors to describe:
        ///
        ///   * The font will be centered vertically in the cell.
        ///
        ///   * The cursor will remain the same size as the font, but may be
        ///     adjusted separately with `adjust-cursor-height`.
        ///
        ///   * Powerline glyphs will be adjusted along with the cell height so
        ///     that things like status lines continue to look aligned.
        "adjust-cell-width" => adjust_cell_width: Option<MetricModifier> = None,

        "adjust-cell-height" => adjust_cell_height: Option<MetricModifier> = None,

        /// Distance in pixels or percentage adjustment from the bottom of the cell to the text baseline.
        /// Increase to move baseline UP, decrease to move baseline DOWN.
        /// See the notes about adjustments in `adjust-cell-width`.
        "adjust-font-baseline" => adjust_font_baseline: Option<MetricModifier> = None,

        /// Distance in pixels or percentage adjustment from the top of the cell to the top of the underline.
        /// Increase to move underline DOWN, decrease to move underline UP.
        /// See the notes about adjustments in `adjust-cell-width`.
        "adjust-underline-position" => adjust_underline_position: Option<MetricModifier> = None,

        /// Thickness in pixels of the underline.
        /// See the notes about adjustments in `adjust-cell-width`.
        "adjust-underline-thickness" => adjust_underline_thickness: Option<MetricModifier> = None,

        /// Distance in pixels or percentage adjustment from the top of the cell to the top of the strikethrough.
        /// Increase to move strikethrough DOWN, decrease to move strikethrough UP.
        /// See the notes about adjustments in `adjust-cell-width`.
        "adjust-strikethrough-position" => adjust_strikethrough_position: Option<MetricModifier> = None,

        /// Thickness in pixels or percentage adjustment of the strikethrough.
        /// See the notes about adjustments in `adjust-cell-width`.
        "adjust-strikethrough-thickness" => adjust_strikethrough_thickness: Option<MetricModifier> = None,

        /// Distance in pixels or percentage adjustment from the top of the cell to the top of the overline.
        /// Increase to move overline DOWN, decrease to move overline UP.
        /// See the notes about adjustments in `adjust-cell-width`.
        "adjust-overline-position" => adjust_overline_position: Option<MetricModifier> = None,

        /// Thickness in pixels or percentage adjustment of the overline.
        /// See the notes about adjustments in `adjust-cell-width`.
        "adjust-overline-thickness" => adjust_overline_thickness: Option<MetricModifier> = None,

        /// Thickness in pixels or percentage adjustment of the bar cursor and outlined rect cursor.
        /// See the notes about adjustments in `adjust-cell-width`.
        "adjust-cursor-thickness" => adjust_cursor_thickness: Option<MetricModifier> = None,

        /// Height in pixels or percentage adjustment of the cursor. Currently applies to all cursor types:
        /// bar, rect, and outlined rect.
        /// See the notes about adjustments in `adjust-cell-width`.
        "adjust-cursor-height" => adjust_cursor_height: Option<MetricModifier> = None,

        /// Thickness in pixels or percentage adjustment of box drawing characters.
        /// See the notes about adjustments in `adjust-cell-width`.
        "adjust-box-thickness" => adjust_box_thickness: Option<MetricModifier> = None,

        /// Height in pixels or percentage adjustment of maximum height for nerd font icons.
        ///
        /// A positive (negative) value will increase (decrease) the maximum icon
        /// height. This may not affect all icons equally: the effect depends on whether
        /// the default size of the icon is height-constrained, which in turn depends on
        /// the aspect ratio of both the icon and your primary font.
        ///
        /// Certain icons designed for box drawing and terminal graphics, such as
        /// Powerline symbols, are not affected by this option.
        ///
        /// See the notes about adjustments in `adjust-cell-width`.
        ///
        /// Available in: 1.2.0
        "adjust-icon-height" => adjust_icon_height: Option<MetricModifier> = None,

        /// The method to use for calculating the cell width of a grapheme cluster.
        /// The default value is `unicode` which uses the Unicode standard to determine
        /// grapheme width. This results in correct grapheme width but may result in
        /// cursor-desync issues with some programs (such as shells) that may use a
        /// legacy method such as `wcswidth`.
        ///
        /// Valid values are:
        ///
        /// * `legacy` - Use a legacy method to determine grapheme width, such as
        ///   wcswidth This maximizes compatibility with legacy programs but may result
        ///   in incorrect grapheme width for certain graphemes such as skin-tone
        ///   emoji, non-English characters, etc.
        ///
        ///   This is called "legacy" and not something more specific because the
        ///   behavior is undefined and we want to retain the ability to modify it.
        ///   For example, we may or may not use libc `wcswidth` now or in the future.
        ///
        /// * `unicode` - Use the Unicode standard to determine grapheme width.
        ///
        /// If a running program explicitly enables terminal mode 2027, then `unicode`
        /// width will be forced regardless of this configuration. When mode 2027 is
        /// reset, this configuration will be used again.
        ///
        /// This configuration can be changed at runtime but will not affect existing
        /// terminals. Only new terminals will use the new configuration.
        "grapheme-width-method" => grapheme_width_method: GraphemeWidthMethod = GraphemeWidthMethod::Unicode,

        /// FreeType load flags to enable. The format of this is a list of flags to
        /// enable separated by commas. If you prefix a flag with `no-` then it is
        /// disabled. If you omit a flag, its default value is used, so you must
        /// explicitly disable flags you don't want. You can also use `true` or `false`
        /// to turn all flags on or off.
        ///
        /// This configuration only applies to Ghostty builds that use FreeType.
        /// This is usually the case only for Linux builds. macOS uses CoreText
        /// and does not have an equivalent configuration.
        ///
        /// Available flags:
        ///
        ///   * `hinting` - Enable or disable hinting. Enabled by default.
        ///
        ///   * `force-autohint` - Always use the freetype auto-hinter instead of
        ///     the font's native hinter. Disabled by default.
        ///
        ///   * `monochrome` - Instructs renderer to use 1-bit monochrome rendering.
        ///     This will disable anti-aliasing, and probably not look very good unless
        ///     you're using a pixel font. Disabled by default.
        ///
        ///   * `autohint` - Enable the freetype auto-hinter. Enabled by default.
        ///
        ///   * `light` - Use a light hinting style, better preserving glyph shapes.
        ///     This is the most common setting in GTK apps and therefore also Ghostty's
        ///     default. This has no effect if `monochrome` is enabled. Enabled by
        ///     default.
        ///
        /// Example: `hinting`, `no-hinting`, `force-autohint`, `no-force-autohint`
        "freetype-load-flags" => freetype_load_flags: FreetypeLoadFlags = FreetypeLoadFlags::default(),

        /// A theme to use. This can be a built-in theme name, a custom theme
        /// name, or an absolute path to a custom theme file. Ghostty also supports
        /// specifying a different theme to use for light and dark mode. Each
        /// option is documented below.
        ///
        /// If the theme is an absolute pathname, Ghostty will attempt to load that
        /// file as a theme. If that file does not exist or is inaccessible, an error
        /// will be logged and no other directories will be searched.
        ///
        /// If the theme is not an absolute pathname, two different directories will be
        /// searched for a file name that matches the theme. This is case sensitive on
        /// systems with case-sensitive filesystems. It is an error for a theme name to
        /// include path separators unless it is an absolute pathname.
        ///
        /// The first directory is the `themes` subdirectory of your Ghostty
        /// configuration directory. This is `$XDG_CONFIG_HOME/ghostty/themes` or
        /// `~/.config/ghostty/themes`.
        ///
        /// The second directory is the `themes` subdirectory of the Ghostty resources
        /// directory. Ghostty ships with a multitude of themes that will be installed
        /// into this directory. On macOS, this list is in the
        /// `Ghostty.app/Contents/Resources/ghostty/themes` directory. On Linux, this
        /// list is in the `share/ghostty/themes` directory (wherever you installed the
        /// Ghostty "share" directory.
        ///
        /// To see a list of available themes, run `ghostty +list-themes`.
        ///
        /// A theme file is simply another Ghostty configuration file. They share
        /// the same syntax and same configuration options. A theme can set any valid
        /// configuration option so please do not use a theme file from an untrusted
        /// source. The built-in themes are audited to only set safe configuration
        /// options.
        ///
        /// Some options cannot be set within theme files. The reason these are not
        /// supported should be self-evident. A theme file cannot set `theme` or
        /// `config-file`. At the time of writing this, Ghostty will not show any
        /// warnings or errors if you set these options in a theme file but they will
        /// be silently ignored.
        ///
        /// Any additional colors specified via background, foreground, palette, etc.
        /// will override the colors specified in the theme.
        ///
        /// To specify a different theme for light and dark mode, use the following
        /// syntax: `light:theme-name,dark:theme-name`. For example:
        /// `light:Rose Pine Dawn,dark:Rose Pine`. Whitespace around all values are
        /// trimmed and order of light and dark does not matter. Both light and dark
        /// must be specified in this form. In this form, the theme used will be
        /// based on the current desktop environment theme.
        ///
        /// There are some known bugs with light/dark mode theming. These will
        /// be fixed in a future update:
        ///
        ///   - macOS: titlebar tabs style is not updated when switching themes.
        "theme" => theme: Option<Theme> = None,

        /// Background color for the window.
        /// Specified as either hex (`#RRGGBB` or `RRGGBB`) or a named X11 color.
        "background" => background: Color = Color { r: 0x28, g: 0x2C, b: 0x34 },

        /// Foreground color for the window.
        /// Specified as either hex (`#RRGGBB` or `RRGGBB`) or a named X11 color.
        "foreground" => foreground: Color = Color { r: 0xFF, g: 0xFF, b: 0xFF },

        /// Background image for the terminal.
        ///
        /// This should be a path to a PNG or JPEG file, other image formats are
        /// not yet supported.
        ///
        /// The background image is currently per-terminal, not per-window. If
        /// you are a heavy split user, the background image will be repeated across
        /// splits. A future improvement to Ghostty will address this.
        ///
        /// WARNING: Background images are currently duplicated in VRAM per-terminal.
        /// For sufficiently large images, this could lead to a large increase in
        /// memory usage (specifically VRAM usage). A future Ghostty improvement
        /// will resolve this by sharing image textures across terminals.
        ///
        /// Available since: 1.2.0
        "background-image" => background_image: Option<Path> = None,

        /// Background image opacity.
        ///
        /// This is relative to the value of `background-opacity`.
        ///
        /// A value of `1.0` (the default) will result in the background image being
        /// placed on top of the general background color, and then the combined result
        /// will be adjusted to the opacity specified by `background-opacity`.
        ///
        /// A value less than `1.0` will result in the background image being mixed
        /// with the general background color before the combined result is adjusted
        /// to the configured `background-opacity`.
        ///
        /// A value greater than `1.0` will result in the background image having a
        /// higher opacity than the general background color. For instance, if the
        /// configured `background-opacity` is `0.5` and `background-image-opacity`
        /// is set to `1.5`, then the final opacity of the background image will be
        /// `0.5 * 1.5 = 0.75`.
        ///
        /// Available since: 1.2.0
        "background-image-opacity" => background_image_opacity: f32 = 1.0,

        /// Background image position.
        ///
        /// Valid values are:
        ///   * `top-left`
        ///   * `top-center`
        ///   * `top-right`
        ///   * `center-left`
        ///   * `center`
        ///   * `center-right`
        ///   * `bottom-left`
        ///   * `bottom-center`
        ///   * `bottom-right`
        ///
        /// The default value is `center`.
        ///
        /// Available since: 1.2.0
        "background-image-position" => background_image_position: BackgroundImagePosition = BackgroundImagePosition::Center,

        /// Background image fit.
        ///
        /// Valid values are:
        ///
        ///  * `contain`
        ///
        ///    Preserving the aspect ratio, scale the background image to the largest
        ///    size that can still be contained within the terminal, so that the whole
        ///    image is visible.
        ///
        ///  * `cover`
        ///
        ///    Preserving the aspect ratio, scale the background image to the smallest
        ///    size that can completely cover the terminal. This may result in one or
        ///    more edges of the image being clipped by the edge of the terminal.
        ///
        ///  * `stretch`
        ///
        ///    Stretch the background image to the full size of the terminal, without
        ///    preserving the aspect ratio.
        ///
        ///  * `none`
        ///
        ///    Don't scale the background image.
        ///
        /// The default value is `contain`.
        ///
        /// Available since: 1.2.0
        "background-image-fit" => background_image_fit: BackgroundImageFit = BackgroundImageFit::Contain,

        /// Whether to repeat the background image or not.
        ///
        /// If this is set to true, the background image will be repeated if there
        /// would otherwise be blank space around it because it doesn't completely
        /// fill the terminal area.
        ///
        /// The default value is `false`.
        ///
        /// Available since: 1.2.0
        "background-image-repeat" => background_image_repeat: bool = false,

        /// The foreground and background color for selection. If this is not set, then
        /// the selection color is just the inverted window background and foreground
        /// (note: not to be confused with the cell bg/fg).
        /// Specified as either hex (`#RRGGBB` or `RRGGBB`) or a named X11 color.
        /// Since version 1.2.0, this can also be set to `cell-foreground` to match
        /// the cell foreground color, or `cell-background` to match the cell
        /// background color.
        "selection-foreground" => selection_foreground: Option<TerminalColor> = None,

        "selection-background" => selection_background: Option<TerminalColor> = None,

        /// Whether to clear selected text when typing. This defaults to `true`.
        /// This is typical behavior for most terminal emulators as well as
        /// text input fields. If you set this to `false`, then the selected text
        /// will not be cleared when typing.
        ///
        /// "Typing" is specifically defined as any non-modifier (shift, control,
        /// alt, etc.) keypress that produces data to be sent to the application
        /// running within the terminal (e.g. the shell). Additionally, selection
        /// is cleared when any preedit or composition state is started (e.g.
        /// when typing languages such as Japanese).
        ///
        /// If this is `false`, then the selection can still be manually
        /// cleared by clicking once or by pressing `escape`.
        ///
        /// Available since: 1.2.0
        "selection-clear-on-typing" => selection_clear_on_typing: bool = true,

        /// Whether to clear selected text after copying. This defaults to `false`.
        ///
        /// When set to `true`, the selection will be automatically cleared after
        /// any copy operation that invokes the `copy_to_clipboard` keyboard binding.
        /// Importantly, this will not clear the selection if the copy operation
        /// was invoked via `copy-on-select`.
        ///
        /// When set to `false`, the selection remains visible after copying, allowing
        /// to see what was copied and potentially perform additional operations
        /// on the same selection.
        "selection-clear-on-copy" => selection_clear_on_copy: bool = false,

        /// Characters that mark word boundaries during text selection operations such
        /// as double-clicking. When selecting a word, the selection will stop at any
        /// of these characters.
        ///
        /// This is similar to the `WORDCHARS` environment variable in zsh, except this
        /// specifies the boundary characters rather than the word characters. The
        /// default includes common delimiters and punctuation that typically separate
        /// words in code and prose.
        ///
        /// Each character in this string becomes a word boundary. Multi-byte UTF-8
        /// characters are supported, but only single codepoints can be specified.
        /// Multi-codepoint sequences (e.g. emoji) are not supported.
        ///
        /// The null character (U+0000) is always treated as a boundary and does not
        /// need to be included in this configuration.
        ///
        /// Default: `` \t'"│`|:;,()[]{}<>$ ``
        ///
        /// To add or remove specific characters, you can set this to a custom value.
        /// For example, to treat semicolons as part of words:
        ///
        ///     selection-word-chars = " \t'\"│`|:,()[]{}<>$"
        ///
        /// Available since: 1.3.0
        "selection-word-chars" => selection_word_chars: SelectionWordChars = SelectionWordChars::default(),

        /// The minimum contrast ratio between the foreground and background colors.
        /// The contrast ratio is a value between 1 and 21. A value of 1 allows for no
        /// contrast (e.g. black on black). This value is the contrast ratio as defined
        /// by the [WCAG 2.0 specification](https://www.w3.org/TR/WCAG20/).
        ///
        /// If you want to avoid invisible text (same color as background), a value of
        /// 1.1 is a good value. If you want to avoid text that is difficult to read, a
        /// value of 3 or higher is a good value. The higher the value, the more likely
        /// that text will become black or white.
        ///
        /// This value does not apply to Emoji or images.
        "minimum-contrast" => minimum_contrast: f64 = 1.0,

        /// Color palette for the 256 color form that many terminal applications use.
        /// The syntax of this configuration is `N=COLOR` where `N` is 0 to 255 (for
        /// the 256 colors in the terminal color table) and `COLOR` is a typical RGB
        /// color code such as `#AABBCC` or `AABBCC`, or a named X11 color. For example,
        /// `palette = 5=#BB78D9` will set the 'purple' color.
        ///
        /// The palette index can be in decimal, binary, octal, or hexadecimal.
        /// Decimal is assumed unless a prefix is used: `0b` for binary, `0o` for octal,
        /// and `0x` for hexadecimal.
        ///
        /// For definitions on the color indices and what they canonically map to,
        /// [see this cheat sheet](https://www.ditig.com/256-colors-cheat-sheet).
        ///
        /// For most themes, you only need to set the first 16 colors (0–15) since the
        /// rest of the palette (16–255) will be automatically generated by
        /// default (see `palette-generate` for more details).
        "palette" => palette: Palette = Palette::default(),

        /// Whether to automatically generate the extended 256 color palette
        /// (indices 16–255) from the base 16 ANSI colors.
        ///
        /// This lets theme authors specify only the base 16 colors and have the
        /// rest of the palette be automatically generated in a consistent and
        /// aesthetic way.
        ///
        /// When enabled, the 6×6×6 color cube and 24-step grayscale ramp are
        /// derived from interpolations of the base palette, giving a more cohesive
        /// look. Colors that have been explicitly set via `palette` are never
        /// overwritten.
        ///
        /// The default value is false (disabled), because many legacy programs
        /// using the 256-color palette hardcode assumptions about what these
        /// colors are (mostly assuming the xterm 256 color palette). However, this
        /// is still a very useful tool for theme authors and users who want
        /// to customize their palette without having to specify all 256 colors.
        ///
        /// For more information on how the generation works, see here:
        /// https://gist.github.com/jake-stewart/0a8ea46159a7da2c808e5be2177e1783
        ///
        /// Available since: 1.3.0
        "palette-generate" => palette_generate: bool = false,

        /// Invert the palette colors generated when `palette-generate` is enabled,
        /// so that the colors go in reverse order. This allows palette-based
        /// applications to work well in both light and dark mode since the
        /// palettes are always relatively good colors.
        ///
        /// This defaults to off because some legacy terminal applications
        /// hardcode the assumption that palette indices 16–231 are ordered from
        /// darkest to lightest, so enabling this would make them unreadable.
        /// This is not a generally good assumption and we encourage modern
        /// terminal applications to use the indices in a more semantic way.
        ///
        /// This has no effect if `palette-generate` is disabled.
        ///
        /// For more information see `palette-generate`.
        ///
        /// Available since: 1.3.0
        "palette-harmonious" => palette_harmonious: bool = false,

        /// The color of the cursor. If this is not set, a default will be chosen.
        ///
        /// Direct colors can be specified as either hex (`#RRGGBB` or `RRGGBB`)
        /// or a named X11 color.
        ///
        /// Additionally, special values can be used to set the color to match
        /// other colors at runtime:
        ///
        ///   * `cell-foreground` - Match the cell foreground color.
        ///     (Available since: 1.2.0)
        ///
        ///   * `cell-background` - Match the cell background color.
        ///     (Available since: 1.2.0)
        "cursor-color" => cursor_color: Option<TerminalColor> = None,

        /// The opacity level (opposite of transparency) of the cursor. A value of 1
        /// is fully opaque and a value of 0 is fully transparent. A value less than 0
        /// or greater than 1 will be clamped to the nearest valid value. Note that a
        /// sufficiently small value such as 0.3 may be effectively invisible and may
        /// make it difficult to find the cursor.
        "cursor-opacity" => cursor_opacity: f64 = 1.0,

        /// The style of the cursor. This sets the default style. A running program can
        /// still request an explicit cursor style using escape sequences (such as `CSI
        /// q`). Shell configurations will often request specific cursor styles.
        ///
        /// Note that shell integration will automatically set the cursor to a bar at
        /// a prompt, regardless of this configuration. You can disable that behavior
        /// by specifying `shell-integration-features = no-cursor` or disabling shell
        /// integration entirely.
        ///
        /// Valid values are:
        ///
        ///   * `block`
        ///   * `bar`
        ///   * `underline`
        ///   * `block_hollow`
        "cursor-style" => cursor_style: CursorStyle = CursorStyle::Block,

        /// Sets the default blinking state of the cursor. This is just the default
        /// state; running programs may override the cursor style using `DECSCUSR` (`CSI
        /// q`).
        ///
        /// If this is not set, the cursor blinks by default. Note that this is not the
        /// same as a "true" value, as noted below.
        ///
        /// If this is not set at all (`null`), then Ghostty will respect DEC Mode 12
        /// (AT&T cursor blink) as an alternate approach to turning blinking on/off. If
        /// this is set to any value other than null, DEC mode 12 will be ignored but
        /// `DECSCUSR` will still be respected.
        ///
        /// Valid values are:
        ///
        ///   * ` ` (blank)
        ///   * `true`
        ///   * `false`
        "cursor-style-blink" => cursor_style_blink: Option<bool> = None,

        /// The color of the text under the cursor. If this is not set, a default will
        /// be chosen.
        /// Specified as either hex (`#RRGGBB` or `RRGGBB`) or a named X11 color.
        /// Since version 1.2.0, this can also be set to `cell-foreground` to match
        /// the cell foreground color, or `cell-background` to match the cell
        /// background color.
        "cursor-text" => cursor_text: Option<TerminalColor> = None,

        /// Enables the ability to move the cursor at prompts by clicking on a
        /// location in the prompt text.
        ///
        /// This feature requires shell integration, specifically prompt marking
        /// via `OSC 133`. Some shells like Fish (v4) and Nu (0.111+) natively
        /// support this while others may require additional configuration or
        /// Ghostty's shell integration features to be enabled.
        ///
        /// Depending on the shell, this works either by translating your click
        /// position into a series of synthetic arrow key movements or by sending
        /// a click event directly to the shell. In either case, some unexpected
        /// behavior around edge cases is possible.
        "cursor-click-to-move" => cursor_click_to_move: bool = true,

        /// Hide the mouse immediately when typing. The mouse becomes visible again
        /// when the mouse is used (button, movement, etc.). Platform-specific behavior
        /// may dictate other scenarios where the mouse is shown. For example on macOS,
        /// the mouse is shown again when a new window, tab, or split is created.
        "mouse-hide-while-typing" => mouse_hide_while_typing: bool = false,

        /// When to scroll the surface to the bottom. The format of this is a list of
        /// options to enable separated by commas. If you prefix an option with `no-`
        /// then it is disabled. If you omit an option, its default value is used.
        ///
        /// Available options:
        ///
        /// - `keystroke` If set, scroll the surface to the bottom when the user
        ///   presses a key that results in data being sent to the PTY (basically
        ///   anything but modifiers or keybinds that are processed by Ghostty).
        ///
        /// - `output` If set, scroll the surface to the bottom if there is new data
        ///   to display (e.g., when new lines are printed to the terminal).
        ///
        /// The default is `keystroke, no-output`.
        "scroll-to-bottom" => scroll_to_bottom: ScrollToBottom = ScrollToBottom::default(),

        /// Determines whether running programs can detect the shift key pressed with a
        /// mouse click. Typically, the shift key is used to extend mouse selection.
        ///
        /// The default value of `false` means that the shift key is not sent with
        /// the mouse protocol and will extend the selection. This value can be
        /// conditionally overridden by the running program with the `XTSHIFTESCAPE`
        /// sequence.
        ///
        /// The value `true` means that the shift key is sent with the mouse protocol
        /// but the running program can override this behavior with `XTSHIFTESCAPE`.
        ///
        /// The value `never` is the same as `false` but the running program cannot
        /// override this behavior with `XTSHIFTESCAPE`. The value `always` is the
        /// same as `true` but the running program cannot override this behavior with
        /// `XTSHIFTESCAPE`.
        ///
        /// If you always want shift to extend mouse selection even if the program
        /// requests otherwise, set this to `never`.
        ///
        /// Valid values are:
        ///
        ///   * `true`
        ///   * `false`
        ///   * `always`
        ///   * `never`
        "mouse-shift-capture" => mouse_shift_capture: MouseShiftCapture = MouseShiftCapture::False,

        /// Enable or disable mouse reporting. When set to `false`, mouse events will
        /// not be reported to terminal applications even if they request it. This
        /// allows you to always use the mouse for selection and other terminal UI
        /// interactions without applications capturing mouse input.
        ///
        /// When set to `true` (the default), terminal applications can request mouse
        /// reporting and will receive mouse events according to their requested mode.
        ///
        /// This can be toggled at runtime using the `toggle_mouse_reporting` keybind
        /// action.
        "mouse-reporting" => mouse_reporting: bool = true,

        /// Multiplier for scrolling distance with the mouse wheel.
        ///
        /// A prefix of `precision:` or `discrete:` can be used to set the multiplier
        /// only for scrolling with the specific type of devices. These can be
        /// comma-separated to set both types of multipliers at the same time, e.g.
        /// `precision:0.1,discrete:3`. If no prefix is used, the multiplier applies
        /// to all scrolling devices. Specifying a prefix was introduced in Ghostty
        /// 1.2.1.
        ///
        /// The value will be clamped to [0.01, 10,000]. Both of these are extreme
        /// and you're likely to have a bad experience if you set either extreme.
        ///
        /// The default value is "3" for discrete devices and "1" for precision devices.
        "mouse-scroll-multiplier" => mouse_scroll_multiplier: MouseScrollMultiplier = MouseScrollMultiplier::default(),

        /// The opacity level (opposite of transparency) of the background. A value of
        /// 1 is fully opaque and a value of 0 is fully transparent. A value less than 0
        /// or greater than 1 will be clamped to the nearest valid value.
        ///
        /// On macOS, background opacity is disabled when the terminal enters native
        /// fullscreen. This is because the background becomes gray and it can cause
        /// widgets to show through which isn't generally desirable.
        ///
        /// On macOS, changing this configuration requires restarting Ghostty completely.
        "background-opacity" => background_opacity: f64 = 1.0,

        /// Applies background opacity to cells with an explicit background color
        /// set.
        ///
        /// Normally, `background-opacity` is only applied to the window background.
        /// If a cell has an explicit background color set, such as red, then that
        /// background color will be fully opaque. An effect of this is that some
        /// terminal applications that repaint the background color of the terminal
        /// such as a Neovim and Tmux may not respect the `background-opacity`
        /// (by design).
        ///
        /// Setting this to `true` will apply the `background-opacity` to all cells
        /// regardless of whether they have an explicit background color set or not.
        ///
        /// Available since: 1.2.0
        "background-opacity-cells" => background_opacity_cells: bool = false,

        /// Whether to blur the background when `background-opacity` is less than 1.
        ///
        /// Valid values are:
        ///
        ///   * a nonnegative integer specifying the *blur intensity*
        ///   * `false`, equivalent to a blur intensity of 0
        ///   * `true`, equivalent to the default blur intensity of 20, which is
        ///     reasonable for a good looking blur. Higher blur intensities may
        ///     cause strange rendering and performance issues.
        ///
        /// On macOS 26.0 and later, there are additional special values that
        /// can be set to use the native macOS glass effects:
        ///
        ///   * `macos-glass-regular` - Standard glass effect with some opacity
        ///   * `macos-glass-clear` - Highly transparent glass effect
        ///
        /// If the macOS values are set, then this implies `background-blur = true`
        /// on non-macOS platforms.
        ///
        /// Supported on macOS and on some Linux desktop environments, including:
        ///
        ///   * KDE Plasma (Wayland and X11)
        ///
        /// Warning: the exact blur intensity is _ignored_ under KDE Plasma, and setting
        /// this setting to either `true` or any positive blur intensity value would
        /// achieve the same effect. The reason is that KWin, the window compositor
        /// powering Plasma, only has one global blur setting and does not allow
        /// applications to specify individual blur settings.
        ///
        /// To configure KWin's global blur setting, open System Settings and go to
        /// "Apps & Windows" > "Window Management" > "Desktop Effects" and select the
        /// "Blur" plugin. If disabled, enable it by ticking the checkbox to the left.
        /// Then click on the "Configure" button and there will be two sliders that
        /// allow you to set background blur and noise intensities for all apps,
        /// including Ghostty.
        ///
        /// All other Linux desktop environments are as of now unsupported. Users may
        /// need to set environment-specific settings and/or install third-party plugins
        /// in order to support background blur, as there isn't a unified interface for
        /// doing so.
        "background-blur" => background_blur: BackgroundBlur = BackgroundBlur::False,

        /// The opacity level (opposite of transparency) of an unfocused split.
        /// Unfocused splits by default are slightly faded out to make it easier to see
        /// which split is focused. To disable this feature, set this value to 1.
        ///
        /// A value of 1 is fully opaque and a value of 0 is fully transparent. Because
        /// "0" is not useful (it makes the window look very weird), the minimum value
        /// is 0.15. This value still looks weird but you can at least see what's going
        /// on. A value outside of the range 0.15 to 1 will be clamped to the nearest
        /// valid value.
        "unfocused-split-opacity" => unfocused_split_opacity: f64 = 0.7,

        /// The color to dim the unfocused split. Unfocused splits are dimmed by
        /// rendering a semi-transparent rectangle over the split. This sets the color of
        /// that rectangle and can be used to carefully control the dimming effect.
        ///
        /// This will default to the background color.
        ///
        /// Specified as either hex (`#RRGGBB` or `RRGGBB`) or a named X11 color.
        "unfocused-split-fill" => unfocused_split_fill: Option<Color> = None,

        /// The color of the split divider. If this is not set, a default will be chosen.
        /// Specified as either hex (`#RRGGBB` or `RRGGBB`) or a named X11 color.
        ///
        /// Available since: 1.1.0
        "split-divider-color" => split_divider_color: Option<Color> = None,

        /// Control when Ghostty preserves a zoomed split. Under normal circumstances,
        /// any operation that changes focus or layout of the split tree in a window
        /// will unzoom any zoomed split. This configuration allows you to control
        /// this behavior.
        ///
        /// This can be set to `navigation` to preserve the zoomed split state
        /// when navigating to another split (e.g. via `goto_split`). This will
        /// change the zoomed split to the newly focused split instead of unzooming.
        ///
        /// Any options can also be prefixed with `no-` to disable that option.
        ///
        /// Example: `split-preserve-zoom = navigation`
        ///
        /// Available since: 1.3.0
        "split-preserve-zoom" => split_preserve_zoom: SplitPreserveZoom = SplitPreserveZoom::default(),

        /// The foreground and background color for search matches. This only applies
        /// to non-focused search matches, also known as candidate matches.
        ///
        /// Valid values:
        ///
        ///   - Hex (`#RRGGBB` or `RRGGBB`)
        ///   - Named X11 color
        ///   - "cell-foreground" to match the cell foreground color
        ///   - "cell-background" to match the cell background color
        ///
        /// The default value is black text on a golden yellow background.
        "search-foreground" => search_foreground: TerminalColor = TerminalColor::Color(Color { r: 0, g: 0, b: 0 }),

        "search-background" => search_background: TerminalColor = TerminalColor::Color(Color { r: 0xFF, g: 0xE0, b: 0x82 }),

        /// The foreground and background color for the currently selected search match.
        /// This is the focused match that will be jumped to when using next/previous
        /// search navigation.
        ///
        /// Valid values:
        ///
        ///   - Hex (`#RRGGBB` or `RRGGBB`)
        ///   - Named X11 color
        ///   - "cell-foreground" to match the cell foreground color
        ///   - "cell-background" to match the cell background color
        ///
        /// The default value is black text on a soft peach background.
        "search-selected-foreground" => search_selected_foreground: TerminalColor = TerminalColor::Color(Color { r: 0, g: 0, b: 0 }),

        "search-selected-background" => search_selected_background: TerminalColor = TerminalColor::Color(Color { r: 0xF2, g: 0xA5, b: 0x7E }),

        /// The command to run, usually a shell. If this is not an absolute path, it'll
        /// be looked up in the `PATH`. If this is not set, a default will be looked up
        /// from your system. The rules for the default lookup are:
        ///
        ///   * `SHELL` environment variable
        ///
        ///   * `passwd` entry (user information)
        ///
        /// This can contain additional arguments to run the command with. If additional
        /// arguments are provided, the command will be executed using `/bin/sh -c`
        /// to offload shell argument expansion.
        ///
        /// To avoid shell expansion altogether, prefix the command with `direct:`, e.g.
        /// `direct:nvim foo`. This will avoid the roundtrip to `/bin/sh` but will also
        /// not support any shell parsing such as arguments with spaces, filepaths with
        /// `~`, globs, etc. (Available since: 1.2.0)
        ///
        /// You can also explicitly prefix the command with `shell:` to always wrap the
        /// command in a shell. This can be used to ensure our heuristics to choose the
        /// right mode are not used in case they are wrong. (Available since: 1.2.0)
        ///
        /// This command will be used for all new terminal surfaces, i.e. new windows,
        /// tabs, etc. If you want to run a command only for the first terminal surface
        /// created when Ghostty starts, use the `initial-command` configuration.
        ///
        /// Ghostty supports the common `-e` flag for executing a command with
        /// arguments. For example, `ghostty -e fish --with --custom --args`.
        /// This flag sets the `initial-command` configuration, see that for more
        /// information.
        "command" => command: Option<Command> = None,

        /// This is the same as "command", but only applies to the first terminal
        /// surface created when Ghostty starts. Subsequent terminal surfaces will use
        /// the `command` configuration.
        ///
        /// After the first terminal surface is created (or closed), there is no
        /// way to run this initial command again automatically. As such, setting
        /// this at runtime works but will only affect the next terminal surface
        /// if it is the first one ever created.
        ///
        /// If you're using the `ghostty` CLI there is also a shortcut to set this
        /// with arguments directly: you can use the `-e` flag. For example: `ghostty -e
        /// fish --with --custom --args`. The `-e` flag automatically forces some
        /// other behaviors as well:
        ///
        ///   * Disables shell expansion since the input is expected to already
        ///     be shell-expanded by the upstream (e.g. the shell used to type in
        ///     the `ghostty -e` command).
        ///
        ///   * `gtk-single-instance=false` - This ensures that a new instance is
        ///     launched and the CLI args are respected.
        ///
        ///   * `quit-after-last-window-closed=true` - This ensures that the Ghostty
        ///     process will exit when the command exits. Additionally, the
        ///     `quit-after-last-window-closed-delay` is unset.
        ///
        ///   * `shell-integration=detect` (if not `none`) - This prevents forcibly
        ///     injecting any configured shell integration into the command's
        ///     environment. With `-e` its highly unlikely that you're executing a
        ///     shell and forced shell integration is likely to cause problems
        ///     (e.g. by wrapping your command in a shell, setting env vars, etc.).
        ///     This is a safety measure to prevent unexpected behavior. If you want
        ///     shell integration with a `-e`-executed command, you must either
        ///     name your binary appropriately or source the shell integration script
        ///     manually.
        "initial-command" => initial_command: Option<Command> = None,

        /// Controls when command finished notifications are sent. There are
        /// three options:
        ///
        /// * `never` - Never send notifications (the default).
        /// * `unfocused` - Only send notifications if the surface that the command is
        ///   running in is not focused.
        /// * `always` - Always send notifications.
        ///
        /// Command finished notifications requires that either shell integration is
        /// enabled, or that your shell sends OSC 133 escape sequences to mark the start
        /// and end of commands.
        ///
        /// On GTK, there is a context menu item that will enable command finished
        /// notifications for a single command, overriding the `never` and `unfocused`
        /// options.
        ///
        /// Available since 1.3.0.
        "notify-on-command-finish" => notify_on_command_finish: NotifyOnCommandFinish = NotifyOnCommandFinish::Never,

        /// If command finished notifications are enabled, this controls how the user is
        /// notified.
        ///
        /// Available options:
        ///
        /// * `bell` - enabled by default
        /// * `notify` - disabled by default
        ///
        /// Options can be combined by listing them as a comma separated list. Options
        /// can be negated by prefixing them with `no-`. For example `no-bell,notify`.
        ///
        /// Available since 1.3.0.
        "notify-on-command-finish-action" => notify_on_command_finish_action: NotifyOnCommandFinishAction = NotifyOnCommandFinishAction { bell: true, notify: false },

        /// If command finished notifications are enabled, this controls how long a
        /// command must have been running before a notification will be sent. The
        /// default is five seconds.
        ///
        /// The duration is specified as a series of numbers followed by time units.
        /// Whitespace is allowed between numbers and units. Each number and unit will
        /// be added together to form the total duration.
        ///
        /// The allowed time units are as follows:
        ///
        ///   * `y` - 365 SI days, or 8760 hours, or 31536000 seconds. No adjustments
        ///     are made for leap years or leap seconds.
        ///   * `d` - one SI day, or 86400 seconds.
        ///   * `h` - one hour, or 3600 seconds.
        ///   * `m` - one minute, or 60 seconds.
        ///   * `s` - one second.
        ///   * `ms` - one millisecond, or 0.001 second.
        ///   * `us` or `µs` - one microsecond, or 0.000001 second.
        ///   * `ns` - one nanosecond, or 0.000000001 second.
        ///
        /// Examples:
        ///   * `1h30m`
        ///   * `45s`
        ///
        /// Units can be repeated and will be added together. This means that
        /// `1h1h` is equivalent to `2h`. This is confusing and should be avoided.
        /// A future update may disallow this.
        ///
        /// The maximum value is `584y 49w 23h 34m 33s 709ms 551µs 615ns`. Any
        /// value larger than this will be clamped to the maximum value.
        ///
        /// Available since 1.3.0
        "notify-on-command-finish-after" => notify_on_command_finish_after: Duration = Duration { duration: 5 * NS_PER_S },

        /// Extra environment variables to pass to commands launched in a terminal
        /// surface. The format is `env=KEY=VALUE`.
        ///
        /// `env = foo=bar`
        /// `env = bar=baz`
        ///
        /// Setting `env` to an empty string will reset the entire map to default
        /// (empty).
        ///
        /// `env =`
        ///
        /// Setting a key to an empty string will remove that particular key and
        /// corresponding value from the map.
        ///
        /// `env = foo=bar`
        /// `env = foo=`
        ///
        /// will result in `foo` not being passed to the launched commands.
        ///
        /// Setting a key multiple times will overwrite previous entries.
        ///
        /// `env = foo=bar`
        /// `env = foo=baz`
        ///
        /// will result in `foo=baz` being passed to the launched commands.
        ///
        /// These environment variables will override any existing environment
        /// variables set by Ghostty. For example, if you set `GHOSTTY_RESOURCES_DIR`
        /// then the value you set here will override the value Ghostty typically
        /// automatically injects.
        ///
        /// These environment variables _will not_ be passed to commands run by Ghostty
        /// for other purposes, like `open` or `xdg-open` used to open URLs in your
        /// browser.
        ///
        /// Available since: 1.2.0
        "env" => env: RepeatableStringMap = RepeatableStringMap::default(),

        /// Data to send as input to the command on startup.
        ///
        /// The configured `command` will be launched using the typical rules,
        /// then the data specified as this input will be written to the pty
        /// before any other input can be provided.
        ///
        /// The bytes are sent as-is with no additional encoding. Therefore, be
        /// cautious about input that can contain control characters, because this
        /// can be used to execute programs in a shell.
        ///
        /// The format of this value is:
        ///
        ///   * `raw:<string>` - Send raw text as-is. This uses Zig string literal
        ///     syntax so you can specify control characters and other standard
        ///     escapes.
        ///
        ///   * `path:<path>` - Read a filepath and send the contents. The path
        ///     must be to a file with finite length. e.g. don't use a device
        ///     such as `/dev/stdin` or `/dev/urandom` as these will block
        ///     terminal startup indefinitely. Files are limited to 10MB
        ///     in size to prevent excessive memory usage. If you have files
        ///     larger than this you should write a script to read the file
        ///     and send it to the terminal.
        ///
        /// If no valid prefix is found, it is assumed to be a `raw:` input.
        /// This is an ergonomic choice to allow you to simply write
        /// `input = "Hello, world!"` (a common case) without needing to prefix
        /// every value with `raw:`.
        ///
        /// This can be repeated multiple times to send more data. The data
        /// is concatenated directly with no separator characters in between
        /// (e.g. no newline).
        ///
        /// If any of the input sources do not exist, then none of the input
        /// will be sent. Input sources are not verified until the terminal
        /// is starting, so missing paths will not show up in config validation.
        ///
        /// Changing this configuration at runtime will only affect new
        /// terminals.
        ///
        /// Available since: 1.2.0
        "input" => input: RepeatableReadableIO = RepeatableReadableIO::default(),

        /// If true, keep the terminal open after the command exits. Normally, the
        /// terminal window closes when the running command (such as a shell) exits.
        /// With this true, the terminal window will stay open until any keypress is
        /// received.
        ///
        /// This is primarily useful for scripts or debugging.
        "wait-after-command" => wait_after_command: bool = false,

        /// The number of milliseconds of runtime below which we consider a process exit
        /// to be abnormal. This is used to show an error message when the process exits
        /// too quickly.
        ///
        /// On Linux, this must be paired with a non-zero exit code. On macOS, we allow
        /// any exit code because of the way shell processes are launched via the login
        /// command.
        "abnormal-command-exit-runtime" => abnormal_command_exit_runtime: u32 = 250,

        /// The size of the scrollback buffer in bytes. This also includes the active
        /// screen. No matter what this is set to, enough memory will always be
        /// allocated for the visible screen and anything leftover is the limit for
        /// the scrollback.
        ///
        /// When this limit is reached, the oldest lines are removed from the
        /// scrollback.
        ///
        /// Scrollback currently exists completely in memory. This means that the
        /// larger this value, the larger potential memory usage. Scrollback is
        /// allocated lazily up to this limit, so if you set this to a very large
        /// value, it will not immediately consume a lot of memory.
        ///
        /// This size is per terminal surface, not for the entire application.
        ///
        /// It is not currently possible to set an unlimited scrollback buffer.
        /// This is a future planned feature.
        ///
        /// This can be changed at runtime but will only affect new terminal surfaces.
        "scrollback-limit" => scrollback_limit: usize = 10_000_000,

        /// Control when the scrollbar is shown to scroll the scrollback buffer.
        ///
        /// The default value is `system`.
        ///
        /// Valid values:
        ///
        ///   * `system` - Respect the system settings for when to show scrollbars.
        ///     For example, on macOS, this will respect the "Scrollbar behavior"
        ///     system setting which by default usually only shows scrollbars while
        ///     actively scrolling or hovering the gutter.
        ///
        ///   * `never` - Never show a scrollbar. You can still scroll using the mouse,
        ///     keybind actions, etc. but you will not have a visual UI widget showing
        ///     a scrollbar.
        "scrollbar" => scrollbar: Scrollbar = Scrollbar::System,

        /// Match a regular expression against the terminal text and associate clicking
        /// it with an action. This can be used to match URLs, file paths, etc. Actions
        /// can be opening using the system opener (e.g. `open` or `xdg-open`) or
        /// executing any arbitrary binding action.
        ///
        /// Links that are configured earlier take precedence over links that are
        /// configured later.
        ///
        /// A default link that matches a URL and opens it in the system opener always
        /// exists. This can be disabled using `link-url`.
        ///
        /// TODO: This can't currently be set!
        "link" => link: RepeatableLink = RepeatableLink::default(),

        /// Enable URL matching. URLs are matched on hover with control (Linux) or
        /// command (macOS) pressed and open using the default system application for
        /// the linked URL.
        ///
        /// The URL matcher is always lowest priority of any configured links (see
        /// `link`). If you want to customize URL matching, use `link` and disable this.
        "link-url" => link_url: bool = true,

        /// Show link previews for a matched URL.
        ///
        /// When true, link previews are shown for all matched URLs. When false, link
        /// previews are never shown. When set to "osc8", link previews are only shown
        /// for hyperlinks created with the OSC 8 sequence (in this case, the link text
        /// can differ from the link destination).
        ///
        /// Available since: 1.2.0
        "link-previews" => link_previews: LinkPreviews = LinkPreviews::True,

        /// Whether to start the window in a maximized state. This setting applies
        /// to new windows and does not apply to tabs, splits, etc. However, this setting
        /// will apply to all new windows, not just the first one.
        ///
        /// Available since: 1.1.0
        "maximize" => maximize: bool = false,

        /// Start new windows in fullscreen. This setting applies to new windows and
        /// does not apply to tabs, splits, etc. However, this setting will apply to all
        /// new windows, not just the first one.
        ///
        /// Allowable values are:
        ///
        ///   * `false` - Don't start in fullscreen (default)
        ///   * `true` - Start in native fullscreen
        ///   * `non-native` - (macOS only) Start in non-native fullscreen, hiding the
        ///     menu bar. This is faster than native fullscreen since it doesn't use
        ///     animations. On non-macOS platforms, this behaves the same as `true`.
        ///   * `non-native-visible-menu` - (macOS only) Start in non-native fullscreen,
        ///     keeping the menu bar visible. On non-macOS platforms, behaves like `true`.
        ///   * `non-native-padded-notch` - (macOS only) Start in non-native fullscreen,
        ///     hiding the menu bar but padding for the notch on applicable devices.
        ///     On non-macOS platforms, behaves like `true`.
        ///
        /// Important: tabs DO NOT WORK with non-native fullscreen modes. Non-native
        /// fullscreen removes the titlebar and macOS native tabs require the titlebar.
        /// If you use tabs, use `true` (native) instead.
        ///
        /// On macOS, `true` (native fullscreen) does not work if `window-decoration`
        /// is set to `false`, because native fullscreen on macOS requires window
        /// decorations.
        "fullscreen" => fullscreen: Fullscreen = Fullscreen::False,

        /// The title Ghostty will use for the window. This will force the title of the
        /// window to be this title at all times and Ghostty will ignore any set title
        /// escape sequences programs (such as Neovim) may send.
        ///
        /// If you want a blank title, set this to one or more spaces by quoting
        /// the value. For example, `title = " "`. This effectively hides the title.
        /// This is necessary because setting a blank value resets the title to the
        /// default value of the running program.
        ///
        /// This configuration can be reloaded at runtime. If it is set, the title
        /// will update for all windows. If it is unset, the next title change escape
        /// sequence will be honored but previous changes will not retroactively
        /// be set. This latter case may require you to restart programs such as Neovim
        /// to get the new title.
        "title" => title: Option<String> = None,

        /// The setting that will change the application class value.
        ///
        /// This controls the class field of the `WM_CLASS` X11 property (when running
        /// under X11), the Wayland application ID (when running under Wayland), and the
        /// bus name that Ghostty uses to connect to DBus.
        ///
        /// Note that changing this value between invocations will create new, separate
        /// instances, of Ghostty when running with `gtk-single-instance=true`. See that
        /// option for more details.
        ///
        /// Changing this value may break launching Ghostty from `.desktop` files, via
        /// DBus activation, or systemd user services as the system is expecting Ghostty
        /// to connect to DBus using the default `class` when it is launched.
        ///
        /// The class name must follow the requirements defined [in the GTK
        /// documentation](https://docs.gtk.org/gio/type_func.Application.id_is_valid.html).
        ///
        /// The default is `com.mitchellh.ghostty`.
        ///
        /// This only affects GTK builds.
        "class" => class: Option<String> = None,

        /// This controls the instance name field of the `WM_CLASS` X11 property when
        /// running under X11. It has no effect otherwise.
        ///
        /// The default is `ghostty`.
        ///
        /// This only affects GTK builds.
        "x11-instance-name" => x11_instance_name: Option<String> = None,

        /// The directory to change to after starting the command.
        ///
        /// This setting is secondary to the `window-inherit-working-directory`
        /// setting. If a previous Ghostty terminal exists in the same process,
        /// `window-inherit-working-directory` will take precedence. Otherwise, this
        /// setting will be used. Typically, this setting is used only for the first
        /// window.
        ///
        /// The default is `inherit` except in special scenarios listed next. On macOS,
        /// if Ghostty can detect it is launched from launchd (double-clicked) or
        /// `open`, then it defaults to `home`. On Linux with GTK, if Ghostty can detect
        /// it was launched from a desktop launcher, then it defaults to `home`.
        ///
        /// The value of this must be an absolute path, a path prefixed with `~/`
        /// (the tilde will be expanded to the user's home directory), or
        /// one of the special values below:
        ///
        ///   * `home` - The home directory of the executing user.
        ///
        ///   * `inherit` - The working directory of the launching process.
        "working-directory" => working_directory: Option<WorkingDirectory> = None,

        /// Key bindings. The format is `trigger=action`. Duplicate triggers will
        /// overwrite previously set values. The list of actions is available in
        /// the documentation or using the `ghostty +list-actions` command.
        ///
        /// Trigger: `+`-separated list of keys and modifiers. Example: `ctrl+a`,
        /// `ctrl+shift+b`, `up`.
        ///
        /// If the key is a single Unicode codepoint, the trigger will match
        /// any presses that produce that codepoint. These are impacted by
        /// keyboard layouts. For example, `a` will match the `a` key on a
        /// QWERTY keyboard, but will match the `q` key on a AZERTY keyboard
        /// (assuming US physical layout).
        ///
        /// For Unicode codepoints, matching is done by comparing the set of
        /// modifiers with the unmodified codepoint. The unmodified codepoint is
        /// sometimes called an "unshifted character" in other software, but all
        /// modifiers are considered, not only shift. For example, `ctrl+a` will match
        /// `a` but not `ctrl+shift+a` (which is `A` on a US keyboard).
        ///
        /// Further, codepoint matching is case-insensitive and the unmodified
        /// codepoint is always case folded for comparison. As a result,
        /// `ctrl+A` configured will match when `ctrl+a` is pressed. Note that
        /// this means some key combinations are impossible depending on keyboard
        /// layout. For example, `ctrl+_` is impossible on a US keyboard because
        /// `_` is `shift+-` and `ctrl+shift+-` is not equal to `ctrl+_` (because
        /// the modifiers don't match!). More details on impossible key combinations
        /// can be found at this excellent source written by Qt developers:
        /// https://doc.qt.io/qt-6/qkeysequence.html#keyboard-layout-issues
        ///
        /// Physical key codes can be specified by using any of the key codes
        /// as specified by the [W3C specification](https://www.w3.org/TR/uievents-code/).
        /// For example, `KeyA` will match the physical `a` key on a US standard
        /// keyboard regardless of the keyboard layout. These are case-sensitive.
        ///
        /// For aesthetic reasons, the w3c codes also support snake case. For
        /// example, `key_a` is equivalent to `KeyA`. The only exceptions are
        /// function keys, e.g. `F1` is `f1` (no underscore). This is a consequence
        /// of our internal code using snake case but is purposely supported
        /// and tested so it is safe to use. It allows an all-lowercase binding
        /// which I find more aesthetically pleasing.
        ///
        /// Function keys such as `insert`, `up`, `f5`, etc. are also specified
        /// using the keys as specified by the previously linked W3C specification.
        ///
        /// Physical keys always match with a higher priority than Unicode codepoints,
        /// so if you specify both `a` and `KeyA`, the physical key will always be used
        /// regardless of what order they are configured.
        ///
        /// The special key `catch_all` can be used to match any key that is not
        /// otherwise bound. This can be combined with modifiers, for example
        /// `ctrl+catch_all` will match any key pressed with `ctrl` that is not
        /// otherwise bound. When looking up a binding, Ghostty first tries to match
        /// `catch_all` with modifiers. If no match is found and the event has
        /// modifiers, it falls back to `catch_all` without modifiers.
        ///
        /// Valid modifiers are `shift`, `ctrl` (alias: `control`), `alt` (alias: `opt`,
        /// `option`), and `super` (alias: `cmd`, `command`). You may use the modifier
        /// or the alias. When debugging keybinds, the non-aliased modifier will always
        /// be used in output.
        ///
        /// Note: The fn or "globe" key on keyboards are not supported as a
        /// modifier. This is a limitation of the operating systems and GUI toolkits
        /// that Ghostty uses.
        ///
        /// Some additional notes for triggers:
        ///
        ///   * modifiers cannot repeat, `ctrl+ctrl+a` is invalid.
        ///
        ///   * modifiers and keys can be in any order, `shift+a+ctrl` is *weird*,
        ///     but valid.
        ///
        ///   * only a single key input is allowed, `ctrl+a+b` is invalid.
        ///
        /// You may also specify multiple triggers separated by `>` to require a
        /// sequence of triggers to activate the action. For example,
        /// `ctrl+a>n=new_window` will only trigger the `new_window` action if the
        /// user presses `ctrl+a` followed separately by `n`. In other software, this
        /// is sometimes called a leader key, a key chord, a key table, etc. There
        /// is no hardcoded limit on the number of parts in a sequence.
        ///
        /// Warning: If you define a sequence as a CLI argument to `ghostty`,
        /// you probably have to quote the keybind since `>` is a special character
        /// in most shells. Example: ghostty --keybind='ctrl+a>n=new_window'
        ///
        /// A trigger sequence has some special handling:
        ///
        ///   * Ghostty will wait an indefinite amount of time for the next key in
        ///     the sequence. There is no way to specify a timeout. The only way to
        ///     force the output of a prefix key is to assign another keybind to
        ///     specifically output that key (e.g. `ctrl+a>ctrl+a=text:foo`) or
        ///     press an unbound key which will send both keys to the program.
        ///
        ///   * If an unbound key is pressed during a sequence and a `catch_all`
        ///     binding exists that would `ignore` the input, the entire sequence
        ///     is dropped and nothing happens. Otherwise, the entire sequence is
        ///     encoded and sent to the running program as if no keybind existed.
        ///
        ///   * If a prefix in a sequence is previously bound, the sequence will
        ///     override the previous binding. For example, if `ctrl+a` is bound to
        ///     `new_window` and `ctrl+a>n` is bound to `new_tab`, pressing `ctrl+a`
        ///     will do nothing.
        ///
        ///   * Adding to the above, if a previously bound sequence prefix is
        ///     used in a new, non-sequence binding, the entire previously bound
        ///     sequence will be unbound. For example, if you bind `ctrl+a>n` and
        ///     `ctrl+a>t`, and then bind `ctrl+a` directly, both `ctrl+a>n` and
        ///     `ctrl+a>t` will become unbound.
        ///
        ///   * Trigger sequences are not allowed for `global:` or `all:`-prefixed
        ///     triggers. This is a limitation we could remove in the future.
        ///
        /// Action is the action to take when the trigger is satisfied. It takes the
        /// format `action` or `action:param`. The latter form is only valid if the
        /// action requires a parameter.
        ///
        ///   * `ignore` - Do nothing, ignore the key input. This can be used to
        ///     black hole certain inputs to have no effect.
        ///
        ///   * `unbind` - Remove the binding. This makes it so the previous action
        ///     is removed, and the key will be sent through to the child command
        ///     if it is printable. Unbind will remove any matching trigger,
        ///     including `physical:`-prefixed triggers without specifying the
        ///     prefix.
        ///
        ///   * `csi:text` - Send a CSI sequence. e.g. `csi:A` sends "cursor up".
        ///
        ///   * `esc:text` - Send an escape sequence. e.g. `esc:d` deletes to the
        ///     end of the word to the right.
        ///
        ///   * `text:text` - Send a string. Uses Zig string literal syntax.
        ///     e.g. `text:\x15` sends Ctrl-U.
        ///
        ///   * All other actions can be found in the documentation or by using the
        ///     `ghostty +list-actions` command.
        ///
        /// Some notes for the action:
        ///
        ///   * The parameter is taken as-is after the `:`. Double quotes or
        ///     other mechanisms are included and NOT parsed. If you want to
        ///     send a string value that includes spaces, wrap the entire
        ///     trigger/action in double quotes. Example: `--keybind="up=csi:A B"`
        ///
        /// There are some additional special values that can be specified for
        /// keybind:
        ///
        ///   * `keybind=clear` will clear all set keybindings. Warning: this
        ///     removes ALL keybindings up to this point, including the default
        ///     keybindings.
        ///
        /// The keybind trigger can be prefixed with some special values to change
        /// the behavior of the keybind. These are:
        ///
        ///  * `all:`
        ///
        ///    Make the keybind apply to all terminal surfaces. By default,
        ///    keybinds only apply to the focused terminal surface. If this is true,
        ///    then the keybind will be sent to all terminal surfaces. This only
        ///    applies to actions that are surface-specific. For actions that
        ///    are already global (e.g. `quit`), this prefix has no effect.
        ///
        ///    Available since: 1.0.0
        ///
        ///  * `global:`
        ///
        ///    Make the keybind global. By default, keybinds only work within Ghostty
        ///    and under the right conditions (application focused, sometimes terminal
        ///    focused, etc.). If you want a keybind to work globally across your system
        ///    (e.g. even when Ghostty is not focused), specify this prefix.
        ///    This prefix implies `all:`.
        ///
        ///    Note: this does not work in all environments; see the additional notes
        ///    below for more information.
        ///
        ///    Available since: 1.0.0 on macOS, 1.2.0 on GTK
        ///
        ///  * `unconsumed:`
        ///
        ///    Do not consume the input. By default, a keybind will consume the input,
        ///    meaning that the associated encoding (if any) will not be sent to the
        ///    running program in the terminal. If you wish to send the encoded value
        ///    to the program, specify the `unconsumed:` prefix before the entire
        ///    keybind. For example: `unconsumed:ctrl+a=reload_config`. `global:` and
        ///    `all:`-prefixed keybinds will always consume the input regardless of
        ///    this setting. Since they are not associated with a specific terminal
        ///    surface, they're never encoded.
        ///
        ///    Available since: 1.0.0
        ///
        ///  * `performable:`
        ///
        ///    Only consume the input if the action is able to be performed.
        ///    For example, the `copy_to_clipboard` action will only consume the input
        ///    if there is a selection to copy. If there is no selection, Ghostty
        ///    behaves as if the keybind was not set. This has no effect with `global:`
        ///    or `all:`-prefixed keybinds. For key sequences, this will reset the
        ///    sequence if the action is not performable (acting identically to not
        ///    having a keybind set at all).
        ///
        ///    Performable keybinds will not appear as menu shortcuts in the
        ///    application menu. This is because the menu shortcuts force the
        ///    action to be performed regardless of the state of the terminal.
        ///    Performable keybinds will still work, they just won't appear as
        ///    a shortcut label in the menu.
        ///
        ///    Available since: 1.1.0
        ///
        /// Keybind triggers are not unique per prefix combination. For example,
        /// `ctrl+a` and `global:ctrl+a` are not two separate keybinds. The keybind
        /// set later will overwrite the keybind set earlier. In this case, the
        /// `global:` keybind will be used.
        ///
        /// Multiple prefixes can be specified. For example,
        /// `global:unconsumed:ctrl+a=reload_config` will make the keybind global
        /// and not consume the input to reload the config.
        ///
        /// Note: `global:` is only supported on macOS and certain Linux platforms.
        ///
        /// On macOS, this feature requires accessibility permissions to be granted
        /// to Ghostty. When a `global:` keybind is specified and Ghostty is launched
        /// or reloaded, Ghostty will attempt to request these permissions.
        /// If the permissions are not granted, the keybind will not work. On macOS,
        /// you can find these permissions in System Preferences -> Privacy & Security
        /// -> Accessibility.
        ///
        /// On Linux, you need a desktop environment that implements the
        /// [Global Shortcuts](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html)
        /// protocol as a part of its XDG desktop protocol implementation.
        /// Desktop environments that are known to support (or not support)
        /// global shortcuts include:
        ///
        ///  - Users using KDE Plasma (since [5.27](https://kde.org/announcements/plasma/5/5.27.0/#wayland))
        ///    and GNOME (since [48](https://release.gnome.org/48/#and-thats-not-all)) should be able
        ///    to use global shortcuts with little to no configuration.
        ///
        ///  - Some manual configuration is required on Hyprland. Consult the steps
        ///    outlined on the [Hyprland Wiki](https://wiki.hyprland.org/Configuring/Binds/#dbus-global-shortcuts)
        ///    to set up global shortcuts correctly.
        ///    (Important: [`xdg-desktop-portal-hyprland`](https://wiki.hyprland.org/Hypr-Ecosystem/xdg-desktop-portal-hyprland/)
        ///    must also be installed!)
        ///
        ///  - Notably, global shortcuts have not been implemented on wlroots-based
        ///    compositors like Sway (see [upstream issue](https://github.com/emersion/xdg-desktop-portal-wlr/issues/240)).
        ///
        /// ## Chained Actions
        ///
        /// A keybind can have multiple actions by using the `chain` keyword for
        /// subsequent actions. When a keybind is activated, all chained actions are
        /// executed in order. The syntax is:
        ///
        /// ```ini
        /// keybind = ctrl+a=new_window
        /// keybind = chain=goto_split:left
        /// ```
        ///
        /// This binds `ctrl+a` to first open a new window, then move focus to the
        /// left split. Each `chain` entry appends an action to the most recently
        /// defined keybind. You can chain as many actions as you want:
        ///
        /// ```ini
        /// keybind = ctrl+a=new_window
        /// keybind = chain=goto_split:left
        /// keybind = chain=toggle_fullscreen
        /// ```
        ///
        /// Chained actions cannot have prefixes like `global:` or `unconsumed:`.
        /// The flags from the original keybind apply to the entire chain.
        ///
        /// Chained actions work with key sequences as well. For example:
        ///
        /// ```ini
        /// keybind = ctrl+a>n=new_window
        /// keybind = chain=goto_split:left
        /// ````
        ///
        /// Chains with key sequences apply to the most recent binding in the
        /// sequence.
        ///
        /// Chained keybinds are available since Ghostty 1.3.0.
        ///
        /// ## Key Tables
        ///
        /// You may also create a named set of keybindings known as a "key table."
        /// A key table must be explicitly activated for the bindings to become
        /// available. This can be used to implement features such as a
        /// "copy mode", "vim mode", etc. Generically, this can implement modal
        /// keyboard input.
        ///
        /// Key tables are defined using the syntax `<table>/<binding>`. The
        /// `<binding>` value is everything documented above for keybinds. The
        /// `<table>` value is the name of the key table. Table names can contain
        /// anything except `/`, `=`, `+`, and `>`. The characters `+` and `>` are
        /// reserved for keybind syntax (modifier combinations and key sequences).
        /// For example `foo/ctrl+a=new_window` defines a binding within a table
        /// named `foo`.
        ///
        /// Tables are activated and deactivated using the binding actions
        /// `activate_key_table:<name>` and `deactivate_key_table`. Other table
        /// related binding actions also exist; see the documentation for a full list.
        /// These are the primary way to interact with key tables.
        ///
        /// Binding lookup proceeds from the innermost table outward, so keybinds in
        /// the default table remain available unless explicitly unbound in an inner
        /// table.
        ///
        /// A key table has some special syntax and handling:
        ///
        ///   * `<name>/` (with no binding) defines and clears a table, resetting all
        ///     of its keybinds and settings.
        ///
        ///   * You cannot activate a table that is already the innermost table; such
        ///     attempts are ignored. However, the same table can appear multiple times
        ///     in the stack as long as it is not innermost (e.g., `A -> B -> A -> B`
        ///     is valid, but `A -> B -> B` is not).
        ///
        ///   * A table can be activated in one-shot mode using
        ///     `activate_key_table_once:<name>`. A one-shot table is automatically
        ///     deactivated when any non-catch-all binding is invoked.
        ///
        ///   * Key sequences work within tables: `foo/ctrl+a>ctrl+b=new_window`.
        ///     If an invalid key is pressed, the sequence ends but the table remains
        ///     active.
        ///
        ///   * Chain actions work within tables, the `chain` keyword applies to
        ///     the most recently defined binding in the table. e.g. if you set
        ///     `table/ctrl+a=new_window` you can chain by using `chain=text:hello`.
        ///     Important: chain itself doesn't get prefixed with the table name,
        ///     since it applies to the most recent binding in any table.
        ///
        ///   * Prefixes like `global:` work within tables:
        ///     `foo/global:ctrl+a=new_window`.
        ///
        /// Key tables are available since Ghostty 1.3.0.
        "keybind" => keybind: Keybinds = Keybinds::default(),

        /// Remap modifier keys within Ghostty. This allows you to swap or reassign
        /// modifier keys at the application level without affecting system-wide
        /// settings.
        ///
        /// The format is `from=to` where both `from` and `to` are modifier key names.
        /// You can use generic names like `ctrl`, `alt`, `shift`, `super` (macOS:
        /// `cmd`/`command`) or sided names like `left_ctrl`, `right_alt`, etc.
        ///
        /// This will NOT change keyboard layout or key encodings outside of Ghostty.
        /// For example, on macOS, `option+a` may still produce `å` even if `option` is
        /// remapped to `ctrl`. Desktop environments usually handle key layout long
        /// before Ghostty receives the key events.
        ///
        /// Example:
        ///
        ///     key-remap = ctrl=super
        ///     key-remap = left_control=right_alt
        ///
        /// Important notes:
        ///
        /// * This is a one-way remap. If you remap `ctrl=super`, then the physical
        ///   Ctrl key acts as Super, but the Super key remains Super.
        ///
        /// * Remaps are not transitive. If you remap `ctrl=super` and `alt=ctrl`,
        ///   pressing Alt will produce Ctrl, NOT Super.
        ///
        /// * This affects both keybind matching and terminal input encoding.
        ///   This does NOT impact keyboard layout or how keys are interpreted
        ///   prior to Ghostty receiving them. For example, `option+a` on macOS
        ///   may still produce `å` even if `option` is remapped to `ctrl`.
        ///
        /// * Generic modifiers (e.g. `ctrl`) match both left and right physical keys.
        ///   Use sided names (e.g. `left_ctrl`) to remap only one side.
        ///
        /// There are other edge case scenarios that may not behave as expected
        /// but are working as intended the way this feature is designed:
        ///
        /// * On macOS, bindings in the main menu will trigger before any remapping
        ///   is done. This is because macOS itself handles menu activation and
        ///   this happens before Ghostty receives the key event. To workaround
        ///   this, you should unbind the menu items and rebind them using your
        ///   desired modifier.
        ///
        /// This configuration can be repeated to specify multiple remaps.
        "key-remap" => key_remap: ghostty_input::KeyRemapSet = ghostty_input::KeyRemapSet::default(),

        /// Horizontal window padding. This applies padding between the terminal cells
        /// and the left and right window borders. The value is in points, meaning that
        /// it will be scaled appropriately for screen DPI.
        ///
        /// If this value is set too large, the screen will render nothing, because the
        /// grid will be completely squished by the padding. It is up to you as the user
        /// to pick a reasonable value. If you pick an unreasonable value, a warning
        /// will appear in the logs.
        ///
        /// Changing this configuration at runtime will only affect new terminals, i.e.
        /// new windows, tabs, etc.
        ///
        /// To set a different left and right padding, specify two numerical values
        /// separated by a comma. For example, `window-padding-x = 2,4` will set the
        /// left padding to 2 and the right padding to 4. If you want to set both
        /// paddings to the same value, you can use a single value. For example,
        /// `window-padding-x = 2` will set both paddings to 2.
        "window-padding-x" => window_padding_x: WindowPadding = WindowPadding { top_left: 2, bottom_right: 2 },

        /// Vertical window padding. This applies padding between the terminal cells and
        /// the top and bottom window borders. The value is in points, meaning that it
        /// will be scaled appropriately for screen DPI.
        ///
        /// If this value is set too large, the screen will render nothing, because the
        /// grid will be completely squished by the padding. It is up to you as the user
        /// to pick a reasonable value. If you pick an unreasonable value, a warning
        /// will appear in the logs.
        ///
        /// Changing this configuration at runtime will only affect new terminals,
        /// i.e. new windows, tabs, etc.
        ///
        /// To set a different top and bottom padding, specify two numerical values
        /// separated by a comma. For example, `window-padding-y = 2,4` will set the
        /// top padding to 2 and the bottom padding to 4. If you want to set both
        /// paddings to the same value, you can use a single value. For example,
        /// `window-padding-y = 2` will set both paddings to 2.
        "window-padding-y" => window_padding_y: WindowPadding = WindowPadding { top_left: 2, bottom_right: 2 },

        /// The viewport dimensions are usually not perfectly divisible by the cell
        /// size. In this case, some extra padding on the end of a column and the bottom
        /// of the final row may exist. If this is `true`, then this extra padding
        /// is automatically balanced between all four edges to minimize imbalance on
        /// one side. If this is `false`, the top left grid cell will always hug the
        /// edge with zero padding other than what may be specified with the other
        /// `window-padding` options.
        ///
        /// If other `window-padding` fields are set and this is `true`, this will still
        /// apply. The other padding is applied first and may affect how many grid cells
        /// actually exist, and this is applied last in order to balance the padding
        /// given a certain viewport size and grid cell size.
        "window-padding-balance" => window_padding_balance: bool = false,

        /// The color of the padding area of the window. Valid values are:
        ///
        /// * `background` - The background color specified in `background`.
        /// * `extend` - Extend the background color of the nearest grid cell.
        /// * `extend-always` - Same as "extend" but always extends without applying
        ///   any of the heuristics that disable extending noted below.
        ///
        /// The "extend" value will be disabled in certain scenarios. On primary
        /// screen applications (e.g. not something like Neovim), the color will not
        /// be extended vertically if any of the following are true:
        ///
        /// * The nearest row has any cells that have the default background color.
        ///   The thinking is that in this case, the default background color looks
        ///   fine as a padding color.
        /// * The nearest row is a prompt row (requires shell integration). The
        ///   thinking here is that prompts often contain powerline glyphs that
        ///   do not look good extended.
        /// * The nearest row contains a perfect fit powerline character. These
        ///   don't look good extended.
        "window-padding-color" => window_padding_color: WindowPaddingColor = WindowPaddingColor::Background,

        /// Synchronize rendering with the screen refresh rate. If true, this will
        /// minimize tearing and align redraws with the screen but may cause input
        /// latency. If false, this will maximize redraw frequency but may cause tearing,
        /// and under heavy load may use more CPU and power.
        ///
        /// This defaults to true because out-of-sync rendering on macOS can
        /// cause kernel panics (macOS 14.4+) and performance issues for external
        /// displays over some hardware such as DisplayLink. If you want to minimize
        /// input latency, set this to false with the known aforementioned risks.
        ///
        /// Changing this value at runtime will only affect new terminals.
        ///
        /// This setting is only supported currently on macOS.
        "window-vsync" => window_vsync: bool = true,

        /// If true, new windows will inherit the working directory of the
        /// previously focused window. If no window was previously focused, the default
        /// working directory will be used (the `working-directory` option).
        "window-inherit-working-directory" => window_inherit_working_directory: bool = true,

        /// If true, new tabs will inherit the working directory of the
        /// previously focused tab. If no tab was previously focused, the default
        /// working directory will be used (the `working-directory` option).
        "tab-inherit-working-directory" => tab_inherit_working_directory: bool = true,

        /// If true, new split panes will inherit the working directory of the
        /// previously focused split. If no split was previously focused, the default
        /// working directory will be used (the `working-directory` option).
        "split-inherit-working-directory" => split_inherit_working_directory: bool = true,

        /// If true, new windows and tabs will inherit the font size of the previously
        /// focused window. If no window was previously focused, the default font size
        /// will be used. If this is false, the default font size specified in the
        /// configuration `font-size` will be used.
        "window-inherit-font-size" => window_inherit_font_size: bool = true,

        /// Configure a preference for window decorations. This setting specifies
        /// a _preference_; the actual OS, desktop environment, window manager, etc.
        /// may override this preference. Ghostty will do its best to respect this
        /// preference but it may not always be possible.
        ///
        /// Valid values:
        ///
        ///  * `none`
        ///
        ///    All window decorations will be disabled. Titlebar, borders, etc. will
        ///    not be shown. On macOS, this will also disable tabs (enforced by the
        ///    system).
        ///
        ///  * `auto`
        ///
        ///    Automatically decide to use either client-side or server-side
        ///    decorations based on the detected preferences of the current OS and
        ///    desktop environment. This option usually makes Ghostty look the most
        ///    "native" for your desktop.
        ///
        ///  * `client`
        ///
        ///    Prefer client-side decorations.
        ///
        ///    Available since: 1.1.0
        ///
        ///  * `server`
        ///
        ///    Prefer server-side decorations. This is only relevant on Linux with GTK,
        ///    either on X11, or Wayland on a compositor that supports the
        ///    `org_kde_kwin_server_decoration` protocol (e.g. KDE Plasma, but almost
        ///    any non-GNOME desktop supports this protocol).
        ///
        ///    If `server` is set but the environment doesn't support server-side
        ///    decorations, client-side decorations will be used instead.
        ///
        ///    Available since: 1.1.0
        ///
        /// The default value is `auto`.
        ///
        /// For the sake of backwards compatibility and convenience, this setting also
        /// accepts boolean true and false values. If set to `true`, this is equivalent
        /// to `auto`. If set to `false`, this is equivalent to `none`.
        /// This is convenient for users who live primarily on systems that don't
        /// differentiate between client and server-side decorations (e.g. macOS and
        /// Windows).
        ///
        /// The "toggle_window_decorations" keybind action can be used to create
        /// a keybinding to toggle this setting at runtime.
        ///
        /// macOS: To hide the titlebar without removing the native window borders
        ///        or rounded corners, use `macos-titlebar-style = hidden` instead.
        "window-decoration" => window_decoration: WindowDecoration = WindowDecoration::Auto,

        /// The font that will be used for the application's window and tab titles.
        ///
        /// If this setting is left unset, the system default font will be used.
        ///
        /// Note: any font available on the system may be used, this font is not
        /// required to be a fixed-width font.
        ///
        /// Available since: 1.0.0 on macOS, 1.1.0 on GTK
        "window-title-font-family" => window_title_font_family: Option<String> = None,

        /// The text that will be displayed in the subtitle of the window. Valid values:
        ///
        ///   * `false` - Disable the subtitle.
        ///   * `working-directory` - Set the subtitle to the working directory of the
        ///      surface.
        ///
        /// This feature is only supported on GTK.
        ///
        /// Available since: 1.1.0
        "window-subtitle" => window_subtitle: WindowSubtitle = WindowSubtitle::False,

        /// The theme to use for the windows. Valid values:
        ///
        ///   * `auto` - Determine the theme based on the configured terminal
        ///      background color. This has no effect if the "theme" configuration
        ///      has separate light and dark themes. In that case, the behavior
        ///      of "auto" is equivalent to "system".
        ///   * `system` - Use the system theme.
        ///   * `light` - Use the light theme regardless of system theme.
        ///   * `dark` - Use the dark theme regardless of system theme.
        ///   * `ghostty` - Use the background and foreground colors specified in the
        ///     Ghostty configuration. This is only supported on Linux builds.
        ///
        /// On macOS, if `macos-titlebar-style` is `tabs` or `transparent`, the window theme will be
        /// automatically set based on the luminosity of the terminal background color.
        /// This only applies to terminal windows. This setting will still apply to
        /// non-terminal windows within Ghostty.
        ///
        /// This is currently only supported on macOS and Linux.
        "window-theme" => window_theme: WindowTheme = WindowTheme::Auto,

        /// The color space to use when interpreting terminal colors. "Terminal colors"
        /// refers to colors specified in your configuration and colors produced by
        /// direct-color SGR sequences.
        ///
        /// Valid values:
        ///
        ///   * `srgb` - Interpret colors in the sRGB color space. This is the default.
        ///   * `display-p3` - Interpret colors in the Display P3 color space.
        ///
        /// This setting is currently only supported on macOS.
        "window-colorspace" => window_colorspace: WindowColorspace = WindowColorspace::Srgb,

        /// The initial window size. This size is in terminal grid cells by default.
        /// Both values must be set to take effect. If only one value is set, it is
        /// ignored.
        ///
        /// We don't currently support specifying a size in pixels but a future change
        /// can enable that. If this isn't specified, the app runtime will determine
        /// some default size.
        ///
        /// Note that the window manager may put limits on the size or override the
        /// size. For example, a tiling window manager may force the window to be a
        /// certain size to fit within the grid. There is nothing Ghostty will do about
        /// this, but it will make an effort.
        ///
        /// Sizes larger than the screen size will be clamped to the screen size.
        /// This can be used to create a maximized-by-default window size.
        ///
        /// This will not affect new tabs, splits, or other nested terminal elements.
        /// This only affects the initial window size of any new window. Changing this
        /// value will not affect the size of the window after it has been created. This
        /// is only used for the initial size.
        ///
        /// BUG: On Linux with GTK, the calculated window size will not properly take
        /// into account window decorations. As a result, the grid dimensions will not
        /// exactly match this configuration. If window decorations are disabled (see
        /// `window-decoration`), then this will work as expected.
        ///
        /// Windows smaller than 10 wide by 4 high are not allowed.
        "window-height" => window_height: u32 = 0,

        "window-width" => window_width: u32 = 0,

        /// The starting window position. This position is in pixels and is relative
        /// to the top-left corner of the primary monitor. Both values must be set to take
        /// effect. If only one value is set, it is ignored.
        ///
        /// Note that the window manager may put limits on the position or override
        /// the position. For example, a tiling window manager may force the window
        /// to be a certain position to fit within the grid. There is nothing Ghostty
        /// will do about this, but it will make an effort.
        ///
        /// Also note that negative values are also up to the operating system and
        /// window manager. Some window managers may not allow windows to be placed
        /// off-screen.
        ///
        /// Invalid positions are runtime-specific, but generally the positions are
        /// clamped to the nearest valid position.
        ///
        /// On macOS, the window position is relative to the top-left corner of
        /// the visible screen area. This means that if the menu bar is visible, the
        /// window will be placed below the menu bar.
        ///
        /// Note: this is only supported on macOS. The GTK runtime does not support
        /// setting the window position, as windows are only allowed position
        /// themselves in X11 and not Wayland.
        "window-position-x" => window_position_x: Option<i16> = None,

        "window-position-y" => window_position_y: Option<i16> = None,

        /// Whether to enable saving and restoring window state. Window state includes
        /// their position, size, tabs, splits, etc. Some window state requires shell
        /// integration, such as preserving working directories. See `shell-integration`
        /// for more information.
        ///
        /// There are three valid values for this configuration:
        ///
        ///   * `default` will use the default system behavior. On macOS, this
        ///     will only save state if the application is forcibly terminated
        ///     or if it is configured systemwide via Settings.app.
        ///
        ///   * `never` will never save window state.
        ///
        ///   * `always` will always save window state whenever Ghostty is exited.
        ///
        /// If you change this value to `never` while Ghostty is not running, the next
        /// Ghostty launch will NOT restore the window state.
        ///
        /// If you change this value to `default` while Ghostty is not running and the
        /// previous exit saved state, the next Ghostty launch will still restore the
        /// window state. This is because Ghostty cannot know if the previous exit was
        /// due to a forced save or not (macOS doesn't provide this information).
        ///
        /// If you change this value so that window state is saved while Ghostty is not
        /// running, the previous window state will not be restored because Ghostty only
        /// saves state on exit if this is enabled.
        ///
        /// The default value is `default`.
        ///
        /// This is currently only supported on macOS. This has no effect on Linux.
        "window-save-state" => window_save_state: WindowSaveState = WindowSaveState::Default,

        /// Resize the window in discrete increments of the focused surface's cell size.
        /// If this is disabled, surfaces are resized in pixel increments. Currently
        /// only supported on macOS.
        "window-step-resize" => window_step_resize: bool = false,

        /// The position where new tabs are created. Valid values:
        ///
        ///   * `current` - Insert the new tab after the currently focused tab,
        ///     or at the end if there are no focused tabs.
        ///
        ///   * `end` - Insert the new tab at the end of the tab list.
        "window-new-tab-position" => window_new_tab_position: WindowNewTabPosition = WindowNewTabPosition::Current,

        /// Whether to show the tab bar.
        ///
        /// Valid values:
        ///
        ///  - `always`
        ///
        ///    Always display the tab bar, even when there's only one tab.
        ///
        ///    Available since: 1.2.0
        ///
        ///  - `auto` *(default)*
        ///
        ///    Automatically show and hide the tab bar. The tab bar is only
        ///    shown when there are two or more tabs present.
        ///
        ///  - `never`
        ///
        ///    Never show the tab bar. Tabs are only accessible via the tab
        ///    overview or by keybind actions.
        ///
        /// Currently only supported on Linux (GTK).
        "window-show-tab-bar" => window_show_tab_bar: WindowShowTabBar = WindowShowTabBar::Auto,

        /// Background color for the window titlebar. This only takes effect if
        /// window-theme is set to ghostty. Currently only supported in the GTK app
        /// runtime.
        ///
        /// Specified as either hex (`#RRGGBB` or `RRGGBB`) or a named X11 color.
        "window-titlebar-background" => window_titlebar_background: Option<Color> = None,

        /// Foreground color for the window titlebar. This only takes effect if
        /// window-theme is set to ghostty. Currently only supported in the GTK app
        /// runtime.
        ///
        /// Specified as either hex (`#RRGGBB` or `RRGGBB`) or a named X11 color.
        "window-titlebar-foreground" => window_titlebar_foreground: Option<Color> = None,

        /// This controls when resize overlays are shown. Resize overlays are a
        /// transient popup that shows the size of the terminal while the surfaces are
        /// being resized. The possible options are:
        ///
        ///   * `always` - Always show resize overlays.
        ///   * `never` - Never show resize overlays.
        ///   * `after-first` - The resize overlay will not appear when the surface
        ///                     is first created, but will show up if the surface is
        ///                     subsequently resized.
        ///
        /// The default is `after-first`.
        "resize-overlay" => resize_overlay: ResizeOverlay = ResizeOverlay::AfterFirst,

        /// If resize overlays are enabled, this controls the position of the overlay.
        /// The possible options are:
        ///
        ///   * `center`
        ///   * `top-left`
        ///   * `top-center`
        ///   * `top-right`
        ///   * `bottom-left`
        ///   * `bottom-center`
        ///   * `bottom-right`
        ///
        /// The default is `center`.
        "resize-overlay-position" => resize_overlay_position: ResizeOverlayPosition = ResizeOverlayPosition::Center,

        /// If resize overlays are enabled, this controls how long the overlay is
        /// visible on the screen before it is hidden. The default is ¾ of a second or
        /// 750 ms.
        ///
        /// The duration is specified as a series of numbers followed by time units.
        /// Whitespace is allowed between numbers and units. Each number and unit will
        /// be added together to form the total duration.
        ///
        /// The allowed time units are as follows:
        ///
        ///   * `y` - 365 SI days, or 8760 hours, or 31536000 seconds. No adjustments
        ///     are made for leap years or leap seconds.
        ///   * `d` - one SI day, or 86400 seconds.
        ///   * `h` - one hour, or 3600 seconds.
        ///   * `m` - one minute, or 60 seconds.
        ///   * `s` - one second.
        ///   * `ms` - one millisecond, or 0.001 second.
        ///   * `us` or `µs` - one microsecond, or 0.000001 second.
        ///   * `ns` - one nanosecond, or 0.000000001 second.
        ///
        /// Examples:
        ///   * `1h30m`
        ///   * `45s`
        ///
        /// Units can be repeated and will be added together. This means that
        /// `1h1h` is equivalent to `2h`. This is confusing and should be avoided.
        /// A future update may disallow this.
        ///
        /// The maximum value is `584y 49w 23h 34m 33s 709ms 551µs 615ns`. Any
        /// value larger than this will be clamped to the maximum value.
        ///
        /// Available since 1.0.0
        "resize-overlay-duration" => resize_overlay_duration: Duration = Duration { duration: 750 * NS_PER_MS },

        /// If true, when there are multiple split panes, the mouse selects the pane
        /// that is focused. This only applies to the currently focused window; e.g.
        /// mousing over a split in an unfocused window will not focus that split
        /// and bring the window to front.
        ///
        /// Default is false.
        "focus-follows-mouse" => focus_follows_mouse: bool = false,

        /// Whether to allow programs running in the terminal to read/write to the
        /// system clipboard (OSC 52, for googling). The default is to allow clipboard
        /// reading after prompting the user and allow writing unconditionally.
        ///
        /// Valid values are:
        ///
        ///   * `ask`
        ///   * `allow`
        ///   * `deny`
        ///
        "clipboard-read" => clipboard_read: ClipboardAccess = ClipboardAccess::Ask,

        "clipboard-write" => clipboard_write: ClipboardAccess = ClipboardAccess::Allow,

        /// Trims trailing whitespace on data that is copied to the clipboard. This does
        /// not affect data sent to the clipboard via `clipboard-write`. This only
        /// applies to trailing whitespace on lines that have other characters.
        /// Completely blank lines always have their whitespace trimmed.
        "clipboard-trim-trailing-spaces" => clipboard_trim_trailing_spaces: bool = true,

        /// Require confirmation before pasting text that appears unsafe. This helps
        /// prevent a "copy/paste attack" where a user may accidentally execute unsafe
        /// commands by pasting text with newlines.
        "clipboard-paste-protection" => clipboard_paste_protection: bool = true,

        /// If true, bracketed pastes will be considered safe. By default, bracketed
        /// pastes are considered safe. "Bracketed" pastes are pastes while the running
        /// program has bracketed paste mode enabled (a setting set by the running
        /// program, not the terminal emulator).
        "clipboard-paste-bracketed-safe" => clipboard_paste_bracketed_safe: bool = true,

        /// Enables or disabled title reporting (CSI 21 t). This escape sequence
        /// allows the running program to query the terminal title. This is a common
        /// security issue and is disabled by default.
        ///
        /// Warning: This can expose sensitive information at best and enable
        /// arbitrary code execution at worst (with a maliciously crafted title
        /// and a minor amount of user interaction).
        ///
        /// Available since: 1.0.1
        "title-report" => title_report: bool = false,

        /// The total amount of bytes that can be used for image data (e.g. the Kitty
        /// image protocol) per terminal screen. The maximum value is 4,294,967,295
        /// (4GiB). The default is 320MB. If this is set to zero, then all image
        /// protocols will be disabled.
        ///
        /// This value is separate for primary and alternate screens so the effective
        /// limit per surface is double.
        "image-storage-limit" => image_storage_limit: u32 = 320 * 1000 * 1000,

        /// Whether to automatically copy selected text to the clipboard. `true`
        /// will prefer to copy to the selection clipboard, otherwise it will copy to
        /// the system clipboard.
        ///
        /// The value `clipboard` will always copy text to the selection clipboard
        /// as well as the system clipboard.
        ///
        /// Middle-click paste will always use the selection clipboard. Middle-click
        /// paste is always enabled even if this is `false`.
        ///
        /// The default value is true on Linux and macOS.
        "copy-on-select" => copy_on_select: CopyOnSelect = CopyOnSelect::True,

        /// The action to take when the user right-clicks on the terminal surface.
        ///
        /// Valid values:
        ///   * `context-menu` - Show the context menu.
        ///   * `paste` - Paste the contents of the clipboard.
        ///   * `copy` - Copy the selected text to the clipboard.
        ///   * `copy-or-paste` - If there is a selection, copy the selected text to
        ///      the clipboard; otherwise, paste the contents of the clipboard.
        ///   * `ignore` - Do nothing, ignore the right-click.
        ///
        /// The default value is `context-menu`.
        "right-click-action" => right_click_action: RightClickAction = RightClickAction::ContextMenu,

        /// The time in milliseconds between clicks to consider a click a repeat
        /// (double, triple, etc.) or an entirely new single click. A value of zero will
        /// use a platform-specific default. The default on macOS is determined by the
        /// OS settings. On every other platform it is 500ms.
        "click-repeat-interval" => click_repeat_interval: u32 = 0,

        /// Additional configuration files to read. This configuration can be repeated
        /// to read multiple configuration files. Configuration files themselves can
        /// load more configuration files. Paths are relative to the file containing the
        /// `config-file` directive. For command-line arguments, paths are relative to
        /// the current working directory.
        ///
        /// Prepend a ? character to the file path to suppress errors if the file does
        /// not exist. If you want to include a file that begins with a literal ?
        /// character, surround the file path in double quotes (").
        ///
        /// Cycles are not allowed. If a cycle is detected, an error will be logged and
        /// the configuration file will be ignored.
        ///
        /// Configuration files are loaded after the configuration they're defined
        /// within in the order they're defined. **THIS IS A VERY SUBTLE BUT IMPORTANT
        /// POINT.** To put it another way: configuration files do not take effect
        /// until after the entire configuration is loaded. For example, in the
        /// configuration below:
        ///
        /// ```
        /// config-file = "foo"
        /// a = 1
        /// ```
        ///
        /// If "foo" contains `a = 2`, the final value of `a` will be 2, because
        /// `foo` is loaded after the configuration file that configures the
        /// nested `config-file` value.
        "config-file" => config_file: RepeatablePath = RepeatablePath::default(),

        /// When this is true, the default configuration file paths will be loaded.
        /// The default configuration file paths are currently only the XDG
        /// config path ($XDG_CONFIG_HOME/ghostty/config.ghostty).
        ///
        /// If this is false, the default configuration paths will not be loaded.
        /// This is targeted directly at using Ghostty from the CLI in a way
        /// that minimizes external effects.
        ///
        /// This is a CLI-only configuration. Setting this in a configuration file
        /// will have no effect. It is not an error, but it will not do anything.
        /// This configuration can only be set via CLI arguments.
        "config-default-files" => config_default_files: bool = true,

        /// Confirms that a surface should be closed before closing it.
        ///
        /// This defaults to `true`. If set to `false`, surfaces will close without
        /// any confirmation. This can also be set to `always`, which will always
        /// confirm closing a surface, even if shell integration says a process isn't
        /// running.
        "confirm-close-surface" => confirm_close_surface: ConfirmCloseSurface = ConfirmCloseSurface::True,

        /// Whether or not to quit after the last surface is closed.
        ///
        /// This defaults to `false` on macOS since that is standard behavior for
        /// a macOS application. On Linux, this defaults to `true` since that is
        /// generally expected behavior.
        ///
        /// On Linux, if this is `true`, Ghostty can delay quitting fully until a
        /// configurable amount of time has passed after the last window is closed.
        /// See the documentation of `quit-after-last-window-closed-delay`.
        "quit-after-last-window-closed" => quit_after_last_window_closed: bool = true,

        /// Controls how long Ghostty will stay running after the last open surface has
        /// been closed. This only has an effect if `quit-after-last-window-closed` is
        /// also set to `true`.
        ///
        /// The minimum value for this configuration is `1s`. Any values lower than
        /// this will be clamped to `1s`.
        ///
        /// The duration is specified as a series of numbers followed by time units.
        /// Whitespace is allowed between numbers and units. Each number and unit will
        /// be added together to form the total duration.
        ///
        /// The allowed time units are as follows:
        ///
        ///   * `y` - 365 SI days, or 8760 hours, or 31536000 seconds. No adjustments
        ///     are made for leap years or leap seconds.
        ///   * `d` - one SI day, or 86400 seconds.
        ///   * `h` - one hour, or 3600 seconds.
        ///   * `m` - one minute, or 60 seconds.
        ///   * `s` - one second.
        ///   * `ms` - one millisecond, or 0.001 second.
        ///   * `us` or `µs` - one microsecond, or 0.000001 second.
        ///   * `ns` - one nanosecond, or 0.000000001 second.
        ///
        /// Examples:
        ///   * `1h30m`
        ///   * `45s`
        ///
        /// Units can be repeated and will be added together. This means that
        /// `1h1h` is equivalent to `2h`. This is confusing and should be avoided.
        /// A future update may disallow this.
        ///
        /// The maximum value is `584y 49w 23h 34m 33s 709ms 551µs 615ns`. Any
        /// value larger than this will be clamped to the maximum value.
        ///
        /// By default `quit-after-last-window-closed-delay` is unset and
        /// Ghostty will quit immediately after the last window is closed if
        /// `quit-after-last-window-closed` is `true`.
        ///
        /// Only implemented on Linux.
        "quit-after-last-window-closed-delay" => quit_after_last_window_closed_delay: Option<Duration> = None,

        /// This controls whether an initial window is created when Ghostty
        /// is run. Note that if `quit-after-last-window-closed` is `true` and
        /// `quit-after-last-window-closed-delay` is set, setting `initial-window` to
        /// `false` will mean that Ghostty will quit after the configured delay if no
        /// window is ever created. Only implemented on Linux and macOS.
        "initial-window" => initial_window: bool = true,

        /// The duration that undo operations remain available. After this
        /// time, the operation will be removed from the undo stack and
        /// cannot be undone.
        ///
        /// The default value is 5 seconds.
        ///
        /// This timeout applies per operation, meaning that if you perform
        /// multiple operations, each operation will have its own timeout.
        /// New operations do not reset the timeout of previous operations.
        ///
        /// A timeout of zero will effectively disable undo operations. It is
        /// not possible to set an infinite timeout, but you can set a very
        /// large timeout to effectively disable the timeout (on the order of years).
        /// This is highly discouraged, as it will cause the undo stack to grow
        /// indefinitely, memory usage to grow unbounded, and terminal sessions
        /// to never actually quit.
        ///
        /// The duration is specified as a series of numbers followed by time units.
        /// Whitespace is allowed between numbers and units. Each number and unit will
        /// be added together to form the total duration.
        ///
        /// The allowed time units are as follows:
        ///
        ///   * `y` - 365 SI days, or 8760 hours, or 31536000 seconds. No adjustments
        ///     are made for leap years or leap seconds.
        ///   * `d` - one SI day, or 86400 seconds.
        ///   * `h` - one hour, or 3600 seconds.
        ///   * `m` - one minute, or 60 seconds.
        ///   * `s` - one second.
        ///   * `ms` - one millisecond, or 0.001 second.
        ///   * `us` or `µs` - one microsecond, or 0.000001 second.
        ///   * `ns` - one nanosecond, or 0.000000001 second.
        ///
        /// Examples:
        ///   * `1h30m`
        ///   * `45s`
        ///
        /// Units can be repeated and will be added together. This means that
        /// `1h1h` is equivalent to `2h`. This is confusing and should be avoided.
        /// A future update may disallow this.
        ///
        /// This configuration is only supported on macOS. Linux doesn't
        /// support undo operations at all so this configuration has no
        /// effect.
        ///
        /// Available since: 1.2.0
        "undo-timeout" => undo_timeout: Duration = Duration { duration: 5 * NS_PER_S },

        /// The position of the "quick" terminal window. To learn more about the
        /// quick terminal, see the documentation for the `toggle_quick_terminal`
        /// binding action.
        ///
        /// Valid values are:
        ///
        ///   * `top` - Terminal appears at the top of the screen.
        ///   * `bottom` - Terminal appears at the bottom of the screen.
        ///   * `left` - Terminal appears at the left of the screen.
        ///   * `right` - Terminal appears at the right of the screen.
        ///   * `center` - Terminal appears at the center of the screen.
        ///
        /// On macOS, changing this configuration requires restarting Ghostty
        /// completely.
        ///
        /// Note: There is no default keybind for toggling the quick terminal.
        /// To enable this feature, bind the `toggle_quick_terminal` action to a key.
        "quick-terminal-position" => quick_terminal_position: QuickTerminalPosition = QuickTerminalPosition::Top,

        /// The size of the quick terminal.
        ///
        /// The size can be specified either as a percentage of the screen dimensions
        /// (height/width), or as an absolute size in pixels. Percentage values are
        /// suffixed with `%` (e.g. `20%`) while pixel values are suffixed with `px`
        /// (e.g. `300px`). A bare value without a suffix is a config error.
        ///
        /// When only one size is specified, the size parameter affects the size of
        /// the quick terminal on its *primary axis*, which depends on its position:
        /// height for quick terminals placed on the top or bottom, and width for left
        /// or right. The primary axis of a centered quick terminal depends on the
        /// monitor's orientation: height when on a landscape monitor, and width when
        /// on a portrait monitor.
        ///
        /// The *secondary axis* would be maximized for non-center positioned
        /// quick terminals unless another size parameter is specified, separated
        /// from the first by a comma (`,`). Percentage and pixel sizes can be mixed
        /// together: for instance, a size of `50%,500px` for a top-positioned quick
        /// terminal would be half a screen tall, and 500 pixels wide.
        ///
        /// Available since: 1.2.0
        "quick-terminal-size" => quick_terminal_size: QuickTerminalSize = QuickTerminalSize::default(),

        /// The layer of the quick terminal window. The higher the layer,
        /// the more windows the quick terminal may conceal.
        ///
        /// Valid values are:
        ///
        ///  * `overlay`
        ///
        ///    The quick terminal appears in front of all windows.
        ///
        ///  * `top` (default)
        ///
        ///    The quick terminal appears in front of normal windows but behind
        ///    fullscreen overlays like lock screens.
        ///
        ///  * `bottom`
        ///
        ///    The quick terminal appears behind normal windows but in front of
        ///    wallpapers and other windows in the background layer.
        ///
        ///  * `background`
        ///
        ///    The quick terminal appears behind all windows.
        ///
        /// GTK Wayland only.
        ///
        /// Available since: 1.2.0
        "gtk-quick-terminal-layer" => gtk_quick_terminal_layer: QuickTerminalLayer = QuickTerminalLayer::Top,

        /// The namespace for the quick terminal window.
        ///
        /// This is an identifier that is used by the Wayland compositor and/or
        /// scripts to determine the type of layer surfaces and to possibly apply
        /// unique effects.
        ///
        /// GTK Wayland only.
        ///
        /// Available since: 1.2.0
        "gtk-quick-terminal-namespace" => gtk_quick_terminal_namespace: String = "ghostty-quick-terminal".to_string(),

        /// The screen where the quick terminal should show up.
        ///
        /// Valid values are:
        ///
        ///  * `main` - The screen that the operating system recommends as the main
        ///    screen. On macOS, this is the screen that is currently receiving
        ///    keyboard input. This screen is defined by the operating system and
        ///    not chosen by Ghostty.
        ///
        ///  * `mouse` - The screen that the mouse is currently hovered over.
        ///
        ///  * `macos-menu-bar` - The screen that contains the macOS menu bar as
        ///    set in the display settings on macOS. This is a bit confusing because
        ///    every screen on macOS has a menu bar, but this is the screen that
        ///    contains the primary menu bar.
        ///
        /// The default value is `main` because this is the recommended screen
        /// by the operating system.
        ///
        /// Only implemented on macOS.
        "quick-terminal-screen" => quick_terminal_screen: QuickTerminalScreen = QuickTerminalScreen::Main,

        /// Duration (in seconds) of the quick terminal enter and exit animation.
        /// Set it to 0 to disable animation completely. This can be changed at
        /// runtime.
        ///
        /// Only implemented on macOS.
        "quick-terminal-animation-duration" => quick_terminal_animation_duration: f64 = 0.2,

        /// Automatically hide the quick terminal when focus shifts to another window.
        /// Set it to false for the quick terminal to remain open even when it loses focus.
        ///
        /// Defaults to true on macOS and on false on Linux/BSD. This is because global
        /// shortcuts on Linux require system configuration and are considerably less
        /// accessible than on macOS, meaning that it is more preferable to keep the
        /// quick terminal open until the user has completed their task.
        /// This default may change in the future.
        "quick-terminal-autohide" => quick_terminal_autohide: bool = false,

        /// This configuration option determines the behavior of the quick terminal
        /// when switching between macOS spaces. macOS spaces are virtual desktops
        /// that can be manually created or are automatically created when an
        /// application is in full-screen mode.
        ///
        /// Valid values are:
        ///
        ///  * `move` - When switching to another space, the quick terminal will
        ///    also moved to the current space.
        ///
        ///  * `remain` - The quick terminal will stay only in the space where it
        ///    was originally opened and will not follow when switching to another
        ///    space.
        ///
        /// The default value is `move`.
        ///
        /// Only implemented on macOS.
        /// On Linux the behavior is always equivalent to `move`.
        ///
        /// Available since: 1.1.0
        "quick-terminal-space-behavior" => quick_terminal_space_behavior: QuickTerminalSpaceBehavior = QuickTerminalSpaceBehavior::Move,

        /// Determines under which circumstances that the quick terminal should receive
        /// keyboard input. See the corresponding [Wayland documentation](https://wayland.app/protocols/wlr-layer-shell-unstable-v1#zwlr_layer_surface_v1:enum:keyboard_interactivity)
        /// for a more detailed explanation of the behavior of each option.
        ///
        /// > [!NOTE]
        /// > The exact behavior of each option may differ significantly across
        /// > compositors -- experiment with them on your system to find one that
        /// > suits your liking!
        ///
        /// Valid values are:
        ///
        ///  * `none`
        ///
        ///    The quick terminal will not receive any keyboard input.
        ///
        ///  * `on-demand` (default)
        ///
        ///    The quick terminal would only receive keyboard input when it is focused.
        ///
        ///  * `exclusive`
        ///
        ///    The quick terminal will always receive keyboard input, even when another
        ///    window is currently focused.
        ///
        /// Only has an effect on Linux Wayland.
        /// On macOS the behavior is always equivalent to `on-demand`.
        ///
        /// Available since: 1.2.0
        "quick-terminal-keyboard-interactivity" => quick_terminal_keyboard_interactivity: QuickTerminalKeyboardInteractivity = QuickTerminalKeyboardInteractivity::OnDemand,

        /// Whether to enable shell integration auto-injection or not. Shell integration
        /// greatly enhances the terminal experience by enabling a number of features:
        ///
        ///   * Working directory reporting so new tabs, splits inherit the
        ///     previous terminal's working directory.
        ///
        ///   * Prompt marking that enables the "jump_to_prompt" keybinding.
        ///
        ///   * If you're sitting at a prompt, closing a terminal will not ask
        ///     for confirmation.
        ///
        ///   * Resizing the window with a complex prompt usually paints much
        ///     better.
        ///
        /// Allowable values are:
        ///
        ///   * `none` - Do not do any automatic injection. You can still manually
        ///     configure your shell to enable the integration.
        ///
        ///   * `detect` - Detect the shell based on the filename.
        ///
        ///   * `bash`, `elvish`, `fish`, `nushell`, `zsh` - Use this specific shell injection scheme.
        ///
        /// The default value is `detect`.
        "shell-integration" => shell_integration: ShellIntegration = ShellIntegration::Detect,

        /// Shell integration features to enable. These require our shell integration
        /// to be loaded, either automatically via shell-integration or manually.
        ///
        /// The format of this is a list of features to enable separated by commas. If
        /// you prefix a feature with `no-` then it is disabled. If you omit a feature,
        /// its default value is used, so you must explicitly disable features you don't
        /// want. You can also use `true` or `false` to turn all features on or off.
        ///
        /// Example: `cursor`, `no-cursor`, `sudo`, `no-sudo`, `title`, `no-title`
        ///
        /// Available features:
        ///
        ///   * `cursor` - Set the cursor to a bar at the prompt.
        ///
        ///   * `sudo` - Set sudo wrapper to preserve terminfo.
        ///
        ///   * `title` - Set the window title via shell integration.
        ///
        ///   * `ssh-env` - Enable SSH environment variable compatibility. Automatically
        ///     converts TERM from `xterm-ghostty` to `xterm-256color` when connecting to
        ///     remote hosts and propagates COLORTERM, TERM_PROGRAM, and TERM_PROGRAM_VERSION.
        ///     Whether or not these variables will be accepted by the remote host(s) will
        ///     depend on whether or not the variables are allowed in their sshd_config.
        ///     (Available since: 1.2.0)
        ///
        ///   * `ssh-terminfo` - Enable automatic terminfo installation on remote hosts.
        ///     Attempts to install Ghostty's terminfo entry using `infocmp` and `tic` when
        ///     connecting to hosts that lack it. Requires `infocmp` to be available locally
        ///     and `tic` to be available on remote hosts. Once terminfo is installed on a
        ///     remote host, it will be automatically "cached" to avoid repeat installations.
        ///     If desired, the `+ssh-cache` CLI action can be used to manage the installation
        ///     cache manually using various arguments.
        ///     (Available since: 1.2.0)
        ///
        ///   * `path` - Add Ghostty's binary directory to PATH. This ensures the `ghostty`
        ///     command is available in the shell even if shell init scripts reset PATH.
        ///     This is particularly useful on macOS where PATH is often overridden by
        ///     system scripts. The directory is only added if not already present.
        ///
        /// SSH features work independently and can be combined for optimal experience:
        /// when both `ssh-env` and `ssh-terminfo` are enabled, Ghostty will install its
        /// terminfo on remote hosts and use `xterm-ghostty` as TERM, falling back to
        /// `xterm-256color` with environment variables if terminfo installation fails.
        "shell-integration-features" => shell_integration_features: ShellIntegrationFeatures = ShellIntegrationFeatures::default(),

        /// Custom entries into the command palette.
        ///
        /// Each entry requires the title, the corresponding action, and an optional
        /// description. Each field should be prefixed with the field name, a colon
        /// (`:`), and then the specified value. The syntax for actions is identical
        /// to the one for keybind actions. Whitespace in between fields is ignored.
        ///
        /// If you need to embed commas or any other special characters in the values,
        /// enclose the value in double quotes and it will be interpreted as a Zig
        /// string literal. This is also useful for including whitespace at the
        /// beginning or the end of a value. See the
        /// [Zig documentation](https://ziglang.org/documentation/master/#Escape-Sequences)
        /// for more information on string literals. Note that multiline string literals
        /// are not supported.
        ///
        /// Double quotes can not be used around the field names.
        ///
        /// ```ini
        /// command-palette-entry = title:Reset Font Style, action:csi:0m
        /// command-palette-entry = title:Crash on Main Thread,description:Causes a crash on the main (UI) thread.,action:crash:main
        /// command-palette-entry = title:Focus Split: Right,description:"Focus the split to the right, if it exists.",action:goto_split:right
        /// command-palette-entry = title:"Ghostty",description:"Add a little Ghostty to your terminal.",action:"text:\xf0\x9f\x91\xbb"
        /// ```
        ///
        /// By default, the command palette is preloaded with most actions that might
        /// be useful in an interactive setting yet do not have easily accessible or
        /// memorizable shortcuts. The default entries can be cleared by setting this
        /// setting to an empty value:
        ///
        /// ```ini
        /// command-palette-entry =
        /// ```
        ///
        /// Available since: 1.2.0
        "command-palette-entry" => command_palette_entry: RepeatableCommand = RepeatableCommand::default(),

        /// Sets the reporting format for OSC sequences that request color information.
        /// Ghostty currently supports OSC 10 (foreground), OSC 11 (background), and
        /// OSC 4 (256 color palette) queries, and by default the reported values
        /// are scaled-up RGB values, where each component are 16 bits. This is how
        /// most terminals report these values. However, some legacy applications may
        /// require 8-bit, unscaled, components. We also support turning off reporting
        /// altogether. The components are lowercase hex values.
        ///
        /// Allowable values are:
        ///
        ///   * `none` - OSC 4/10/11 queries receive no reply
        ///
        ///   * `8-bit` - Color components are return unscaled, e.g. `rr/gg/bb`
        ///
        ///   * `16-bit` - Color components are returned scaled, e.g. `rrrr/gggg/bbbb`
        ///
        /// The default value is `16-bit`.
        "osc-color-report-format" => osc_color_report_format: OSCColorReportFormat = OSCColorReportFormat::Bit16,

        /// If true, allows the "KAM" mode (ANSI mode 2) to be used within
        /// the terminal. KAM disables keyboard input at the request of the
        /// application. This is not a common feature and is not recommended
        /// to be enabled. This will not be documented further because
        /// if you know you need KAM, you know. If you don't know if you
        /// need KAM, you don't need it.
        "vt-kam-allowed" => vt_kam_allowed: bool = false,

        /// Custom shaders to run after the default shaders. This is a file path
        /// to a GLSL-syntax shader for all platforms.
        ///
        /// Warning: Invalid shaders can cause Ghostty to become unusable such as by
        /// causing the window to be completely black. If this happens, you can
        /// unset this configuration to disable the shader.
        ///
        /// Custom shader support is based on and compatible with the Shadertoy shaders.
        /// Shaders should specify a `mainImage` function and the available uniforms
        /// largely match Shadertoy, with some caveats and Ghostty-specific extensions.
        ///
        /// The uniform values available to shaders are as follows:
        ///
        ///  * `sampler2D iChannel0` - Input texture.
        ///
        ///    A texture containing the current terminal screen. If multiple custom
        ///    shaders are specified, the output of previous shaders is written to
        ///    this texture, to allow combining multiple effects.
        ///
        ///  * `vec3 iResolution` - Output texture size, `[width, height, 1]` (in px).
        ///
        ///  * `float iTime` - Time in seconds since first frame was rendered.
        ///
        ///  * `float iTimeDelta` - Time in seconds since previous frame was rendered.
        ///
        ///  * `float iFrameRate` - Average framerate. (NOT CURRENTLY SUPPORTED)
        ///
        ///  * `int iFrame` - Number of frames that have been rendered so far.
        ///
        ///  * `float iChannelTime[4]` - Current time for video or sound input. (N/A)
        ///
        ///  * `vec3 iChannelResolution[4]` - Resolutions of the 4 input samplers.
        ///
        ///    Currently only `iChannel0` exists, and `iChannelResolution[0]` is
        ///    identical to `iResolution`.
        ///
        ///  * `vec4 iMouse` - Mouse input info. (NOT CURRENTLY SUPPORTED)
        ///
        ///  * `vec4 iDate` - Date/time info. (NOT CURRENTLY SUPPORTED)
        ///
        ///  * `float iSampleRate` - Sample rate for audio. (N/A)
        ///
        /// Ghostty-specific extensions:
        ///
        ///  * `vec4 iCurrentCursor` - Info about the terminal cursor.
        ///
        ///    - `iCurrentCursor.xy` is the -X, +Y corner of the current cursor.
        ///    - `iCurrentCursor.zw` is the width and height of the current cursor.
        ///
        ///  * `vec4 iPreviousCursor` - Info about the previous terminal cursor.
        ///
        ///  * `vec4 iCurrentCursorColor` - Color of the terminal cursor.
        ///
        ///  * `vec4 iPreviousCursorColor` - Color of the previous terminal cursor.
        ///
        ///  * `vec4 iCurrentCursorStyle` - Style of the terminal cursor
        ///
        ///    Macros simplified use are defined for the various cursor styles:
        ///
        ///    - `CURSORSTYLE_BLOCK` or `0`
        ///    - `CURSORSTYLE_BLOCK_HOLLOW` or `1`
        ///    - `CURSORSTYLE_BAR` or `2`
        ///    - `CURSORSTYLE_UNDERLINE` or `3`
        ///    - `CURSORSTYLE_LOCK` or `4`
        ///
        ///  * `vec4 iPreviousCursorStyle` - Style of the previous terminal cursor
        ///
        ///  * `vec4 iCursorVisible` - Visibility of the terminal cursor.
        ///
        ///  * `float iTimeCursorChange` - Timestamp of terminal cursor change.
        ///
        ///    When the terminal cursor changes position or color, this is set to
        ///    the same time as the `iTime` uniform, allowing you to compute the
        ///    time since the change by subtracting this from `iTime`.
        ///
        ///  * `float iTimeFocus` - Timestamp when the surface last gained iFocus.
        ///
        ///    When the surface gains focus, this is set to the current value of
        ///    `iTime`, similar to how `iTimeCursorChange` works. This allows you
        ///    to compute the time since focus was gained or lost by calculating
        ///    `iTime - iTimeFocus`. Use this to create animations that restart
        ///    when the terminal regains focus.
        ///
        ///  * `int iFocus` - Current focus state of the surface.
        ///
        ///    Set to 1.0 when the surface is focused, 0.0 when unfocused. This
        ///    allows shaders to detect unfocused state and avoid animation artifacts
        ///    from large time deltas caused by infrequent "deceptive frames"
        ///    (e.g., modifier key presses, link hover events in unfocused split panes).
        ///    Check `iFocus > 0` to determine if the surface is currently focused.
        ///
        ///  * `vec3 iPalette[256]` - The 256-color terminal palette.
        ///
        ///    RGB values for all 256 colors in the terminal palette, normalized
        ///    to [0.0, 1.0]. Index 0-15 are the ANSI colors, 16-231 are the 6x6x6
        ///    color cube, and 232-255 are the grayscale colors.
        ///
        ///  * `vec3 iBackgroundColor` - Terminal background color (RGB).
        ///
        ///  * `vec3 iForegroundColor` - Terminal foreground color (RGB).
        ///
        ///  * `vec3 iCursorColor` - Terminal cursor color (RGB).
        ///
        ///  * `vec3 iCursorText` - Terminal cursor text color (RGB).
        ///
        ///  * `vec3 iSelectionBackgroundColor` - Selection background color (RGB).
        ///
        ///  * `vec3 iSelectionForegroundColor` - Selection foreground color (RGB).
        ///
        /// If the shader fails to compile, the shader will be ignored. Any errors
        /// related to shader compilation will not show up as configuration errors
        /// and only show up in the log, since shader compilation happens after
        /// configuration loading on the dedicated render thread.  For interactive
        /// development, use [shadertoy.com](https://shadertoy.com).
        ///
        /// This can be repeated multiple times to load multiple shaders. The shaders
        /// will be run in the order they are specified.
        ///
        /// This can be changed at runtime and will affect all open terminals.
        "custom-shader" => custom_shader: RepeatablePath = RepeatablePath::default(),

        /// If `true` (default), the focused terminal surface will run an animation
        /// loop when custom shaders are used. This uses slightly more CPU (generally
        /// less than 10%) but allows the shader to animate. This only runs if there
        /// are custom shaders and the terminal is focused.
        ///
        /// If this is set to `false`, the terminal and custom shader will only render
        /// when the terminal is updated. This is more efficient but the shader will
        /// not animate.
        ///
        /// This can also be set to `always`, which will always run the animation
        /// loop regardless of whether the terminal is focused or not. The animation
        /// loop will still only run when custom shaders are used. Note that this
        /// will use more CPU per terminal surface and can become quite expensive
        /// depending on the shader and your terminal usage.
        ///
        /// This can be changed at runtime and will affect all open terminals.
        "custom-shader-animation" => custom_shader_animation: CustomShaderAnimation = CustomShaderAnimation::True,

        /// Bell features to enable if bell support is available in your runtime. Not
        /// all features are available on all runtimes. The format of this is a list of
        /// features to enable separated by commas. If you prefix a feature with `no-`
        /// then it is disabled. If you omit a feature, its default value is used.
        ///
        /// Valid values are:
        ///
        ///  * `system`
        ///
        ///    Instruct the system to notify the user using built-in system functions.
        ///    This could result in an audiovisual effect, a notification, or something
        ///    else entirely. Changing these effects require altering system settings:
        ///    for instance under the "Sound > Alert Sound" setting in GNOME,
        ///    or the "Accessibility > System Bell" settings in KDE Plasma.
        ///
        ///    On macOS, this plays the system alert sound.
        ///
        ///  * `audio`
        ///
        ///    Play a custom sound. (Available since 1.3.0 on macOS)
        ///
        ///  * `attention` *(enabled by default)*
        ///
        ///    Request the user's attention when Ghostty is unfocused, until it has
        ///    received focus again. On macOS, this will bounce the app icon in the
        ///    dock once. On Linux, the behavior depends on the desktop environment
        ///    and/or the window manager/compositor:
        ///
        ///    - On KDE, the background of the desktop icon in the task bar would be
        ///      highlighted;
        ///
        ///    - On GNOME, you may receive a notification that, when clicked, would
        ///      bring the Ghostty window into focus;
        ///
        ///    - On Sway, the window may be decorated with a distinctly colored border;
        ///
        ///    - On other systems this may have no effect at all.
        ///
        ///  * `title` *(enabled by default)*
        ///
        ///    Prepend a bell emoji (🔔) to the title of the alerted surface until the
        ///    terminal is re-focused or interacted with (such as on keyboard input).
        ///
        ///  * `border`
        ///
        ///    Display a border around the alerted surface until the terminal is
        ///    re-focused or interacted with (such as on keyboard input).
        ///
        ///    Available since: 1.2.0 on GTK, 1.2.1 on macOS
        ///
        /// Example: `audio`, `no-audio`, `system`, `no-system`
        ///
        /// Available since: 1.2.0
        "bell-features" => bell_features: BellFeatures = BellFeatures::default(),

        /// If `audio` is an enabled bell feature, this is a path to an audio file. If
        /// the path is not absolute, it is considered relative to the directory of the
        /// configuration file that it is referenced from, or from the current working
        /// directory if this is used as a CLI flag. The path may be prefixed with `~/`
        /// to reference the user's home directory.
        ///
        /// Available since: 1.2.0 on GTK, 1.3.0 on macOS.
        "bell-audio-path" => bell_audio_path: Option<Path> = None,

        /// If `audio` is an enabled bell feature, this is the volume to play the audio
        /// file at (relative to the system volume). This is a floating point number
        /// ranging from 0.0 (silence) to 1.0 (as loud as possible). The default is 0.5.
        ///
        /// Available since: 1.2.0 on GTK, 1.3.0 on macOS.
        "bell-audio-volume" => bell_audio_volume: f64 = 0.5,

        /// Control the in-app notifications that Ghostty shows.
        ///
        /// On Linux (GTK), in-app notifications show up as toasts. Toasts appear
        /// overlaid on top of the terminal window. They are used to show information
        /// that is not critical but may be important.
        ///
        /// Possible notifications are:
        ///
        ///   - `clipboard-copy` (default: true) - Show a notification when text is copied
        ///     to the clipboard.
        ///   - `config-reload` (default: true) - Show a notification when
        ///     the configuration is reloaded.
        ///
        /// To specify a notification to enable, specify the name of the notification.
        /// To specify a notification to disable, prefix the name with `no-`. For
        /// example, to disable `clipboard-copy`, set this configuration to
        /// `no-clipboard-copy`. To enable it, set this configuration to `clipboard-copy`.
        ///
        /// Multiple notifications can be enabled or disabled by separating them
        /// with a comma.
        ///
        /// A value of "false" will disable all notifications. A value of "true" will
        /// enable all notifications.
        ///
        /// This configuration only applies to GTK.
        ///
        /// Available since: 1.1.0
        "app-notifications" => app_notifications: AppNotifications = AppNotifications::default(),

        /// If anything other than false, fullscreen mode on macOS will not use the
        /// native fullscreen, but make the window fullscreen without animations and
        /// using a new space. It's faster than the native fullscreen mode since it
        /// doesn't use animations.
        ///
        /// Important: tabs DO NOT WORK in this mode. Non-native fullscreen removes
        /// the titlebar and macOS native tabs require the titlebar. If you use tabs,
        /// you should not use this mode.
        ///
        /// If you fullscreen a window with tabs, the currently focused tab will
        /// become fullscreen while the others will remain in a separate window in
        /// the background. You can switch to that window using normal window-switching
        /// keybindings such as command+tilde. When you exit fullscreen, the window
        /// will return to the tabbed state it was in before.
        ///
        /// Allowable values are:
        ///
        ///   * `true` - Use non-native macOS fullscreen, hide the menu bar
        ///   * `false` - Use native macOS fullscreen
        ///   * `visible-menu` - Use non-native macOS fullscreen, keep the menu bar
        ///     visible
        ///   * `padded-notch` - Use non-native macOS fullscreen, hide the menu bar,
        ///     but ensure the window is not obscured by the notch on applicable
        ///     devices. The area around the notch will remain transparent currently,
        ///     but in the future we may fill it with the window background color.
        ///
        /// Changing this option at runtime works, but will only apply to the next
        /// time the window is made fullscreen. If a window is already fullscreen,
        /// it will retain the previous setting until fullscreen is exited.
        "macos-non-native-fullscreen" => macos_non_native_fullscreen: NonNativeFullscreen = NonNativeFullscreen::False,

        /// Whether the window buttons in the macOS titlebar are visible. The window
        /// buttons are the colored buttons in the upper left corner of most macOS apps,
        /// also known as the traffic lights, that allow you to close, miniaturize, and
        /// zoom the window.
        ///
        /// This setting has no effect when `window-decoration = none` or
        /// `macos-titlebar-style = hidden`, as the window buttons are always hidden in
        /// these modes.
        ///
        /// Valid values are:
        ///
        ///   * `visible` - Show the window buttons.
        ///   * `hidden` - Hide the window buttons.
        ///
        /// The default value is `visible`.
        ///
        /// Changing this option at runtime only applies to new windows.
        ///
        /// Available since: 1.2.0
        "macos-window-buttons" => macos_window_buttons: MacWindowButtons = MacWindowButtons::Visible,

        /// The style of the macOS titlebar. Available values are: "native",
        /// "transparent", "tabs", and "hidden".
        ///
        /// The "native" style uses the native macOS titlebar with zero customization.
        /// The titlebar will match your window theme (see `window-theme`).
        ///
        /// The "transparent" style is the same as "native" but the titlebar will
        /// be transparent and allow your window background color to come through.
        /// This makes a more seamless window appearance but looks a little less
        /// typical for a macOS application and may not work well with all themes.
        ///
        /// The "transparent" style will also update in real-time to dynamic
        /// changes to the window background color, e.g. via OSC 11. To make this
        /// more aesthetically pleasing, this only happens if the terminal is
        /// a window, tab, or split that borders the top of the window. This
        /// avoids a disjointed appearance where the titlebar color changes
        /// but all the topmost terminals don't match.
        ///
        /// The "tabs" style is a completely custom titlebar that integrates the
        /// tab bar into the titlebar. This titlebar always matches the background
        /// color of the terminal. There are some limitations to this style:
        /// On macOS 13 and below, saved window state will not restore tabs correctly.
        /// macOS 14 does not have this issue and any other macOS version has not
        /// been tested.
        ///
        /// The "hidden" style hides the titlebar. Unlike `window-decoration = none`,
        /// however, it does not remove the frame from the window or cause it to have
        /// squared corners. Changing to or from this option at run-time may affect
        /// existing windows in buggy ways.
        ///
        /// When "hidden", the top titlebar area can no longer be used for dragging
        /// the window. To drag the window, you can use option+click on the resizable
        /// areas of the frame to drag the window. This is a standard macOS behavior
        /// and not something Ghostty enables.
        ///
        /// The default value is "transparent". This is an opinionated choice
        /// but its one I think is the most aesthetically pleasing and works in
        /// most cases.
        ///
        /// Changing this option at runtime only applies to new windows.
        "macos-titlebar-style" => macos_titlebar_style: MacTitlebarStyle = MacTitlebarStyle::Transparent,

        /// Whether the proxy icon in the macOS titlebar is visible. The proxy icon
        /// is the icon that represents the folder of the current working directory.
        /// You can see this very clearly in the macOS built-in Terminal.app
        /// titlebar.
        ///
        /// The proxy icon is only visible with the native macOS titlebar style.
        ///
        /// Valid values are:
        ///
        ///   * `visible` - Show the proxy icon.
        ///   * `hidden` - Hide the proxy icon.
        ///
        /// The default value is `visible`.
        ///
        /// This setting can be changed at runtime and will affect all currently
        /// open windows but only after their working directory changes again.
        /// Therefore, to make this work after changing the setting, you must
        /// usually `cd` to a different directory, open a different file in an
        /// editor, etc.
        "macos-titlebar-proxy-icon" => macos_titlebar_proxy_icon: MacTitlebarProxyIcon = MacTitlebarProxyIcon::Visible,

        /// Controls the windowing behavior when dropping a file or folder
        /// onto the Ghostty icon in the macOS dock.
        ///
        /// Valid values are:
        ///
        ///   * `new-tab` - Create a new tab in the current window, or open
        ///     a new window if none exist.
        ///   * `new-window` - Create a new window unconditionally.
        ///
        /// The default value is `new-tab`.
        ///
        /// This setting is only supported on macOS and has no effect on other
        /// platforms.
        "macos-dock-drop-behavior" => macos_dock_drop_behavior: MacOSDockDropBehavior = MacOSDockDropBehavior::NewTab,

        /// macOS doesn't have a distinct "alt" key and instead has the "option"
        /// key which behaves slightly differently. On macOS by default, the
        /// option key plus a character will sometimes produce a Unicode character.
        /// For example, on US standard layouts option-b produces "∫". This may be
        /// undesirable if you want to use "option" as an "alt" key for keybindings
        /// in terminal programs or shells.
        ///
        /// This configuration lets you change the behavior so that option is treated
        /// as alt.
        ///
        /// The default behavior (unset) will depend on your active keyboard
        /// layout. If your keyboard layout is one of the keyboard layouts listed
        /// below, then the default value is "true". Otherwise, the default
        /// value is "false". Keyboard layouts with a default value of "true" are:
        ///
        ///   - U.S. Standard
        ///   - U.S. International
        ///
        /// Note that if an *Option*-sequence doesn't produce a printable character, it
        /// will be treated as *Alt* regardless of this setting. (e.g. `alt+ctrl+a`).
        ///
        /// Explicit values that can be set:
        ///
        /// If `true`, the *Option* key will be treated as *Alt*. This makes terminal
        /// sequences expecting *Alt* to work properly, but will break Unicode input
        /// sequences on macOS if you use them via the *Alt* key.
        ///
        /// You may set this to `false` to restore the macOS *Alt* key unicode
        /// sequences but this will break terminal sequences expecting *Alt* to work.
        ///
        /// The values `left` or `right` enable this for the left or right *Option*
        /// key, respectively.
        "macos-option-as-alt" => macos_option_as_alt: Option<ghostty_input::OptionAsAlt> = None,

        /// Whether to enable the macOS window shadow. The default value is true.
        /// With some window managers and window transparency settings, you may
        /// find false more visually appealing.
        "macos-window-shadow" => macos_window_shadow: bool = true,

        /// If true, the macOS icon in the dock and app switcher will be hidden. This is
        /// mainly intended for those primarily using the quick-terminal mode.
        ///
        /// Note that setting this to true means that keyboard layout changes
        /// will no longer be automatic.
        ///
        /// Control whether macOS app is excluded from the dock and app switcher,
        /// a "hidden" state. This is mainly intended for those primarily using
        /// quick-terminal mode, but is a general configuration for any use
        /// case.
        ///
        /// Available values:
        ///
        ///   * `never` - The macOS app is never hidden.
        ///   * `always` - The macOS app is always hidden.
        ///
        /// Note: When the macOS application is hidden, keyboard layout changes
        /// will no longer be automatic. This is a limitation of macOS.
        ///
        /// Available since: 1.2.0
        "macos-hidden" => macos_hidden: MacHidden = MacHidden::Never,

        /// If true, Ghostty on macOS will automatically enable the "Secure Input"
        /// feature when it detects that a password prompt is being displayed.
        ///
        /// "Secure Input" is a macOS security feature that prevents applications from
        /// reading keyboard events. This can always be enabled manually using the
        /// `Ghostty > Secure Keyboard Entry` menu item.
        ///
        /// Note that automatic password prompt detection is based on heuristics
        /// and may not always work as expected. Specifically, it does not work
        /// over SSH connections, but there may be other cases where it also
        /// doesn't work.
        ///
        /// A reason to disable this feature is if you find that it is interfering
        /// with legitimate accessibility software (or software that uses the
        /// accessibility APIs), since secure input prevents any application from
        /// reading keyboard events.
        "macos-auto-secure-input" => macos_auto_secure_input: bool = true,

        /// If true, Ghostty will show a graphical indication when secure input is
        /// enabled. This indication is generally recommended to know when secure input
        /// is enabled.
        ///
        /// Normally, secure input is only active when a password prompt is displayed
        /// or it is manually (and typically temporarily) enabled. However, if you
        /// always have secure input enabled, the indication can be distracting and
        /// you may want to disable it.
        "macos-secure-input-indication" => macos_secure_input_indication: bool = true,

        /// If true, Ghostty exposes and handles the built-in AppleScript dictionary
        /// on macOS.
        ///
        /// If false, all AppleScript interactions are disabled. This includes
        /// AppleScript commands and AppleScript object lookup for windows, tabs,
        /// and terminals.
        ///
        /// The default is true.
        "macos-applescript" => macos_applescript: bool = true,

        /// Customize the macOS app icon.
        ///
        /// This only affects the icon that appears in the dock, application
        /// switcher, etc. This does not affect the icon in Finder because
        /// that is controlled by a hardcoded value in the signed application
        /// bundle and can't be changed at runtime. For more details on what
        /// exactly is affected, see the `NSApplication.icon` Apple documentation;
        /// that is the API that is being used to set the icon.
        ///
        /// Valid values:
        ///
        ///  * `official` - Use the official Ghostty icon.
        ///  * `blueprint`, `chalkboard`, `microchip`, `glass`, `holographic`,
        ///    `paper`, `retro`, `xray` - Official variants of the Ghostty icon
        ///    hand-created by artists (no AI).
        ///  * `custom` - Use a completely custom icon. The location must be specified
        ///    using the additional `macos-custom-icon` configuration
        ///  * `custom-style` - Use the official Ghostty icon but with custom
        ///    styles applied to various layers. The custom styles must be
        ///    specified using the additional `macos-icon`-prefixed configurations.
        ///    The `macos-icon-ghost-color` and `macos-icon-screen-color`
        ///    configurations are required for this style.
        ///
        /// WARNING: The `custom-style` option is _experimental_. We may change
        /// the format of the custom styles in the future. We're still finalizing
        /// the exact layers and customization options that will be available.
        ///
        /// Other caveats:
        ///
        ///   * The icon in the update dialog will always be the official icon.
        ///     This is because the update dialog is managed through a
        ///     separate framework and cannot be customized without significant
        ///     effort.
        "macos-icon" => macos_icon: MacAppIcon = MacAppIcon::Official,

        /// The absolute path to the custom icon file.
        /// Supported formats include PNG, JPEG, and ICNS.
        ///
        /// Defaults to `~/.config/ghostty/Ghostty.icns`
        "macos-custom-icon" => macos_custom_icon: Option<String> = None,

        /// The material to use for the frame of the macOS app icon.
        ///
        /// Valid values:
        ///
        ///  * `aluminum` - A brushed aluminum frame. This is the default.
        ///  * `beige` - A classic 90's computer beige frame.
        ///  * `plastic` - A glossy, dark plastic frame.
        ///  * `chrome` - A shiny chrome frame.
        ///
        /// Note: This configuration is required when `macos-icon` is set to
        /// `custom-style`.
        "macos-icon-frame" => macos_icon_frame: MacAppIconFrame = MacAppIconFrame::Aluminum,

        /// The color of the ghost in the macOS app icon.
        ///
        /// Note: This configuration is required when `macos-icon` is set to
        /// `custom-style`.
        ///
        /// Specified as either hex (`#RRGGBB` or `RRGGBB`) or a named X11 color.
        "macos-icon-ghost-color" => macos_icon_ghost_color: Option<Color> = None,

        /// The color of the screen in the macOS app icon.
        ///
        /// The screen is a linear gradient so you can specify multiple colors
        /// that make up the gradient. Up to 64 comma-separated colors may be
        /// specified as either hex (`#RRGGBB` or `RRGGBB`) or as named X11
        /// colors. The first color is the bottom of the gradient and the last
        /// color is the top of the gradient.
        ///
        /// Note: This configuration is required when `macos-icon` is set to
        /// `custom-style`.
        "macos-icon-screen-color" => macos_icon_screen_color: Option<ColorList> = None,

        /// Whether macOS Shortcuts are allowed to control Ghostty.
        ///
        /// Ghostty exposes a number of actions that allow Shortcuts to
        /// control and interact with Ghostty. This includes creating new
        /// terminals, sending text to terminals, running commands, invoking
        /// any keybind action, etc.
        ///
        /// This is a powerful feature but can be a security risk if a malicious
        /// shortcut is able to be installed and executed. Therefore, this
        /// configuration allows you to disable this feature.
        ///
        /// Valid values are:
        ///
        /// * `ask` - Ask the user whether for permission. Ghostty will remember
        ///   this choice and never ask again. This is similar to other macOS
        ///   permissions such as microphone access, camera access, etc.
        ///
        /// * `allow` - Allow Shortcuts to control Ghostty without asking.
        ///
        /// * `deny` - Deny Shortcuts from controlling Ghostty.
        ///
        /// Available since: 1.2.0
        "macos-shortcuts" => macos_shortcuts: MacShortcuts = MacShortcuts::Ask,

        /// Put every surface (tab, split, window) into a transient `systemd` scope.
        ///
        /// This allows per-surface resource management. For example, if a shell program
        /// is using too much memory, only that shell will be killed by the oom monitor
        /// instead of the entire Ghostty process. Similarly, if a shell program is
        /// using too much CPU, only that surface will be CPU-throttled.
        ///
        /// This will cause startup times to be slower (a hundred milliseconds or so),
        /// so the default value is "single-instance." In single-instance mode, only
        /// one instance of Ghostty is running (see gtk-single-instance) so the startup
        /// time is a one-time cost. Additionally, single instance Ghostty is much
        /// more likely to have many windows, tabs, etc. so cgroup isolation is a
        /// big benefit.
        ///
        /// This feature requires `systemd`. If `systemd` is unavailable, cgroup
        /// initialization will fail. By default, this will not prevent Ghostty from
        /// working (see `linux-cgroup-hard-fail`).
        ///
        /// Changing this value and reloading the config will not affect existing
        /// surfaces.
        ///
        /// Valid values are:
        ///
        ///   * `never` - Never use cgroups.
        ///   * `always` - Always use cgroups.
        ///   * `single-instance` - Enable cgroups only for Ghostty instances launched
        ///     as single-instance applications (see gtk-single-instance).
        "linux-cgroup" => linux_cgroup: LinuxCgroup = LinuxCgroup::SingleInstance,

        /// Memory limit for any individual terminal process (tab, split, window,
        /// etc.) in bytes. If this is unset then no memory limit will be set.
        ///
        /// Note that this sets the `MemoryHigh` setting on the transient `systemd`
        /// scope, which is a soft limit. You should configure something like
        /// `systemd-oom` to handle killing processes that have too much memory
        /// pressure.
        ///
        /// Changing this value and reloading the config will not affect existing
        /// surfaces.
        ///
        /// See the `systemd.resource-control` manual page for more information:
        /// https://www.freedesktop.org/software/systemd/man/latest/systemd.resource-control.html
        "linux-cgroup-memory-limit" => linux_cgroup_memory_limit: Option<u64> = None,

        /// Number of processes limit for any individual terminal process (tab, split,
        /// window, etc.). If this is unset then no limit will be set.
        ///
        /// Note that this sets the `TasksMax` setting on the transient `systemd` scope,
        /// which is a hard limit.
        ///
        /// Changing this value and reloading the config will not affect existing
        /// surfaces.
        ///
        /// See the `systemd.resource-control` manual page for more information:
        /// https://www.freedesktop.org/software/systemd/man/latest/systemd.resource-control.html
        "linux-cgroup-processes-limit" => linux_cgroup_processes_limit: Option<u64> = None,

        /// If this is false, then creating a transient `systemd` scope (for
        /// `linux-cgroup`) will be allowed to fail and the failure is ignored. This is
        /// useful if you view cgroup isolation as a "nice to have" and not a critical
        /// resource management feature, because surface creation will not fail if
        /// `systemd` APIs fail.
        ///
        /// If this is true, then any transient `systemd` scope creation failure will
        /// cause surface creation to fail.
        ///
        /// Changing this value and reloading the config will not affect existing
        /// surfaces.
        "linux-cgroup-hard-fail" => linux_cgroup_hard_fail: bool = false,

        /// Enable or disable GTK's OpenGL debugging logs. The default is `true` for
        /// debug builds, `false` for all others.
        ///
        /// Available since: 1.1.0
        "gtk-opengl-debug" => gtk_opengl_debug: bool = cfg!(debug_assertions),

        /// If `true`, the Ghostty GTK application will run in single-instance mode:
        /// each new `ghostty` process launched will result in a new window if there is
        /// already a running process.
        ///
        /// If `false`, each new ghostty process will launch a separate application.
        ///
        /// If `detect`, Ghostty will assume true (single instance) unless one of
        /// the following scenarios is found:
        ///
        /// 1. TERM_PROGRAM environment variable is a non-empty value. In this
        /// case, we assume Ghostty is being launched from a graphical terminal
        /// session and you want a dedicated instance.
        ///
        /// 2. Any CLI arguments exist. In this case, we assume you are passing
        /// custom Ghostty configuration. Single instance mode inherits the
        /// configuration from when it was launched, so we must disable single
        /// instance to load the new configuration.
        ///
        /// If either of these scenarios is producing a false positive, you can
        /// set this configuration explicitly to the behavior you want.
        ///
        /// The pre-1.2 option `desktop` has been deprecated. Please replace
        /// this with `detect`.
        ///
        /// The default value is `detect`.
        ///
        /// Note that debug builds of Ghostty have a separate single-instance ID
        /// so you can test single instance without conflicting with release builds.
        "gtk-single-instance" => gtk_single_instance: GtkSingleInstance = GtkSingleInstance::DEFAULT,

        /// When enabled, the full GTK titlebar is displayed instead of your window
        /// manager's simple titlebar. The behavior of this option will vary with your
        /// window manager.
        ///
        /// This option does nothing when `window-decoration` is none or when running
        /// under macOS.
        "gtk-titlebar" => gtk_titlebar: bool = true,

        /// Determines the side of the screen that the GTK tab bar will stick to.
        /// Top, bottom, and hidden are supported. The default is top.
        ///
        /// When `hidden` is set, a tab button displaying the number of tabs will appear
        /// in the title bar. It has the ability to open a tab overview for displaying
        /// tabs. Alternatively, you can use the `toggle_tab_overview` action in a
        /// keybind if your window doesn't have a title bar, or you can switch tabs
        /// with keybinds.
        "gtk-tabs-location" => gtk_tabs_location: GtkTabsLocation = GtkTabsLocation::Top,

        /// If this is `true`, the titlebar will be hidden when the window is maximized,
        /// and shown when the titlebar is unmaximized. GTK only.
        ///
        /// Available since: 1.1.0
        "gtk-titlebar-hide-when-maximized" => gtk_titlebar_hide_when_maximized: bool = false,

        /// Determines the appearance of the top and bottom bars tab bar.
        ///
        /// Valid values are:
        ///
        ///  * `flat` - Top and bottom bars are flat with the terminal window.
        ///  * `raised` - Top and bottom bars cast a shadow on the terminal area.
        ///  * `raised-border` - Similar to `raised` but the shadow is replaced with a
        ///    more subtle border.
        "gtk-toolbar-style" => gtk_toolbar_style: GtkToolbarStyle = GtkToolbarStyle::Raised,

        /// The style of the GTK titlebar. Available values are `native` and `tabs`.
        ///
        /// The `native` titlebar style is a traditional titlebar with a title, a few
        /// buttons and window controls. A separate tab bar will show up below the
        /// titlebar if you have multiple tabs open in the window.
        ///
        /// The `tabs` titlebar merges the tab bar and the traditional titlebar.
        /// This frees up vertical space on your screen if you use multiple tabs. One
        /// limitation of the `tabs` titlebar is that you cannot drag the titlebar
        /// by the titles any longer (as they are tab titles now). Other areas of the
        /// `tabs` title bar can be used to drag the window around.
        ///
        /// The default style is `native`.
        "gtk-titlebar-style" => gtk_titlebar_style: GtkTitlebarStyle = GtkTitlebarStyle::Native,

        /// If `true` (default), then the Ghostty GTK tabs will be "wide." Wide tabs
        /// are the new typical Gnome style where tabs fill their available space.
        /// If you set this to `false` then tabs will only take up space they need,
        /// which is the old style.
        "gtk-wide-tabs" => gtk_wide_tabs: bool = true,

        /// Custom CSS files to be loaded.
        ///
        /// GTK CSS documentation can be found at the following links:
        ///
        ///   * https://docs.gtk.org/gtk4/css-overview.html - An overview of GTK CSS.
        ///   * https://docs.gtk.org/gtk4/css-properties.html - A comprehensive list
        ///     of supported CSS properties.
        ///
        /// Launch Ghostty with `env GTK_DEBUG=interactive ghostty` to tweak Ghostty's
        /// CSS in real time using the GTK Inspector. Errors in your CSS files would
        /// also be reported in the terminal you started Ghostty from. See
        /// https://developer.gnome.org/documentation/tools/inspector.html for more
        /// information about the GTK Inspector.
        ///
        /// This configuration can be repeated multiple times to load multiple files.
        /// Prepend a ? character to the file path to suppress errors if the file does
        /// not exist. If you want to include a file that begins with a literal ?
        /// character, surround the file path in double quotes (").
        /// The file size limit for a single stylesheet is 5MiB.
        ///
        /// Available since: 1.1.0
        "gtk-custom-css" => gtk_custom_css: RepeatablePath = RepeatablePath::default(),

        /// If `true` (default), applications running in the terminal can show desktop
        /// notifications using certain escape sequences such as OSC 9 or OSC 777.
        "desktop-notifications" => desktop_notifications: bool = true,

        /// If `true` (default), applications running in the terminal can show
        /// graphical progress bars using the ConEmu OSC 9;4 escape sequence.
        /// If `false`, progress bar sequences are silently ignored.
        "progress-style" => progress_style: bool = true,

        /// Modifies the color used for bold text in the terminal.
        ///
        /// This can be set to a specific color, using the same format as
        /// `background` or `foreground` (e.g. `#RRGGBB` but other formats
        /// are also supported; see the aforementioned documentation). If a
        /// specific color is set, this color will always be used for the default
        /// bold text color. It will set the rest of the bold colors to `bright`.
        ///
        /// This can also be set to `bright`, which uses the bright color palette
        /// for bold text. For example, if the text is red, then the bold will
        /// use the bright red color. The terminal palette is set with `palette`
        /// but can also be overridden by the terminal application itself using
        /// escape sequences such as OSC 4. (Since Ghostty 1.2.0, the previous
        /// configuration `bold-is-bright` is deprecated and replaced by this
        /// usage).
        ///
        /// Available since Ghostty 1.2.0.
        "bold-color" => bold_color: Option<BoldColor> = None,

        /// The opacity level (opposite of transparency) of the faint text. A value of
        /// 1 is fully opaque and a value of 0 is fully transparent. A value less than 0
        /// or greater than 1 will be clamped to the nearest valid value.
        ///
        /// Available since Ghostty 1.2.0.
        "faint-opacity" => faint_opacity: f64 = 0.5,

        /// This will be used to set the `TERM` environment variable.
        /// HACK: We set this with an `xterm` prefix because vim uses that to enable key
        /// protocols (specifically this will enable `modifyOtherKeys`), among other
        /// features. An option exists in vim to modify this: `:set
        /// keyprotocol=ghostty:kitty`, however a bug in the implementation prevents it
        /// from working properly. https://github.com/vim/vim/pull/13211 fixes this.
        "term" => term: String = "xterm-ghostty".to_string(),

        /// String to send when we receive `ENQ` (`0x05`) from the command that we are
        /// running. Defaults to an empty string if not set.
        "enquiry-response" => enquiry_response: String = String::new(),

        /// Configures the low-level API to use for async IO, eventing, etc.
        ///
        /// Most users should leave this set to `auto`. This will automatically detect
        /// scenarios where APIs may not be available (for example `io_uring` on
        /// certain hardened kernels) and fall back to a different API. However, if
        /// you want to force a specific backend for any reason, you can set this
        /// here.
        ///
        /// Based on various benchmarks, we haven't found a statistically significant
        /// difference between the backends with regards to memory, CPU, or latency.
        /// The choice of backend is more about compatibility and features.
        ///
        /// Available options:
        ///
        ///   * `auto` - Automatically choose the best backend for the platform
        ///     based on available options.
        ///   * `epoll` - Use the `epoll` API
        ///   * `io_uring` - Use the `io_uring` API
        ///
        /// If the selected backend is not available on the platform, Ghostty will
        /// fall back to an automatically chosen backend that is available.
        ///
        /// Changing this value requires a full application restart to take effect.
        ///
        /// This is only supported on Linux, since this is the only platform
        /// where we have multiple options. On macOS, we always use `kqueue`.
        ///
        /// Available since: 1.2.0
        "async-backend" => async_backend: AsyncBackend = AsyncBackend::Auto,

        /// Control the auto-update functionality of Ghostty. This is only supported
        /// on macOS currently, since Linux builds are distributed via package
        /// managers that are not centrally controlled by Ghostty.
        ///
        /// Checking or downloading an update does not send any information to
        /// the project beyond standard network information mandated by the
        /// underlying protocols. To put it another way: Ghostty doesn't explicitly
        /// add any tracking to the update process. The update process works by
        /// downloading information about the latest version and comparing it
        /// client-side to the current version.
        ///
        /// Valid values are:
        ///
        ///  * `off` - Disable auto-updates.
        ///  * `check` - Check for updates and notify the user if an update is
        ///    available, but do not automatically download or install the update.
        ///  * `download` - Check for updates, automatically download the update,
        ///    notify the user, but do not automatically install the update.
        ///
        /// If unset, we defer to Sparkle's default behavior, which respects the
        /// preference stored in the standard user defaults (`defaults(1)`).
        ///
        /// Changing this value at runtime works after a small delay.
        "auto-update" => auto_update: Option<AutoUpdate> = None,

        /// The release channel to use for auto-updates.
        ///
        /// The default value of this matches the release channel of the currently
        /// running Ghostty version. If you download a pre-release version of Ghostty
        /// then this will be set to `tip` and you will receive pre-release updates.
        /// If you download a stable version of Ghostty then this will be set to
        /// `stable` and you will receive stable updates.
        ///
        /// Valid values are:
        ///
        ///  * `stable` - Stable, tagged releases such as "1.0.0".
        ///  * `tip` - Pre-release versions generated from each commit to the
        ///    main branch. This is the version that was in use during private
        ///    beta testing by thousands of people. It is generally stable but
        ///    will likely have more bugs than the stable channel.
        ///
        /// Changing this configuration requires a full restart of
        /// Ghostty to take effect.
        ///
        /// This only works on macOS since only macOS has an auto-update feature.
        "auto-update-channel" => auto_update_channel: Option<ReleaseChannel> = None,
    }
    extra {
        /// List of diagnostics that were generated during the loading of
        /// the configuration.
        diagnostics: DiagnosticList = DiagnosticList::default(),
        /// The conditional truths for the configuration. This is used to
        /// determine if a conditional configuration matches or not.
        conditional_state: conditional::State = conditional::State::default(),
        /// The conditional keys that are used at any point during the
        /// configuration loading.
        conditional_set: HashSet<conditional::Key> = HashSet::new(),
        /// The steps we can use to reload the configuration after it has
        /// been loaded without reopening the files.
        replay_steps: Vec<ReplayStep> = Vec::new(),
        /// Set to true if Ghostty was executed as xdg-terminal-exec on Linux.
        xdg_terminal_exec: bool = false,
    }
}

/// Errors from loading configuration.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Arg(#[from] ArgError),
    #[error("{0}")]
    Other(String),
}

/// This is used to "replay" the configuration. See `load_theme` for details.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplayStep {
    /// An argument to parse as if it came from the CLI or file.
    Arg(String),

    /// A base path to expand relative paths against.
    Expand(String),

    /// A conditional argument. This arg is parsed only if all
    /// conditions match (an "AND"). An "OR" can be achieved by
    /// having multiple conditional arg entries.
    ConditionalArg {
        conditions: Vec<Conditional>,
        arg: String,
    },

    /// A diagnostic to be added to the new configuration when
    /// replayed. This should only be used for diagnostics that won't
    /// be reproduced during playback. For example, `config-file`
    /// errors are not reloaded so they should be added here.
    ///
    /// Diagnostics cannot be conditional. They are always present
    /// even if the conditionals don't match. This helps users find
    /// errors in their configuration.
    Diagnostic(Diagnostic),

    /// The start of a "-e" argument. This marks the end of
    /// traditional configuration and the beginning of the
    /// "-e" initial command magic. This is separate from "arg"
    /// because there are some behaviors unique to this (i.e.
    /// we want to keep this at the end for config-file).
    ///
    /// Note: when "-e" is used, ONLY this is present and
    /// not an additional "arg" with "-e" value.
    DashE,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionalFileAction {
    Loaded,
    NotFound,
    Error,
}

impl Default for Config {
    fn default() -> Self {
        Config::new_default()
    }
}

impl Config {
    /// Build the default configuration (Zig: `Config.default`).
    #[must_use]
    pub fn new_default() -> Config {
        // Build up our basic config
        let mut result = Config::raw_default();

        // Add our default keybindings
        result.keybind.init();

        // Add our default command palette entries
        result.command_palette_entry.init();

        // Add our default link for URL detection
        result.link.links.push(ghostty_input::Link {
            regex: crate::url::REGEX.to_string(),
            action: ghostty_input::link::Action::Open,
            highlight: ghostty_input::link::Highlight::HoverMods(ghostty_input::ctrl_or_super(
                ghostty_input::Mods::default(),
            )),
        });

        result
    }

    /// Load the configuration according to the default rules:
    ///
    ///   1. Defaults
    ///   2. XDG config dir
    ///   3. "Application Support" directory (macOS only)
    ///   4. CLI flags
    ///   5. Recursively defined configuration files
    pub fn load() -> Result<Config, Error> {
        let mut result = Config::new_default();

        // If we have a configuration file in our home directory, parse that first.
        result.load_default_files()?;

        // Parse the config from the CLI args.
        result.load_cli_args()?;

        // Parse the config files that were added from our file and CLI args.
        result.load_recursive_files()?;
        result.finalize()?;

        Ok(result)
    }

    /// Load configuration from an iterator that yields values that look like
    /// command-line arguments, i.e. `--key=value`.
    pub fn load_iter(&mut self, iter: &mut dyn ArgIter) -> Result<(), ArgError> {
        cli_args::parse(self, iter)
    }

    /// Load configuration from the target config file at `path`.
    ///
    /// `path` must be resolved and absolute.
    pub fn load_file(&mut self, path: &str) -> Result<(), Error> {
        assert!(std::path::Path::new(path).is_absolute());
        let file = match crate::file_load::open(path) {
            Ok(f) => f,
            Err(crate::file_load::OpenFileError::NotAFile) => {
                log::warn!("config-file {path}: not reading because it is not a file");
                return Ok(());
            }
            Err(crate::file_load::OpenFileError::FileNotFound) => {
                return Err(Error::Io(std::io::ErrorKind::NotFound.into()));
            }
            Err(e) => return Err(Error::Other(e.to_string())),
        };

        self.load_fs_file(file, path)
    }

    /// Load config from the given File.
    fn load_fs_file(&mut self, file: std::fs::File, path: &str) -> Result<(), Error> {
        log::info!("reading configuration file path={path}");
        let mut reader = std::io::BufReader::with_capacity(2048, file);
        self.load_reader(&mut reader, path)
    }

    /// Load config from the given Reader.
    fn load_reader<R: std::io::BufRead>(
        &mut self,
        reader: &mut R,
        path: &str,
    ) -> Result<(), Error> {
        // If the file starts with a UTF-8 byte order mark, skip it.
        // https://en.wikipedia.org/wiki/Byte_order_mark#UTF-8
        if let Ok(buf) = reader.fill_buf() {
            if buf.starts_with(&[0xef, 0xbb, 0xbf]) {
                log::info!("skipping UTF-8 byte order mark");
                reader.consume(3);
            }
        }
        let mut iter = cli_args::LineIterator::with_path(reader, path);
        self.load_iter(&mut iter)?;
        let dir = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.expand_paths(&dir)?;
        Ok(())
    }

    /// Load optional configuration file from `path`. All errors are ignored.
    ///
    /// Returns the action that was taken.
    pub fn load_optional_file(&mut self, path: &str) -> OptionalFileAction {
        match self.load_file(path) {
            Ok(()) => OptionalFileAction::Loaded,
            Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                OptionalFileAction::NotFound
            }
            Err(err) => {
                log::warn!("error reading optional config file, not loading err={err} path={path}");
                OptionalFileAction::Error
            }
        }
    }

    fn write_config_template(path: &str) -> std::io::Result<()> {
        log::info!("creating template config file: path={path}");
        if let Some(dir) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(dir)?;
        }
        let template = include_str!("config-template").replace("{[path]s}", path);
        std::fs::write(path, template)
    }

    /// Load configurations from the default configuration files. The default
    /// configuration file is at `$XDG_CONFIG_HOME/ghostty/config.ghostty`.
    ///
    /// The legacy `config` file (without extension) is first loaded,
    /// then `config.ghostty`.
    pub fn load_default_files(&mut self) -> Result<(), Error> {
        // Load XDG first
        let legacy_xdg_path =
            crate::file_load::legacy_default_xdg_path().map_err(|e| Error::Other(e.to_string()))?;
        let xdg_path =
            crate::file_load::default_xdg_path().map_err(|e| Error::Other(e.to_string()))?;
        let xdg_loaded = {
            let legacy_xdg_action = self.load_optional_file(&legacy_xdg_path);
            let xdg_action = self.load_optional_file(&xdg_path);
            if xdg_action != OptionalFileAction::NotFound
                && legacy_xdg_action != OptionalFileAction::NotFound
            {
                log::warn!("both config files `{legacy_xdg_path}` and `{xdg_path}` exist.");
                log::warn!("loading them both in that order");
                true
            } else {
                xdg_action != OptionalFileAction::NotFound
                    || legacy_xdg_action != OptionalFileAction::NotFound
            }
        };

        if !xdg_loaded {
            if let Err(err) = Self::write_config_template(&xdg_path) {
                log::warn!("error creating template config file err={err}");
            }
        }
        Ok(())
    }

    /// Load and parse the CLI args.
    pub fn load_cli_args(&mut self) -> Result<(), Error> {
        let argv: Vec<std::ffi::OsString> = std::env::args_os().collect();

        // Fast-path if we are Linux/BSD and have no args.
        if argv.len() <= 1 {
            return Ok(());
        }

        // On Linux, we have a special case where if the executing
        // program is "xdg-terminal-exec" then we treat all CLI
        // args as if they are a command to execute.
        //
        // In this mode, we also behave slightly differently:
        //
        //   - The initial window title is set to the full command. This
        //     can be used with window managers to modify positioning,
        //     styling, etc. based on the command.
        //
        // See: https://github.com/Vladimir-csp/xdg-terminal-exec
        {
            use std::os::unix::ffi::OsStrExt;
            let cargs: Vec<std::ffi::CString> = argv
                .iter()
                .filter_map(|a| std::ffi::CString::new(a.as_bytes()).ok())
                .collect();
            let crefs: Vec<&std::ffi::CStr> = cargs.iter().map(|c| c.as_c_str()).collect();
            if let Some(args) = ghostty_os::xdg::parse_terminal_exec(&crefs) {
                // First, we add an artificial "-e" so that if we
                // replay the inputs to rebuild the config (i.e. if
                // a theme is set) then we will get the same behavior.
                self.replay_steps.push(ReplayStep::DashE);

                // Next, take all remaining args and use that to build up
                // a command to execute.
                let mut builder: Vec<String> = Vec::new();
                for arg in args {
                    let copy = arg.to_string_lossy().into_owned();
                    self.replay_steps.push(ReplayStep::Arg(copy.clone()));
                    builder.push(copy);
                }

                self.xdg_terminal_exec = true;
                self.initial_command = Some(Command::Direct(builder));
                return Ok(());
            }
        }

        // We set config-default-files to true here because this
        // should always be reset so we can detect if it is set
        // in the CLI since it is documented as having no affect
        // from files.
        self.config_default_files = true;

        // Keep track of the replay steps up to this point so we
        // can replay if we are discarding the default files.
        let replay_len_start = self.replay_steps.len();

        // font-family settings set via the CLI overwrite any prior values
        // rather than append. This avoids a UX oddity where you have to
        // specify `font-family=""` to clear the font families.
        self.font_family.overwrite_next = true;
        self.font_family_bold.overwrite_next = true;
        self.font_family_italic.overwrite_next = true;
        self.font_family_bold_italic.overwrite_next = true;

        // Initialize our CLI iterator.
        let mut iter = cli_args::args_iterator();
        let res = self.load_iter(&mut iter);

        // If we are not loading the default files, then we need to
        // replay the steps up to this point so that we can rebuild
        // the config without it.
        if res.is_ok() && !self.config_default_files {
            let replay_len_end = self.replay_steps.len();
            if replay_len_end != replay_len_start {
                log::info!(
                    "config-default-files unset, discarding configuration from default files"
                );

                let mut new_config = self.clone_empty();
                let steps = self.replay_steps[replay_len_start..replay_len_end].to_vec();
                new_config.load_replay(&steps)?;
                *self = new_config;
            }
        }

        self.font_family.overwrite_next = false;
        self.font_family_bold.overwrite_next = false;
        self.font_family_italic.overwrite_next = false;
        self.font_family_bold_italic.overwrite_next = false;
        res?;

        // Any paths referenced from the CLI are relative to the current working
        // directory.
        let cwd = std::fs::canonicalize(".")?;
        self.expand_paths(&cwd.to_string_lossy())?;
        Ok(())
    }

    /// Load and parse the config files that were added in the "config-file" key.
    pub fn load_recursive_files(&mut self) -> Result<(), Error> {
        if self.config_file.value.is_empty() {
            return Ok(());
        }

        // Keeps track of loaded files to prevent cycles.
        let mut loaded: HashSet<String> = HashSet::new();

        // We need to insert all of our loaded config-file values
        // PRIOR to the "-e" in our replay steps, since everything
        // after "-e" becomes an "initial-command". To do this, we
        // dupe the values if we find it.
        let mut replay_suffix: Vec<ReplayStep> = Vec::new();
        if let Some(i) = self
            .replay_steps
            .iter()
            .position(|s| matches!(s, ReplayStep::DashE))
        {
            replay_suffix = self.replay_steps.split_off(i);
        }

        // We must use a while below and not a for(items) because we
        // may add items to the list while iterating for recursive
        // config-file entries.
        let mut i = 0;
        while i < self.config_file.value.len() {
            let (path, optional) = match &self.config_file.value[i] {
                Path::Optional(path) => (path.clone(), true),
                Path::Required(path) => (path.clone(), false),
            };
            i += 1;

            // Error paths
            if path.is_empty() {
                continue;
            }

            // All paths should already be absolute at this point because
            // they're fixed up after each load.
            assert!(std::path::Path::new(&path).is_absolute());

            // We must only load a unique file once
            if !loaded.insert(path.clone()) {
                let diag = Diagnostic {
                    message: format!("config-file {path}: cycle detected"),
                    ..Diagnostic::default()
                };
                self.diagnostics.append(diag.clone());
                self.replay_steps.push(ReplayStep::Diagnostic(diag));
                continue;
            }

            let file = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(err) => {
                    if err.kind() != std::io::ErrorKind::NotFound || !optional {
                        let diag = Diagnostic {
                            message: format!(
                                "error opening config-file {path}: {}",
                                zig_io_error_name(&err)
                            ),
                            ..Diagnostic::default()
                        };
                        self.diagnostics.append(diag.clone());
                        self.replay_steps.push(ReplayStep::Diagnostic(diag));
                    }
                    continue;
                }
            };

            let meta = file.metadata()?;
            if !meta.is_file() {
                let diag = Diagnostic {
                    message: format!(
                        "config-file {path}: not reading because file type is {}",
                        file_kind_name(&meta)
                    ),
                    ..Diagnostic::default()
                };
                self.diagnostics.append(diag.clone());
                self.replay_steps.push(ReplayStep::Diagnostic(diag));
                continue;
            }

            self.load_fs_file(file, &path)?;
        }

        // If we have a suffix, add that back.
        self.replay_steps.extend(replay_suffix);
        Ok(())
    }

    /// Change the state of conditionals and reload the configuration
    /// based on the new state. This returns a new configuration based
    /// on the new state.
    ///
    /// This returns `None` if the conditional state would result in no changes
    /// to the configuration. In this case, the caller can continue to use
    /// the existing configuration or clone if they want a copy.
    ///
    /// This doesn't re-read any files, it just re-applies the same
    /// configuration with the new conditional state. Importantly, this means
    /// that if you change the conditional state and the user in the interim
    /// deleted a file that was referenced in the configuration, then the
    /// configuration can still be reloaded.
    pub fn change_conditional_state(
        &self,
        new: conditional::State,
    ) -> Result<Option<Config>, Error> {
        // If the conditional state between the old and new is the same,
        // then we don't need to do anything.
        let relevant = (self.conditional_set.contains(&conditional::Key::Theme)
            && self.conditional_state.theme != new.theme)
            || (self.conditional_set.contains(&conditional::Key::Os)
                && self.conditional_state.os != new.os);
        if !relevant {
            return Ok(None);
        }

        // Create our new configuration
        let mut new_config = self.clone_empty();

        // Set our conditional state so the replay below can use it
        new_config.conditional_state = new;

        // Replay all of our steps to rebuild the configuration
        new_config.load_replay(&self.replay_steps)?;
        new_config.finalize()?;

        Ok(Some(new_config))
    }

    /// Replay steps into this config (Zig: `Replay.iterator` + `loadIter`).
    /// This handles intermediate steps such as expanding relative paths.
    fn load_replay(&mut self, steps: &[ReplayStep]) -> Result<(), ArgError> {
        let mut i = 0;
        while i < steps.len() {
            let step = &steps[i];
            i += 1;
            match step {
                ReplayStep::Expand(base) => {
                    // This shouldn't happen because to reach this step
                    // means that it succeeded before. Its possible since
                    // expanding paths is a side effect process that the
                    // world state changed and we can't expand anymore.
                    // In that really unfortunate case, we log a warning.
                    if let Err(err) = self.expand_paths(base) {
                        log::warn!("error expanding paths err={err}");
                    }
                }

                ReplayStep::Diagnostic(diag) => self.diagnostics.append(diag.clone()),

                ReplayStep::ConditionalArg { conditions, arg } => {
                    // All conditions must match.
                    if conditions.iter().all(|c| self.conditional_state.match_(c)) {
                        self.load_one_arg(arg)?;
                    }
                }

                ReplayStep::Arg(arg) => self.load_one_arg(arg)?,

                ReplayStep::DashE => {
                    // The "-e" hook consumes every remaining argument.
                    let mut args = vec!["-e".to_string()];
                    while i < steps.len() {
                        match &steps[i] {
                            ReplayStep::Expand(base) => {
                                if let Err(err) = self.expand_paths(base) {
                                    log::warn!("error expanding paths err={err}");
                                }
                            }
                            ReplayStep::Diagnostic(diag) => {
                                self.diagnostics.append(diag.clone());
                            }
                            ReplayStep::ConditionalArg { conditions, arg } => {
                                if conditions.iter().all(|c| self.conditional_state.match_(c)) {
                                    args.push(arg.clone());
                                }
                            }
                            ReplayStep::Arg(arg) => args.push(arg.clone()),
                            ReplayStep::DashE => args.push("-e".to_string()),
                        }
                        i += 1;
                    }
                    let mut it = cli_args::VecIterator::new(args);
                    self.load_iter(&mut it)?;
                }
            }
        }
        Ok(())
    }

    fn load_one_arg(&mut self, arg: &str) -> Result<(), ArgError> {
        let mut it = cli_args::VecIterator::new(vec![arg.to_string()]);
        self.load_iter(&mut it)
    }

    /// Expand the relative paths in config-files to be absolute paths
    /// relative to the base directory.
    fn expand_paths(&mut self, base: &str) -> Result<(), Error> {
        // Keep track of this step for replays
        self.replay_steps.push(ReplayStep::Expand(base.to_string()));

        // Expand all of our paths
        if let Some(path) = &mut self.background_image {
            path.expand(base, &mut self.diagnostics)?;
        }
        if let Some(path) = &mut self.bell_audio_path {
            path.expand(base, &mut self.diagnostics)?;
        }
        self.config_file.expand(base, &mut self.diagnostics)?;
        self.custom_shader.expand(base, &mut self.diagnostics)?;
        self.gtk_custom_css.expand(base, &mut self.diagnostics)?;
        Ok(())
    }

    fn load_theme(&mut self, theme: &Theme) -> Result<(), Error> {
        // Load the correct theme depending on the conditional state.
        // Dark/light themes were programmed prior to conditional configuration
        // so when we introduce that we probably want to replace this.
        let name: &str = match self.conditional_state.theme {
            conditional::Theme::Light => &theme.light,
            conditional::Theme::Dark => &theme.dark,
        };

        // Find our theme file and open it. See the open function for details.
        let resources_dir = ghostty_os::resources_dir().ok();
        let Some(themefile) = crate::theme::open(
            name,
            resources_dir.as_ref().and_then(|r| r.app()),
            &mut self.diagnostics,
        ) else {
            return Ok(());
        };
        let path = themefile.path;
        let file = themefile.file;

        // From this point onwards, we load the theme and do a bit of a dance
        // to achieve two goals:
        //
        //   (1) We want the theme to be loaded and our existing config to
        //       override the theme. So we need to load the theme and apply
        //       our config on top of it.
        //
        //   (2) We want to free existing memory that we aren't using anymore
        //       as a result of reloading the configuration.

        // Load into a new configuration so that we can free the existing memory.
        let mut new_config = self.clone_empty();

        // Load our theme
        let mut reader = std::io::BufReader::with_capacity(2048, file);
        let mut iter = cli_args::LineIterator::with_path(&mut reader, path);
        new_config.load_iter(&mut iter)?;

        // Setup our replay to be conditional.
        let theme_name = self.conditional_state.theme.as_str();
        for item in &mut new_config.replay_steps {
            match item {
                ReplayStep::Expand(_) | ReplayStep::Diagnostic(_) => {}

                // If we see "-e" then we do NOT make the following arguments
                // conditional since they are supposed to be part of the
                // initial command.
                ReplayStep::DashE => break,

                // Change our arg to be conditional on our theme.
                ReplayStep::Arg(v) => {
                    *item = ReplayStep::ConditionalArg {
                        conditions: vec![Conditional {
                            key: conditional::Key::Theme,
                            op: conditional::Op::Eq,
                            value: theme_name.to_string(),
                        }],
                        arg: std::mem::take(v),
                    };
                }

                ReplayStep::ConditionalArg { conditions, .. } => {
                    conditions.insert(
                        0,
                        Conditional {
                            key: conditional::Key::Theme,
                            op: conditional::Op::Eq,
                            value: theme_name.to_string(),
                        },
                    );
                }
            }
        }

        // Replay our previous inputs so that we can override values
        // from the theme.
        new_config.load_replay(&self.replay_steps)?;

        // Success, swap our new config in and free the old.
        *self = new_config;
        Ok(())
    }

    /// Call this once after you are done setting configuration. This
    /// is idempotent but will waste memory if called multiple times.
    pub fn finalize(&mut self) -> Result<(), Error> {
        // We always load the theme first because it may set other fields
        // in our config.
        if let Some(theme) = self.theme.clone() {
            let different = theme.light != theme.dark;

            // Warning: load_theme will replace our existing config with a
            // new one, so all state from self prior to this point is replaced.
            self.load_theme(&theme)?;

            // If we have different light vs dark mode themes, disable
            // window-theme = auto since that breaks it.
            if different {
                // This setting doesn't make sense with different light/dark themes
                // because it'll force the theme based on the Ghostty theme.
                if self.window_theme == WindowTheme::Auto {
                    self.window_theme = WindowTheme::System;
                }

                // Mark that we use a conditional theme
                self.conditional_set.insert(conditional::Key::Theme);
            }
        }

        // Used for a variety of defaults. See the function docs as well the
        // specific variable use sites for more details.
        let probable_cli = probable_cli_environment();

        // If we have a font-family set and don't set the others, default
        // the others to the font family. This way, if someone does
        // --font-family=foo, then we try to get the stylized versions of
        // "foo" as well.
        if self.font_family.count() > 0 {
            if self.font_family_bold.count() == 0 {
                self.font_family_bold = self.font_family.clone();
            }
            if self.font_family_italic.count() == 0 {
                self.font_family_italic = self.font_family.clone();
            }
            if self.font_family_bold_italic.count() == 0 {
                self.font_family_bold_italic = self.font_family.clone();
            }
        }

        // Prevent setting TERM to an empty string
        if self.term.is_empty() {
            // HACK: See comment above at definition
            self.term = "xterm-ghostty".to_string();
        }

        // The default for the working directory depends on the system.
        let mut wd: WorkingDirectory = self.working_directory.clone().unwrap_or(if probable_cli {
            WorkingDirectory::Inherit
        } else {
            WorkingDirectory::Home
        });

        // If we are missing either a command or home directory, we need
        // to look up defaults which is kind of expensive. We only do this
        // on desktop (and not in tests, like the Zig `builtin.is_test`).
        if !cfg!(test) && (self.command.is_none() || wd == WorkingDirectory::Home) {
            self.lookup_shell_defaults(probable_cli, &mut wd)?;
        }
        wd.finalize();
        self.working_directory = Some(wd);

        // Apprt-specific defaults (GTK)
        match self.gtk_single_instance {
            GtkSingleInstance::True | GtkSingleInstance::False => {}

            // For detection, we assume single instance unless we're
            // in a CLI environment, then we disable single instance.
            GtkSingleInstance::Detect => {
                self.gtk_single_instance = if probable_cli {
                    GtkSingleInstance::False
                } else {
                    GtkSingleInstance::True
                };
            }
        }

        // Default our click interval
        if self.click_repeat_interval == 0 && !cfg!(test) {
            self.click_repeat_interval = ghostty_os::click_interval().unwrap_or(500);
        }

        // Clamp our mouse scroll multiplier
        self.mouse_scroll_multiplier.precision =
            self.mouse_scroll_multiplier.precision.clamp(0.01, 10_000.0);
        self.mouse_scroll_multiplier.discrete =
            self.mouse_scroll_multiplier.discrete.clamp(0.01, 10_000.0);

        // Clamp our split opacity
        self.unfocused_split_opacity = self.unfocused_split_opacity.clamp(0.15, 1.0);

        // Clamp our contrast
        self.minimum_contrast = self.minimum_contrast.clamp(1.0, 21.0);

        // Minimum window size
        if self.window_width > 0 {
            self.window_width = self.window_width.max(10);
        }
        if self.window_height > 0 {
            self.window_height = self.window_height.max(4);
        }

        // If URLs are disabled, cut off the first link. The first link is
        // always the URL matcher.
        if !self.link_url && !self.link.links.is_empty() {
            self.link.links.remove(0);
        }

        // We warn when the quit-after-last-window-closed-delay is set to a very
        // short value because it can cause Ghostty to quit before the first
        // window is even shown.
        if let Some(duration) = &self.quit_after_last_window_closed_delay {
            if duration.duration < 5 * NS_PER_S {
                log::warn!(
                    "quit-after-last-window-closed-delay is set to a very short value ({duration}), which might cause problems"
                );
            }
        }

        // We can't set this as a struct default because our config is
        // loaded in environments where a build config isn't available.
        if self.auto_update_channel.is_none() {
            self.auto_update_channel = Some(ReleaseChannel::Stable);
        }

        self.faint_opacity = self.faint_opacity.clamp(0.0, 1.0);

        // Finalize key remapping set for efficient lookups
        self.key_remap.finalize();
        Ok(())
    }

    /// Look up the default shell / working directory from the environment
    /// and passwd database (the expensive part of `finalize`).
    fn lookup_shell_defaults(
        &mut self,
        probable_cli: bool,
        wd: &mut WorkingDirectory,
    ) -> Result<(), Error> {
        // First look up the command using the SHELL env var if needed.
        // We don't do this in flatpak because SHELL in Flatpak is always
        // set to /bin/sh.
        if let Some(cmd) = &self.command {
            log::info!("shell src=config value={cmd:?}");
        } else if !ghostty_os::is_flatpak() && probable_cli {
            // If we were launched from the desktop, our SHELL env var
            // will represent our SHELL at login time. We only want to
            // read from SHELL if we're in a probable CLI environment.
            if let Ok(value) = std::env::var("SHELL") {
                log::info!("default shell source=env value={value}");
                self.command = Some(Command::Shell(value));

                // If we don't need the working directory, then we can exit now.
                if *wd != WorkingDirectory::Home {
                    return Ok(());
                }
            }
        }

        // We need the passwd entry for the remainder
        let pw = ghostty_os::passwd::get()?;
        if self.command.is_none() {
            if let Some(sh) = pw.shell {
                log::info!("default shell src=passwd value={sh}");
                self.command = Some(Command::Shell(sh));
            }
        }

        if *wd == WorkingDirectory::Home {
            if let Some(home) = pw.home {
                log::info!("default working directory src=passwd value={home}");
                *wd = WorkingDirectory::Path(home);
            } else {
                *wd = WorkingDirectory::Inherit;
            }
        }

        if self.command.is_none() {
            log::warn!("no default shell found, will default to using sh");
        }
        Ok(())
    }

    /// Add a diagnostic message to the config with the given string.
    /// This is always added with a location of "none".
    pub fn add_diagnostic(&mut self, message: impl Into<String>) {
        self.diagnostics.append(Diagnostic {
            message: message.into(),
            ..Diagnostic::default()
        });
    }

    /// Create a copy of the metadata of this configuration but without
    /// the actual values. Metadata includes conditional state.
    #[must_use]
    pub fn clone_empty(&self) -> Config {
        let mut result = Config::new_default();
        result.conditional_state = self.conditional_state;
        result
    }

    /// Returns an iterator that goes through each changed field from
    /// old to new. The order of old or new do not matter.
    #[must_use]
    pub fn change_iterator<'a>(&'a self, new: &'a Config) -> ChangeIterator<'a> {
        ChangeIterator {
            old: self,
            new,
            i: 0,
        }
    }

    /// Returns true if the given key has changed from old to new.
    #[must_use]
    pub fn changed(&self, new: &Config, key: Key) -> bool {
        match Config::FIELDS.iter().find(|f| f.name == key.0) {
            Some(field) => !(field.eq)(self, new),
            None => false,
        }
    }
}

/// This yields a key for every changed field between old and new.
pub struct ChangeIterator<'a> {
    old: &'a Config,
    new: &'a Config,
    i: usize,
}

impl Iterator for ChangeIterator<'_> {
    type Item = Key;

    fn next(&mut self) -> Option<Key> {
        while self.i < Config::FIELDS.len() {
            let field = &Config::FIELDS[self.i];
            self.i += 1;
            if !(field.eq)(self.old, self.new) {
                return Some(Key(field.name));
            }
        }
        None
    }
}

/// Name of an IO error like Zig's error names (best effort), used in
/// user-visible diagnostics.
fn zig_io_error_name(err: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::NotFound => "FileNotFound".to_string(),
        ErrorKind::PermissionDenied => "AccessDenied".to_string(),
        _ => err.to_string(),
    }
}

fn file_kind_name(meta: &std::fs::Metadata) -> &'static str {
    use std::os::unix::fs::FileTypeExt;
    let ft = meta.file_type();
    if ft.is_dir() {
        "directory"
    } else if ft.is_symlink() {
        "sym_link"
    } else if ft.is_block_device() {
        "block_device"
    } else if ft.is_char_device() {
        "character_device"
    } else if ft.is_fifo() {
        "named_pipe"
    } else if ft.is_socket() {
        "unix_domain_socket"
    } else {
        "unknown"
    }
}

/// This runs a heuristic to determine if we are likely running
/// Ghostty in a CLI environment. We need this to change some behaviors.
/// We should keep the set of behaviors that depend on this as small
/// as possible because magic sucks, but each place is well documented.
fn probable_cli_environment() -> bool {
    // If we have TERM_PROGRAM set to a non-empty value, we assume
    // a graphical terminal environment.
    if let Some(v) = std::env::var_os("TERM_PROGRAM") {
        if !v.is_empty() {
            return true;
        }
    }

    // CLI arguments makes things probable
    if std::env::args_os().len() > 1 {
        return true;
    }

    // Unlikely CLI environment
    false
}

impl CliTarget for Config {
    fn parse_into_field(&mut self, key: &str, value: Option<&str>) -> Result<(), ArgError> {
        self.parse_field(key, value)
    }

    fn valid_values(&self, key: &str) -> Option<&'static [&'static str]> {
        Config::valid_values_for(key)
    }

    fn diagnostics_mut(&mut self) -> Option<&mut DiagnosticList> {
        Some(&mut self.diagnostics)
    }

    /// Callback for the arg parser to allow us to handle special cases
    /// like `-e`. Returns `Ok(false)` if the CLI parsing should halt.
    fn parse_manually_hook(&mut self, arg: &str, iter: &mut dyn ArgIter) -> Result<bool, ArgError> {
        if arg == "-e" {
            // Add the special -e marker. This prevents:
            // (1) config-file from adding args to the end (see #2908)
            // (2) dark/light theme from making this conditional
            self.replay_steps.push(ReplayStep::DashE);

            // Build up the command.
            let mut command: Vec<String> = Vec::new();
            while let Some(param) = iter.next_arg() {
                self.replay_steps.push(ReplayStep::Arg(param.clone()));
                command.push(param);
            }

            if command.is_empty() {
                self.diagnostics.append(Diagnostic {
                    location: iter.location().unwrap_or_default(),
                    message: format!("missing command after {arg}"),
                    ..Diagnostic::default()
                });

                return Ok(false);
            }

            // See "command" docs for the implied configurations and why.
            self.initial_command = Some(Command::Direct(command));
            self.gtk_single_instance = GtkSingleInstance::False;
            self.quit_after_last_window_closed = true;
            self.quit_after_last_window_closed_delay = None;
            if self.shell_integration != ShellIntegration::None {
                self.shell_integration = ShellIntegration::Detect;
            }

            // Do not continue, we consumed everything.
            return Ok(false);
        }

        // Keep track of our input args for replay
        self.replay_steps.push(ReplayStep::Arg(arg.to_string()));

        // If we didn't find a special case, continue parsing normally
        Ok(true)
    }

    fn compatibility(&mut self, key: &str, value: Option<&str>) -> bool {
        match key {
            // Ghostty 1.1 introduced background-blur support for Linux which
            // doesn't support a specific radius value. The renaming is to let
            // one field be used for both platforms (macOS retained the ability
            // to set a radius).
            "background-blur-radius" => {
                cli_args::compatibility_renamed(self, "background-blur", value)
            }

            // Ghostty 1.2 renamed all our adw options to gtk because we now have
            // a hard dependency on libadwaita.
            "adw-toolbar-style" => {
                cli_args::compatibility_renamed(self, "gtk-toolbar-style", value)
            }

            // Ghostty 1.2 removed the `hidden` value from `gtk-tabs-location` and
            // moved it to `window-show-tab-bar`.
            "gtk-tabs-location" => {
                if value.unwrap_or("") == "hidden" {
                    self.window_show_tab_bar = WindowShowTabBar::Never;
                    return true;
                }
                false
            }

            // Ghostty 1.2 lets you set `cell-foreground` and `cell-background`
            // to match the cell foreground and background colors, respectively.
            // This can be used with `cursor-color` and `cursor-text` to recreate
            // this behavior. This applies to selection too.
            "cursor-invert-fg-bg" => {
                // We don't do anything if the value is unset, which is technically
                // not EXACTLY the same as prior behavior since it would fallback
                // to doing whatever cursor-color/cursor-text were set to, but
                // I don't want to store what that is separately so this is close
                // enough.
                //
                // Realistically, these fields were mutually exclusive so anyone
                // relying on that behavior should just upgrade to the new
                // cursor-color/cursor-text fields.
                let Ok(set) = cli_args::parse_bool(value.unwrap_or("t")) else {
                    return false;
                };
                if set {
                    self.cursor_color = Some(TerminalColor::CellForeground);
                    self.cursor_text = Some(TerminalColor::CellBackground);
                }
                true
            }
            "selection-invert-fg-bg" => {
                let Ok(set) = cli_args::parse_bool(value.unwrap_or("t")) else {
                    return false;
                };
                if set {
                    self.selection_foreground = Some(TerminalColor::CellBackground);
                    self.selection_background = Some(TerminalColor::CellForeground);
                }
                true
            }

            // Ghostty 1.2 merged `bold-is-bright` into the new `bold-color`
            // by setting the value to "bright".
            "bold-is-bright" => {
                let Ok(isset) = cli_args::parse_bool(value.unwrap_or("t")) else {
                    return false;
                };
                if isset {
                    self.bold_color = Some(BoldColor::Bright);
                }
                true
            }

            // Ghostty 1.2 removed the "desktop" option and renamed it to "detect".
            // The semantics also changed slightly but this is the correct mapping.
            "gtk-single-instance" => {
                if value.unwrap_or("") == "desktop" {
                    self.gtk_single_instance = GtkSingleInstance::Detect;
                    return true;
                }
                false
            }

            // Ghostty 1.3 rename the "window" option to "new-window".
            // See: https://github.com/ghostty-org/ghostty/pull/9764
            "macos-dock-drop-behavior" => {
                if value.unwrap_or("") == "window" {
                    self.macos_dock_drop_behavior = MacOSDockDropBehavior::NewWindow;
                    return true;
                }
                false
            }

            _ => false,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::field_reassign_with_default,
    clippy::assert_is_empty
)]
mod tests {
    use std::io::Cursor;

    use ghostty_os::TempDir;

    use super::*;
    use crate::cli_args::VecIterator;

    fn it(args: &[&str]) -> VecIterator {
        VecIterator::new(args.iter().map(|s| (*s).to_string()).collect())
    }

    fn write_theme(td: &TempDir, name: &str, data: &str) -> String {
        let p = td.path().join(name);
        std::fs::write(&p, data).unwrap();
        std::fs::canonicalize(&p)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn handle_bom_in_config_files() {
        {
            let data = "\u{feff}abnormal-command-exit-runtime = 2500\n";
            let mut reader = Cursor::new(data.as_bytes().to_vec());
            let mut cfg = Config::default();
            cfg.load_reader(&mut reader, "/home/ghostty/.config/ghostty/config.ghostty")
                .unwrap();
            cfg.finalize().unwrap();

            assert!(cfg.diagnostics.empty());
            assert_eq!(cfg.abnormal_command_exit_runtime, 2500);
        }

        {
            let data = "abnormal-command-exit-runtime = 2500\n";
            let mut reader = Cursor::new(data.as_bytes().to_vec());
            let mut cfg = Config::default();
            cfg.load_reader(&mut reader, "/home/ghostty/.config/ghostty/config.ghostty")
                .unwrap();
            cfg.finalize().unwrap();

            assert!(cfg.diagnostics.empty());
            assert_eq!(cfg.abnormal_command_exit_runtime, 2500);
        }
    }

    #[test]
    fn parse_hook_invalid_command() {
        let mut cfg = Config::default();
        let mut i = it(&["foo"]);
        assert!(cfg.parse_manually_hook("--command", &mut i).unwrap());
        assert!(cfg.command.is_none());
    }

    #[test]
    fn parse_e_command_only() {
        let mut cfg = Config::default();
        let mut i = it(&["foo"]);
        assert!(!cfg.parse_manually_hook("-e", &mut i).unwrap());

        let Some(Command::Direct(cmd)) = &cfg.initial_command else {
            panic!("expected direct");
        };
        assert_eq!(cmd, &["foo"]);
    }

    #[test]
    fn parse_e_command_and_args() {
        let mut cfg = Config::default();
        let mut i = it(&["echo", "foo", "bar baz"]);
        assert!(!cfg.parse_manually_hook("-e", &mut i).unwrap());

        let Some(Command::Direct(cmd)) = &cfg.initial_command else {
            panic!("expected direct");
        };
        assert_eq!(cmd, &["echo", "foo", "bar baz"]);
    }

    #[test]
    fn clone_default() {
        let source = Config::default();
        let dest = source.clone();

        // Should have no changes
        let mut changes = source.change_iterator(&dest);
        assert_eq!(changes.next(), None);
    }

    #[test]
    fn clone_preserves_conditional_state() {
        let mut a = Config::default();
        a.conditional_state.theme = conditional::Theme::Dark;
        let dest = a.clone();

        // Should have no changes
        let mut changes = a.change_iterator(&dest);
        assert_eq!(changes.next(), None);

        // Should have the same conditional state
        assert_eq!(dest.conditional_state.theme, conditional::Theme::Dark);
    }

    #[test]
    fn clone_can_then_change_conditional_state() {
        // This tests a particular bug sequence where:
        //   1. Load light
        //   2. Convert to dark
        //   3. Clone dark
        //   4. Convert to light
        //   5. Config is still dark (bug)
        let td = TempDir::init().unwrap();
        let light = write_theme(&td, "theme_light", include_str!("testdata/theme_light"));
        let dark = write_theme(&td, "theme_dark", include_str!("testdata/theme_dark"));

        let mut cfg_light = Config::default();
        let mut i = it(&[&format!("--theme=light:{light},dark:{dark}")]);
        cfg_light.load_iter(&mut i).unwrap();
        cfg_light.finalize().unwrap();

        let cfg_dark = cfg_light
            .change_conditional_state(conditional::State {
                theme: conditional::Theme::Dark,
                ..conditional::State::default()
            })
            .unwrap()
            .unwrap();

        assert_eq!(
            cfg_dark.background,
            Color {
                r: 0xEE,
                g: 0xEE,
                b: 0xEE
            }
        );

        let cfg_clone = cfg_dark.clone();
        assert_eq!(
            cfg_clone.background,
            Color {
                r: 0xEE,
                g: 0xEE,
                b: 0xEE
            }
        );

        let cfg_light2 = cfg_clone
            .change_conditional_state(conditional::State {
                theme: conditional::Theme::Light,
                ..conditional::State::default()
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            cfg_light2.background,
            Color {
                r: 0xFF,
                g: 0xFF,
                b: 0xFF
            }
        );
    }

    #[test]
    fn clone_preserves_conditional_set() {
        let mut cfg = Config::default();
        let mut i = it(&["--theme=light:foo,dark:bar", "--window-theme=auto"]);
        cfg.load_iter(&mut i).unwrap();
        cfg.finalize().unwrap();

        let clone1 = cfg.clone();
        assert!(clone1.conditional_set.contains(&conditional::Key::Theme));
    }

    #[test]
    fn working_directory_expands_tilde() {
        let mut cfg = Config::default();
        let mut i = it(&["--working-directory=~/projects/ghostty"]);
        cfg.load_iter(&mut i).unwrap();
        cfg.finalize().unwrap();

        let expected = ghostty_os::expand_home("~/projects/ghostty")
            .unwrap_or_else(|_| "~/projects/ghostty".to_string());
        assert_eq!(
            cfg.working_directory.as_ref().unwrap().value().unwrap(),
            expected
        );
    }

    #[test]
    fn changed() {
        let source = Config::default();
        let mut dest = source.clone();
        dest.font_thicken = true;

        assert!(source.changed(&dest, crate::keys::FONT_THICKEN));
        assert!(!source.changed(&dest, crate::keys::FONT_SIZE));
    }

    #[test]
    fn change_conditional_state_ignores_irrelevant_changes() {
        let mut cfg = Config::default();
        let mut i = it(&["--theme=foo"]);
        cfg.load_iter(&mut i).unwrap();
        cfg.finalize().unwrap();

        assert!(
            cfg.change_conditional_state(conditional::State {
                theme: conditional::Theme::Dark,
                ..conditional::State::default()
            })
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn change_conditional_state_applies_relevant_changes() {
        let mut cfg = Config::default();
        let mut i = it(&["--theme=light:foo,dark:bar"]);
        cfg.load_iter(&mut i).unwrap();
        cfg.finalize().unwrap();

        let cfg2 = cfg
            .change_conditional_state(conditional::State {
                theme: conditional::Theme::Dark,
                ..conditional::State::default()
            })
            .unwrap()
            .unwrap();

        assert!(cfg2.conditional_set.contains(&conditional::Key::Theme));
    }

    #[test]
    fn theme_loading() {
        let td = TempDir::init().unwrap();
        let path = write_theme(&td, "theme", include_str!("testdata/theme_simple"));

        let mut cfg = Config::default();
        let mut i = it(&[&format!("--theme={path}")]);
        cfg.load_iter(&mut i).unwrap();
        cfg.finalize().unwrap();

        assert_eq!(
            cfg.background,
            Color {
                r: 0x12,
                g: 0x3A,
                b: 0xBC
            }
        );

        // Not a conditional theme
        assert!(!cfg.conditional_set.contains(&conditional::Key::Theme));
    }

    #[test]
    fn theme_loading_preserves_conditional_state() {
        let td = TempDir::init().unwrap();
        let path = write_theme(&td, "theme", include_str!("testdata/theme_simple"));

        let mut cfg = Config::default();
        cfg.conditional_state = conditional::State {
            theme: conditional::Theme::Dark,
            ..conditional::State::default()
        };
        let mut i = it(&[&format!("--theme={path}")]);
        cfg.load_iter(&mut i).unwrap();
        cfg.finalize().unwrap();

        assert_eq!(cfg.conditional_state.theme, conditional::Theme::Dark);
    }

    #[test]
    fn theme_priority_is_lower_than_config() {
        let td = TempDir::init().unwrap();
        let path = write_theme(&td, "theme", include_str!("testdata/theme_simple"));

        let mut cfg = Config::default();
        let mut i = it(&["--background=#ABCDEF", &format!("--theme={path}")]);
        cfg.load_iter(&mut i).unwrap();
        cfg.finalize().unwrap();

        assert_eq!(
            cfg.background,
            Color {
                r: 0xAB,
                g: 0xCD,
                b: 0xEF
            }
        );
    }

    #[test]
    fn theme_loading_correct_light_dark() {
        let td = TempDir::init().unwrap();
        let light = write_theme(&td, "theme_light", include_str!("testdata/theme_light"));
        let dark = write_theme(&td, "theme_dark", include_str!("testdata/theme_dark"));
        let arg = format!("--theme=light:{light},dark:{dark}");

        // Light
        {
            let mut cfg = Config::default();
            cfg.load_iter(&mut it(&[&arg])).unwrap();
            cfg.finalize().unwrap();
            assert_eq!(
                cfg.background,
                Color {
                    r: 0xFF,
                    g: 0xFF,
                    b: 0xFF
                }
            );
        }

        // Dark
        {
            let mut cfg = Config::default();
            cfg.conditional_state = conditional::State {
                theme: conditional::Theme::Dark,
                ..conditional::State::default()
            };
            cfg.load_iter(&mut it(&[&arg])).unwrap();
            cfg.finalize().unwrap();
            assert_eq!(
                cfg.background,
                Color {
                    r: 0xEE,
                    g: 0xEE,
                    b: 0xEE
                }
            );
        }

        // Light to Dark
        {
            let mut cfg = Config::default();
            cfg.load_iter(&mut it(&[&arg])).unwrap();
            cfg.finalize().unwrap();

            let new = cfg
                .change_conditional_state(conditional::State {
                    theme: conditional::Theme::Dark,
                    ..conditional::State::default()
                })
                .unwrap()
                .unwrap();
            assert_eq!(
                new.background,
                Color {
                    r: 0xEE,
                    g: 0xEE,
                    b: 0xEE
                }
            );
        }
    }

    #[test]
    fn theme_specifying_light_dark_changes_window_theme_from_auto() {
        let mut cfg = Config::default();
        let mut i = it(&["--theme=light:foo,dark:bar", "--window-theme=auto"]);
        cfg.load_iter(&mut i).unwrap();
        cfg.finalize().unwrap();

        assert_eq!(cfg.window_theme, WindowTheme::System);
    }

    #[test]
    fn theme_specifying_light_dark_sets_theme_usage_in_conditional_state() {
        let mut cfg = Config::default();
        let mut i = it(&["--theme=light:foo,dark:bar", "--window-theme=auto"]);
        cfg.load_iter(&mut i).unwrap();
        cfg.finalize().unwrap();

        assert_eq!(cfg.window_theme, WindowTheme::System);
        assert!(cfg.conditional_set.contains(&conditional::Key::Theme));
    }

    #[test]
    fn compatibility_gtk_single_instance_desktop() {
        let mut cfg = Config::default();
        cfg.load_iter(&mut it(&["--gtk-single-instance=desktop"]))
            .unwrap();

        // We need to test this BEFORE finalize, because finalize will
        // convert our detect to a real value.
        assert_eq!(cfg.gtk_single_instance, GtkSingleInstance::Detect);
    }

    #[test]
    fn compatibility_removed_cursor_invert_fg_bg() {
        let mut cfg = Config::default();
        cfg.load_iter(&mut it(&["--cursor-invert-fg-bg"])).unwrap();
        cfg.finalize().unwrap();

        assert_eq!(cfg.cursor_color, Some(TerminalColor::CellForeground));
        assert_eq!(cfg.cursor_text, Some(TerminalColor::CellBackground));
    }

    #[test]
    fn compatibility_removed_selection_invert_fg_bg() {
        let mut cfg = Config::default();
        cfg.load_iter(&mut it(&["--selection-invert-fg-bg"]))
            .unwrap();
        cfg.finalize().unwrap();

        assert_eq!(
            cfg.selection_foreground,
            Some(TerminalColor::CellBackground)
        );
        assert_eq!(
            cfg.selection_background,
            Some(TerminalColor::CellForeground)
        );
    }

    #[test]
    fn compatibility_removed_bold_is_bright() {
        let mut cfg = Config::default();
        cfg.load_iter(&mut it(&["--bold-is-bright"])).unwrap();
        cfg.finalize().unwrap();

        assert_eq!(cfg.bold_color, Some(BoldColor::Bright));
    }

    #[test]
    fn compatibility_window_new_window() {
        let mut cfg = Config::default();
        cfg.load_iter(&mut it(&["--macos-dock-drop-behavior=window"]))
            .unwrap();
        cfg.finalize().unwrap();
        assert_eq!(
            cfg.macos_dock_drop_behavior,
            MacOSDockDropBehavior::NewWindow
        );
    }

    #[test]
    fn value_key_types() {
        // Zig test `Value` in key.zig: the key table has the expected
        // fields with the expected types.
        let c = Config::default();
        assert!(c.font_family.list.is_empty());
        assert!(c.cursor_style_blink.is_none());
        assert!(Config::FIELDS.iter().any(|f| f.name == "font-family"));
    }

    #[test]
    fn config_file_cycle_detected() {
        let td = TempDir::init().unwrap();
        let a = td.path().join("a.conf");
        let b = td.path().join("b.conf");
        std::fs::write(&a, format!("config-file = {}\n", b.display())).unwrap();
        std::fs::write(&b, format!("config-file = {}\n", a.display())).unwrap();

        let mut cfg = Config::default();
        cfg.load_iter(&mut it(&[&format!("--config-file={}", a.display())]))
            .unwrap();
        cfg.expand_paths("/").unwrap();
        cfg.load_recursive_files().unwrap();
        assert!(
            cfg.diagnostics
                .items()
                .iter()
                .any(|d| d.message.contains("cycle detected"))
        );
    }

    #[test]
    fn config_file_optional_missing_is_silent() {
        let mut cfg = Config::default();
        cfg.load_iter(&mut it(&["--config-file=?/nonexistent/ghostty/file"]))
            .unwrap();
        cfg.expand_paths("/").unwrap();
        cfg.load_recursive_files().unwrap();
        assert!(cfg.diagnostics.empty());
    }

    #[test]
    fn default_config_formats_with_docs() {
        let cfg = Config::default();
        let f = crate::file_formatter::FileFormatter {
            config: &cfg,
            docs: true,
            changed: false,
        };
        let out = f.to_string();
        assert!(out.contains("# The font families to use."));
        assert!(out.contains("\nfont-size = 12\n"));
        assert!(out.contains("\nbackground = #282c34\n"));
    }
}
