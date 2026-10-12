//! Theme file lookup (port of `config/theme.zig`).
//!
//! Zig reads the resources dir from global state; here it is passed
//! explicitly (`resources_dir`, the value of `ResourcesDir.app()`).

use std::fs::File;

use ghostty_os::osbytes::EncodedOs;

use crate::diagnostics::{Diagnostic, DiagnosticList};

/// Location of possible themes. The order of this enum matters because it
/// defines the priority of theme search (from top to bottom).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// XDG config dir
    User,
    /// Ghostty resources dir
    Resources,
}

impl Location {
    const ALL: [Location; 2] = [Location::User, Location::Resources];

    /// Returns the directory for the given theme based on this location type.
    ///
    /// This will return None if the directory type doesn't exist or is
    /// invalid for any reason. For example, it is perfectly valid to
    /// install and run Ghostty without the resources directory.
    #[must_use]
    pub fn dir(self, resources_dir: Option<&EncodedOs>) -> Option<EncodedOs> {
        match self {
            Location::User => ghostty_os::xdg::config(&ghostty_os::xdg::Options {
                subdir: Some("ghostty/themes"),
                home: None,
            })
            .ok(),
            Location::Resources => {
                let base = resources_dir?;
                Some(EncodedOs::from_path(&base.to_path().join("themes")))
            }
        }
    }
}

/// An iterator that returns all possible directories for finding themes in
/// order of priority.
#[derive(Debug, Clone)]
pub struct LocationIterator {
    pub resources_dir: Option<EncodedOs>,
    pub i: usize,
}

impl LocationIterator {
    #[must_use]
    pub fn new(resources_dir: Option<&EncodedOs>) -> Self {
        Self {
            resources_dir: resources_dir.cloned(),
            i: 0,
        }
    }

    pub fn next_location(&mut self) -> Option<(Location, EncodedOs)> {
        while self.i < Location::ALL.len() {
            let location = Location::ALL[self.i];
            self.i += 1;
            if let Some(dir) = location.dir(self.resources_dir.as_ref()) {
                return Some((location, dir));
            }
        }
        None
    }

    pub fn reset(&mut self) {
        self.i = 0;
    }
}

/// An opened theme.
#[derive(Debug)]
pub struct OpenedTheme {
    pub path: EncodedOs,
    pub file: File,
}

fn diag(diags: &mut DiagnosticList, message: String) {
    diags.append(Diagnostic {
        message,
        ..Diagnostic::default()
    });
}

fn kind_name(ft: std::fs::FileType) -> &'static str {
    if ft.is_file() {
        "file"
    } else if ft.is_dir() {
        "directory"
    } else if ft.is_symlink() {
        "sym_link"
    } else {
        special_kind_name(ft)
    }
}

#[cfg(unix)]
pub(crate) fn special_kind_name(ft: std::fs::FileType) -> &'static str {
    use std::os::unix::fs::FileTypeExt;
    if ft.is_block_device() {
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

/// wasm32 has no special file types (`std::os::unix` is unavailable).
#[cfg(not(unix))]
pub(crate) fn special_kind_name(_: std::fs::FileType) -> &'static str {
    "unknown"
}

/// Verify that the file is a regular file, adding a diagnostic otherwise.
fn check_regular(file: &File, theme: &str, diags: &mut DiagnosticList) -> bool {
    match file.metadata() {
        Err(err) => {
            diag(diags, format!("not reading theme from \"{theme}\": {err}"));
            false
        }
        Ok(m) if !m.is_file() => {
            diag(
                diags,
                format!(
                    "not reading theme from \"{theme}\": it is a {}",
                    kind_name(m.file_type())
                ),
            );
            false
        }
        Ok(_) => true,
    }
}

