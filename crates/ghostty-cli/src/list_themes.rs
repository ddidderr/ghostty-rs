//! `+list-themes` (ZIG/src/cli/list_themes.zig).
//!
//! The non-interactive listing is a faithful port. The interactive preview
//! (libvaxis in Zig) is implemented with ratatui + crossterm in [`preview`].

use std::{
    cmp::Ordering,
    fmt::Write as _,
    io::{IsTerminal, Write},
};

use ghostty_config::{
    config::Config,
    config_enum,
    config_struct,
    theme::{Location, LocationIterator},
};
use ghostty_os::osbytes::{
    config_str_to_bytes,
    config_str_to_path,
    os_to_config_string,
    path_to_config_string,
};

use crate::{error::Error, tui};

pub mod preview;

// When the number of filtered themes is less than or equal to this threshold,
// the window position will be reset to 0 to show all results from the top.
// This ensures better visibility for small result sets while maintaining
// scroll position for larger lists.
pub(crate) const SMALL_LIST_THRESHOLD: usize = 10;

config_enum! {
    pub enum ColorScheme {
        All => "all",
        Dark => "dark",
        Light => "light",
    }
}

config_struct! {
    pub struct Options {
        /// If true, print the full path to the theme.
        "path" => path: bool = false,

        /// If true, force a plain list of themes.
        "plain" => plain: bool = false,

        /// Specifies the color scheme of the themes to include in the list.
        "color" => color: ColorScheme = ColorScheme::All,
    }
}
cli_options!(Options, help);

/// An entry of the theme list (Zig: `ThemeListElement`).
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeListElement {
    pub location: Location,
    pub path: String,
    pub theme: String,
    pub rank: Option<f64>,
}

impl ThemeListElement {
    /// TODO: use Unicode-aware comparison (as in Zig).
    #[must_use]
    pub fn order(&self, other: &ThemeListElement) -> Ordering {
        let a = self.theme.bytes().map(|c| c.to_ascii_lowercase());
        let b = other.theme.bytes().map(|c| c.to_ascii_lowercase());
        a.cmp(b)
    }

    /// `file://` URI of the theme file (Zig: `toUri`).
    #[must_use]
    pub fn to_uri(&self) -> String {
        let mut out = String::from("file://");
        for b in ghostty_os::osbytes::config_str_to_bytes(&self.path) {
            // std.Uri path component escaping: unreserved + sub-delims + ":@/".
            if b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'-' | b'.'
                        | b'_'
                        | b'~'
                        | b'!'
                        | b'$'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b'*'
                        | b'+'
                        | b','
                        | b';'
                        | b'='
                        | b':'
                        | b'@'
                        | b'/'
                )
            {
                out.push(b as char);
            } else {
                let _ = write!(out, "%{b:02X}");
            }
        }
        out
    }
}

/// The name of a theme location (`{t}` of `themepkg.Location`).
#[must_use]
pub fn location_name(loc: Location) -> &'static str {
    match loc {
        Location::User => "user",
        Location::Resources => "resources",
    }
}

/// Collect the themes of all theme directories.
///
/// Returns the unsorted list (in directory order). Errors opening a
/// directory other than "not found" are printed to stderr and skipped.
#[must_use]
pub fn collect_themes(resources_dir: Option<&str>) -> Vec<ThemeListElement> {
    let mut themes = vec![];
    let mut it = LocationIterator::new(resources_dir);
    while let Some((location, dir)) = it.next_location() {
        let rd = match std::fs::read_dir(config_str_to_path(&dir)) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                eprintln!("error trying to open {dir}: {e}");
                continue;
            }
        };

        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !(ft.is_file() || ft.is_symlink()) {
                continue;
            }
            // Entry names are raw bytes: keep them via the reversible escape.
            let raw_name = entry.file_name();
            let name = os_to_config_string(&raw_name);
            if name == ".DS_Store" {
                continue;
            }
            themes.push(ThemeListElement {
                path: path_to_config_string(&config_str_to_path(&dir).join(&raw_name)),
                location,
                theme: name,
                rank: None,
            });
        }
    }
    themes
}

