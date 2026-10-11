//! `+show-face` (ZIG/src/cli/show_face.zig).
//!
//! DEFERRED: needs the font crate (collection / codepoint resolver). Only the
//! options and help are ported; running the action reports that it is not
//! available.

use ghostty_config::{config_enum, config_struct, diagnostics::DiagnosticList};

use crate::error::Error;

config_enum! {
    /// Font style (Zig: `font.Style`).
    pub enum Style {
        Regular => "regular",
        Bold => "bold",
        Italic => "italic",
        BoldItalic => "bold_italic",
    }
}

config_enum! {
    /// Font presentation (Zig: `font.Presentation`).
    pub enum Presentation {
        Text => "text",
        Emoji => "emoji",
    }
}

config_struct! {
    pub struct Options {
        /// The codepoint to search for.
        "cp" => cp: Option<u32> = None,

        /// Search for all of the codepoints in the string.
        "string" => string: Option<String> = None,

        /// Font style to search for.
        "style" => style: Style = Style::Regular,

        /// If specified, force text or emoji presentation.
        "presentation" => presentation: Option<Presentation> = None,
    }
    extra {
        // Enable arg parsing diagnostics so that we don't get an error if
        // there is a "normal" config setting on the cli.
        diagnostics: DiagnosticList = DiagnosticList::default(),
    }
}
cli_options!(Options, help, diagnostics);

/// The `show-face` command shows what font face Ghostty will use to render a
/// specific codepoint. Note that this command does not take into consideration
/// grapheme clustering or any other Unicode features that might modify the
/// presentation of a codepoint, so this may show a different font face than
/// Ghostty uses to render a codepoint in a terminal session.
///
/// Flags:
///
///   * `--cp`: Find the face for a single codepoint. The codepoint may be specified
///     in decimal (`--cp=65`), hexadecimal (`--cp=0x41`), octal (`--cp=0o101`), or
///     binary (`--cp=0b1000001`).
///
///   * `--string`: Find the face for all of the codepoints in a string. The
///     string must be a valid UTF-8 sequence.
///
///   * `--style`: Search for a specific style. Valid options are `regular`, `bold`,
///     `italic`, and `bold_italic`.
///
///   * `--presentation`: If set, force searching for a specific presentation
///     style. Valid options are `text` and `emoji`. If unset, the presentation
///     style of a codepoint will be inferred from the Unicode standard.
pub fn run(args: &[String]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    // TODO(integration): port the face lookup once ghostty-font lands.
    Err(Error::NotYetAvailable("show-face"))
}