/// Open the given named theme. If there are any errors then messages
/// will be appended to the given error list and None is returned. If
/// a Some return value is returned, there are never any errors added.
///
/// This will never return anything other than a handle to a regular file. If
/// the theme resolves to something other than a regular file a diagnostic entry
/// will be added to the list and None will be returned.
pub fn open(
    theme: &EncodedOs,
    resources_dir: Option<&EncodedOs>,
    diags: &mut DiagnosticList,
) -> Option<OpenedTheme> {
    let theme_path = theme.to_path();
    let theme_text = theme.as_encoded();

    // Absolute themes are loaded a different path.
    if theme_path.is_absolute() {
        let file = open_absolute(theme, diags)?;
        if !check_regular(&file, theme_text, diags) {
            return None;
        }
        return Some(OpenedTheme {
            path: theme.clone(),
            file,
        });
    }

    let basename = theme_path
        .file_name()
        .map(EncodedOs::from_os)
        .unwrap_or_default();
    if *theme != basename {
        diag(
            diags,
            format!(
                "theme \"{theme_text}\" cannot include path separators unless it is an absolute path"
            ),
        );
        return None;
    }

    // Iterate over the possible locations to try to find the
    // one that exists.
    let mut it = LocationIterator::new(resources_dir);
    while let Some((_, dir)) = it.next_location() {
        let path = EncodedOs::from_path(&dir.to_path().join(theme.to_os()));
        match File::open(path.to_path()) {
            Ok(file) => {
                if !check_regular(&file, theme_text, diags) {
                    return None;
                }
                return Some(OpenedTheme { path, file });
            }
            // Not an error, just continue to the next location.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // Anything else is an error we log and give up on.
            Err(err) => {
                diag(
                    diags,
                    format!(
                        "failed to load theme \"{theme_text}\" from the file \"{}\": {err}",
                        path.as_encoded()
                    ),
                );
                return None;
            }
        }
    }

    // Unlikely scenario: the theme doesn't exist. In this case, we reset
    // our iterator, reiterate over in order to build a better error message.
    it.reset();
    while let Some((_, dir)) = it.next_location() {
        let path = EncodedOs::from_path(&dir.to_path().join(theme.to_os()));
        diag(
            diags,
            format!(
                "theme \"{theme_text}\" not found, tried path \"{}\"",
                path.as_encoded()
            ),
        );
    }

    None
}

/// Open the given theme from an absolute path. If there are any errors
/// then messages will be appended to the given error list and None is
/// returned.
pub fn open_absolute(theme: &EncodedOs, diags: &mut DiagnosticList) -> Option<File> {
    let theme = theme.as_encoded();
    match File::open(EncodedOs::from_encoded(theme).to_path()) {
        Ok(f) => Some(f),
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                diag(
                    diags,
                    format!("failed to load theme from the path \"{theme}\""),
                );
            } else {
                diag(
                    diags,
                    format!("failed to load theme from the path \"{theme}\": {err}"),
                );
            }
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn e(s: &str) -> EncodedOs {
        EncodedOs::from_encoded(s)
    }

    #[test]
    fn theme_path_separators_rejected() {
        let mut d = DiagnosticList::default();
        assert!(open(&e("a/b"), None, &mut d).is_none());
        assert!(
            d.items()[0]
                .message
                .contains("cannot include path separators")
        );
    }

    #[test]
    fn theme_absolute_missing_and_dir() {
        let mut d = DiagnosticList::default();
        assert!(open(&e("/nonexistent/ghostty/theme"), None, &mut d).is_none());
        assert_eq!(
            d.items()[0].message,
            "failed to load theme from the path \"/nonexistent/ghostty/theme\""
        );
        let mut d = DiagnosticList::default();
        assert!(open(&e("/tmp"), None, &mut d).is_none());
        assert_eq!(
            d.items()[0].message,
            "not reading theme from \"/tmp\": it is a directory"
        );
    }

    #[test]
    fn theme_found_in_resources() {
        let dir = std::env::temp_dir().join(format!("ghostty-theme-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("themes")).unwrap();
        std::fs::write(dir.join("themes/mytheme-zzz"), "background = #000000\n").unwrap();
        let mut d = DiagnosticList::default();
        let t = open(&e("mytheme-zzz"), Some(&EncodedOs::from_path(&dir)), &mut d).unwrap();
        assert!(t.path.ends_with_plain("themes/mytheme-zzz"));
        assert!(d.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn theme_not_found_lists_tried_paths() {
        let mut d = DiagnosticList::default();
        assert!(open(&e("nope-zzz"), Some(&e("/nonexistent-res")), &mut d).is_none());
        assert!(d.items().iter().any(|x| x.message
            == "theme \"nope-zzz\" not found, tried path \"/nonexistent-res/themes/nope-zzz\""));
    }
}