/// Whether the theme with the given loaded config matches the color filter.
#[must_use]
pub fn should_include_theme(theme_filter: ColorScheme, theme_config: &Config) -> bool {
    let rf = f32::from(theme_config.background.r) / 255.0;
    let gf = f32::from(theme_config.background.g) / 255.0;
    let bf = f32::from(theme_config.background.b) / 255.0;
    let luminance = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
    let is_dark = luminance < 0.5;
    theme_filter == ColorScheme::All
        || (theme_filter == ColorScheme::Dark && is_dark)
        || (theme_filter == ColorScheme::Light && !is_dark)
}

/// Make `path` absolute (`Config::load_file` requires absolute paths).
pub(crate) fn absolute_path(path: &str) -> String {
    std::path::absolute(config_str_to_path(path))
        .map_or_else(|_| path.to_string(), |p| path_to_config_string(&p))
}

fn resources_dir() -> Option<String> {
    ghostty_os::resources_dir()
        .ok()
        .and_then(|r| r.app().map(str::to_string))
}

/// The `list-themes` command is used to preview or list all the available
/// themes for Ghostty.
///
/// If this command is run from a TTY, a TUI preview of the themes will be
/// shown. While in the preview, `F1` will bring up a help screen and `ESC` will
/// exit the preview. Other keys that can be used to navigate the preview are
/// listed in the help screen.
///
/// If this command is not run from a TTY, or the output is piped to another
/// command, a plain list of theme names will be printed to the screen. A plain
/// list can be forced using the `--plain` CLI flag.
///
/// Two different directories will be searched for themes.
///
/// The first directory is the `themes` subdirectory of your Ghostty
/// configuration directory. This is `$XDG_CONFIG_HOME/ghostty/themes` or
/// `~/.config/ghostty/themes`.
///
/// The second directory is the `themes` subdirectory of the Ghostty resources
/// directory. Ghostty ships with a multitude of themes that will be installed
/// into this directory. On macOS, this directory is the
/// `Ghostty.app/Contents/Resources/ghostty/themes`. On Linux, this directory
/// is the `share/ghostty/themes` (wherever you installed the Ghostty "share"
/// directory). If you're running Ghostty from the source, this is the
/// `zig-out/share/ghostty/themes` directory.
///
/// You can also set the `GHOSTTY_RESOURCES_DIR` environment variable to point
/// to the resources directory.
///
/// Flags:
///
///   * `--path`: Show the full path to the theme.
///
///   * `--plain`: Force a plain listing of themes.
///
///   * `--color`: Specify the color scheme of the themes included in the list.
///                This can be `dark`, `light`, or `all`. The default is `all`.
pub fn run(args: &[String]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    let stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    let resources_dir = resources_dir();
    if resources_dir.is_none() {
        writeln!(
            stderr,
            "Could not find the Ghostty resources directory. Please ensure \
             that Ghostty is installed correctly."
        )?;
    }

    let mut themes = collect_themes(resources_dir.as_deref());

    if themes.is_empty() {
        write!(
            stderr,
            "No themes found, check to make sure that the themes were installed correctly."
        )?;
        return Ok(1);
    }

    // sortUnstable in Zig; a stable sort is a valid unstable order.
    themes.sort_by(ThemeListElement::order);

    if tui::CAN_PRETTY_PRINT && !opts.plain && stdout.is_terminal() {
        preview::preview(themes, opts.color)?;
        return Ok(0);
    }

    let mut out = std::io::BufWriter::new(stdout.lock());
    write_plain(&mut out, &themes, &opts)?;
    // Don't forget to flush!
    out.flush()?;
    Ok(0)
}

