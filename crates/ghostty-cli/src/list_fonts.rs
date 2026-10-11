//! `+list-fonts` (ZIG/src/cli/list_fonts.zig).

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

    use std::collections::HashMap;
    use std::io::Write;

    // We'll be putting our fonts into a list categorized by family
    // so it is easier to read the output.
    let mut families: Vec<String> = Vec::new();
    let mut map: HashMap<String, Vec<String>> = HashMap::new();

    // Look up all available fonts
    let disco = ghostty_font::Discover::init();
    let desc = ghostty_font::Descriptor {
        monospace: opts.family.is_none(),
        family: opts.family.clone(),
        style: opts.style.clone(),
        bold: opts.bold,
        italic: opts.italic,
        ..ghostty_font::Descriptor::default()
    };
    let mut it = disco
        .discover(&desc)
        .map_err(|e| Error::Other(e.to_string()))?;
    while let Some(face) = it.next().map_err(|e| Error::Other(e.to_string()))? {
        let family = match face.family_name() {
            Ok(v) => v,
            Err(err) => {
                log::error!("failed to get font family name: {err}");
                continue;
            }
        };
        let full_name = match face.name() {
            Ok(v) => v,
            Err(err) => {
                log::error!("failed to get font name: {err}");
                continue;
            }
        };
        let list = map.entry(family.clone()).or_insert_with(|| {
            families.push(family);
            Vec::new()
        });
        list.push(full_name);
    }

    // Sort our keys.
    if opts.family.is_none() {
        families.sort_unstable();
    }

    // Output each
    let mut stdout = std::io::BufWriter::new(std::io::stdout().lock());
    for family in &families {
        let Some(list) = map.get_mut(family) else {
            continue;
        };
        if list.is_empty() {
            continue;
        }
        if opts.family.is_none() {
            list.sort_unstable();
        }

        writeln!(stdout, "{family}")?;
        for item in list.iter() {
            writeln!(stdout, "  {item}")?;
        }
        writeln!(stdout)?;
    }
    stdout.flush()?;
    Ok(0)
}
