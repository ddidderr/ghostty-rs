//! `+list-fonts` (ZIG/src/cli/list_fonts.zig).
//!
//! DEFERRED: needs the font crate (font discovery). Only the options and
//! help are ported; running the action reports that it is not available.

use ghostty_config::config_struct;

use crate::error::Error;

config_struct! {
    pub struct Options {
        /// The font family to search for. If this is set, then only fonts
        /// matching this family will be listed.
        "family" => family: Option<String> = None,

        /// The style name to search for.
        "style" => style: Option<String> = None,

        /// Font styles to search for. If this is set, then only fonts that
        /// match the given styles will be listed.
        "bold" => bold: bool = false,
        "italic" => italic: bool = false,
    }
}
cli_options!(Options, help);

/// The `list-fonts` command is used to list all the available fonts for
/// Ghostty. This uses the exact same font discovery mechanism Ghostty uses to
/// find fonts to use.
///
/// When executed with no arguments, this will list all available fonts, sorted
/// by family name, then font name. If a family name is given with `--family`,
/// the sorting will be disabled and the results instead will be shown in the
/// same priority order Ghostty would use to pick a font.
///
/// Flags:
///
///   * `--bold`: Filter results to specific bold styles. It is not guaranteed
///     that only those styles are returned. They are only prioritized.
///
///   * `--italic`: Filter results to specific italic styles. It is not guaranteed
///     that only those styles are returned. They are only prioritized.
///
///   * `--style`: Filter results based on the style string advertised by a font.
///     It is not guaranteed that only those styles are returned. They are only
///     prioritized.
///
///   * `--family`: Filter results to a specific font family. The family handling
///     is identical to the `font-family` set of Ghostty configuration values, so
///     this can be used to debug why your desired font may not be loading.
pub fn run(args: &[String]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    // TODO(integration): port the discovery/listing once ghostty-font lands.
    Err(Error::NotYetAvailable("list-fonts"))
}