/// The plain listing. Like Zig, one `Config` is reused for every theme
/// file (so a theme that does not set `background` inherits the previous
/// theme's value).
pub fn write_plain(
    out: &mut dyn Write,
    themes: &[ThemeListElement],
    opts: &Options,
) -> Result<(), Error> {
    let mut theme_config = Config::new_default();
    for theme in themes {
        theme_config.load_file(&absolute_path(&theme.path))?;
        if !should_include_theme(opts.color, &theme_config) {
            continue;
        }
        let loc = location_name(theme.location);
        // Names/paths are raw bytes in Zig: decode the reversible escape so
        // non-UTF-8 bytes are written back unchanged.
        let line = if opts.path {
            format!("{} ({loc}) {}\n", theme.theme, theme.path)
        } else {
            format!("{} ({loc})\n", theme.theme)
        };
        out.write_all(&config_str_to_bytes(&line))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn theme_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, content) in files {
            std::fs::write(dir.join(name), content).unwrap();
        }
        tmp
    }

    #[cfg(unix)]
    #[test]
    fn collect_themes_keeps_non_utf8_names() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("themes");
        std::fs::create_dir_all(&dir).unwrap();
        let raw = std::ffi::OsStr::from_bytes(b"th\xffeme");
        std::fs::write(dir.join(raw), "background = #000000\n").unwrap();
        let res = tmp.path().to_string_lossy().into_owned();
        let themes: Vec<_> = collect_themes(Some(&res))
            .into_iter()
            .filter(|t| t.location == Location::Resources)
            .collect();
        assert_eq!(1, themes.len());
        assert_eq!(config_str_to_bytes(&themes[0].theme), b"th\xffeme".to_vec());
        // The path points at the real file and the listing loads it.
        assert_eq!(config_str_to_path(&themes[0].path), dir.join(raw));
        let mut out = Vec::new();
        write_plain(&mut out, &themes, &Options::default()).unwrap();
        assert_eq!(out, b"th\xffeme (resources)\n");
    }

    #[test]
    fn collect_sort_and_filter() {
        let tmp = theme_dir(&[
            ("Zeta", "background = #ffffff\n"),
            ("alpha", "background = #000000\n"),
            ("Beta", "background = 202020\n"),
            (".DS_Store", "junk"),
        ]);
        let res = tmp.path().to_string_lossy().into_owned();
        let mut themes: Vec<_> = collect_themes(Some(&res))
            .into_iter()
            .filter(|t| t.location == Location::Resources)
            .collect();
        themes.sort_by(ThemeListElement::order);
        let names: Vec<_> = themes.iter().map(|t| t.theme.as_str()).collect();
        assert_eq!(vec!["alpha", "Beta", "Zeta"], names);

        let list = |color, path| {
            let mut out = Vec::new();
            let opts = Options {
                path,
                color,
                ..Options::default()
            };
            write_plain(&mut out, &themes, &opts).unwrap();
            String::from_utf8(out).unwrap()
        };
        assert_eq!(
            "alpha (resources)\nBeta (resources)\nZeta (resources)\n",
            list(ColorScheme::All, false)
        );
        assert_eq!(
            "alpha (resources)\nBeta (resources)\n",
            list(ColorScheme::Dark, false)
        );
        assert_eq!("Zeta (resources)\n", list(ColorScheme::Light, false));
        assert_eq!(
            format!("Zeta (resources) {res}/themes/Zeta\n"),
            list(ColorScheme::Light, true)
        );
    }

    #[test]
    fn luminance_threshold() {
        let mut cfg = Config::new_default();
        cfg.background = ghostty_config::types_a::Color {
            r: 127,
            g: 127,
            b: 127,
        };
        assert!(should_include_theme(ColorScheme::Dark, &cfg));
        cfg.background = ghostty_config::types_a::Color {
            r: 128,
            g: 128,
            b: 128,
        };
        assert!(should_include_theme(ColorScheme::Light, &cfg));
        assert!(should_include_theme(ColorScheme::All, &cfg));
    }

    #[test]
    fn uri_escaping() {
        let t = ThemeListElement {
            location: Location::User,
            path: "/a b/c%d".into(),
            theme: "x".into(),
            rank: None,
        };
        assert_eq!("file:///a%20b/c%25d", t.to_uri());
    }

    #[test]
    fn uri_escaping_percent_encodes_original_bytes() {
        let t = ThemeListElement {
            location: Location::User,
            path: ghostty_os::osbytes::bytes_to_config_string(b"/a/\xff\xc3x"),
            theme: "x".into(),
            rank: None,
        };
        assert_eq!("file:///a/%FF%C3x", t.to_uri());
    }

    #[test]
    fn options_color() {
        let mut opts = Options::default();
        crate::parse_options(
            &mut opts,
            &["g".into(), "--color=dark".into(), "--path".into()],
        )
        .unwrap();
        assert_eq!(ColorScheme::Dark, opts.color);
        assert!(opts.path);
        assert!(crate::parse_options(&mut opts, &["g".into(), "--color=blue".into()]).is_err());
    }
}
