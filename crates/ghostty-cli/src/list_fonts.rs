//! `+list-fonts` (ZIG/src/cli/list_fonts.zig).

use ghostty_config::config_struct;

use crate::error::Error;

/// Printed (without a trailing newline) by `+list-fonts` and `+show-face`
/// when the build has no font discovery mechanism (Zig `Discover == void`,
/// the `freetype` font backend).
pub(crate) const NO_DISCOVERY_MESSAGE: &str = "Ghostty was built without a font discovery mechanism. This is a compile-time\noption. Please review how Ghostty was built from source, contact the\nmaintainer to enable a font discovery mechanism, and try again.";

config_struct! {
    pub struct Options {
        /// The font family to search for. If this is set, then only fonts
        /// matching this family will be listed.
        "family" => family: Option<ghostty_os::osbytes::EncodedOs> = None,

        /// The style name to search for.
        "style" => style: Option<ghostty_os::osbytes::EncodedOs> = None,

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
pub fn run(args: &[impl AsRef<[u8]>]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    use std::{collections::HashMap, io::Write};

    // Its possible to build Ghostty without font discovery!
    if !ghostty_font::HAS_DISCOVERY {
        let mut stderr = crate::zig_writer::ZigWriter::new(crate::zig_writer::stderr_sink(), 1024);
        write!(stderr, "{NO_DISCOVERY_MESSAGE}")?;
        stderr.flush()?;
        return Ok(1);
    }

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
    let mut stdout = crate::zig_writer::ZigWriter::new(crate::zig_writer::stdout_sink(), 2048);
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

#[cfg(test)]
mod tests {
    /// Keyed off the font crate's actually selected backend, not this
    /// crate's own feature flag: cargo feature unification can enable
    /// `ghostty-font/font-backend-freetype` without the CLI forwarding
    /// feature, and on Apple targets the default backend (CoreText) has
    /// discovery.
    ///
    /// Without a discovery mechanism (the `freetype` font backend, Zig
    /// `Discover == void`) `+list-fonts` prints the compile-time-option
    /// message and exits with 1, after parsing the arguments.
    #[test]
    fn discovery_presence_follows_selected_backend() {
        let backend = ghostty_font::options.backend;
        assert_eq!(
            ghostty_font::HAS_DISCOVERY,
            backend.has_fontconfig() || backend.has_coretext()
        );
        if ghostty_font::HAS_DISCOVERY {
            assert_ne!(backend, ghostty_font::Backend::Freetype);
        } else {
            assert_eq!(backend, ghostty_font::Backend::Freetype);
            assert_eq!(super::run(&[] as &[&[u8]]).expect("run"), 1);
            assert!(super::NO_DISCOVERY_MESSAGE.ends_with("try again."));
        }
    }
}
