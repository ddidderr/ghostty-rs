//! `+show-face` (ZIG/src/cli/show_face.zig).

use ghostty_config::{
    config::Config,
    config_enum,
    config_struct,
    diagnostics::{DiagnosticList, Location},
};
use ghostty_font::{
    self as font,
    SharedGrid,
    SharedGridSet,
    collection::{EntryError, GetFaceError},
    face::DesiredSize,
    shared_grid_set::DerivedConfig,
};

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

    run_args(&opts, &mut std::io::stdout(), &mut std::io::stdout())
}

fn run_args(
    opts: &Options,
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
) -> Result<u8, Error> {
    // Print out any diagnostics, unless it's likely that the diagnostic was
    // generated trying to parse a "normal" configuration setting. Exit with an
    // error code if any diagnostics were printed.
    const FIELDS: [&str; 4] = ["cp", "string", "style", "presentation"];
    let mut exit = false;
    for d in opts.diagnostics.items() {
        if !matches!(d.location, Location::Cli(_)) {
            continue;
        }
        if FIELDS.contains(&d.key.as_str()) {
            stderr.write_all(&ghostty_os::osbytes::config_str_to_bytes(&format!(
                "config error: {d}\n"
            )))?;
            exit = true;
        }
    }
    if exit {
        return Ok(1);
    }

    if opts.cp.is_none() && opts.string.is_none() {
        writeln!(
            stderr,
            "You must specify a codepoint with --cp or a string with --string"
        )?;
        return Ok(1);
    }

    let config = match Config::load() {
        Ok(c) => c,
        Err(err) => {
            write!(stderr, "Unable to load config: {err}")?;
            return Ok(1);
        }
    };

    // Print out any diagnostics generated from parsing the config, unless
    // the diagnostic might have been generated because it's actually an
    // action-specific argument.
    for d in config.diagnostics.items() {
        if FIELDS.contains(&d.key.as_str())
            && matches!(d.location, Location::None | Location::Cli(_))
        {
            continue;
        }
        stderr.write_all(&ghostty_os::osbytes::config_str_to_bytes(&format!(
            "config error: {d}\n"
        )))?;
    }

    let font_grid_set = match SharedGridSet::new() {
        Ok(v) => v,
        Err(err) => {
            write!(stderr, "Unable to initialize font grid set: {err}")?;
            return Ok(1);
        }
    };

    let font_size = DesiredSize {
        points: config.font_size,
        xdpi: 96,
        ydpi: 96,
    };
    let font_config = DerivedConfig::new(&config);
    let (key, grid) = match font_grid_set.ref_(&font_config, font_size) {
        Ok(v) => v,
        Err(err) => {
            write!(stderr, "Unable to get font grid: {err}")?;
            return Ok(1);
        }
    };

    let style = match opts.style {
        Style::Regular => font::Style::Regular,
        Style::Bold => font::Style::Bold,
        Style::Italic => font::Style::Italic,
        Style::BoldItalic => font::Style::BoldItalic,
    };
    let presentation = opts.presentation.map(|p| match p {
        Presentation::Text => font::Presentation::Text,
        Presentation::Emoji => font::Presentation::Emoji,
    });

    let mut rc = None;
    if let Some(cp) = opts.cp {
        rc = lookup(stdout, stderr, &grid, style, presentation, cp)?;
    }
    if rc.is_none()
        && let Some(string) = &opts.string
    {
        for ch in string.chars() {
            rc = lookup(stdout, stderr, &grid, style, presentation, ch as u32)?;
            if rc.is_some() {
                break;
            }
        }
    }

    drop(grid);
    font_grid_set.deref(&key);
    Ok(rc.unwrap_or(0))
}

fn lookup(
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
    grid: &SharedGrid,
    style: font::Style,
    presentation: Option<font::Presentation>,
    cp: u32,
) -> Result<Option<u8>, Error> {
    let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
    let Ok(Some(idx)) = grid.get_index(cp, style, presentation) else {
        writeln!(stdout, "U+{cp:02X} \u{ab} {ch} \u{bb} not found.")?;
        return Ok(None);
    };

    let mut st = grid.write();
    let face = match st.resolver.collection.get_face(idx) {
        Ok(f) => f,
        Err(GetFaceError::Entry(EntryError::SpecialHasNoFace)) => {
            writeln!(
                stdout,
                "U+{cp:02X} \u{ab} {ch} \u{bb} is handled by Ghostty's internal sprites."
            )?;
            return Ok(None);
        }
        Err(err) => {
            write!(stderr, "Unable to get face: {err}")?;
            return Ok(Some(1));
        }
    };

    let name = face.name();
    writeln!(
        stdout,
        "U+{cp:02X} \u{ab} {ch} \u{bb} found in face \u{201c}{name}\u{201d}."
    )?;
    Ok(None)
}
