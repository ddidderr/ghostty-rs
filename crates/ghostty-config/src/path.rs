//! Path config values (port of `config/path.zig`).

use std::{
    fmt,
    path::{Component, PathBuf},
};

use ghostty_os::osbytes::EncodedOs;

use crate::{
    c_value::CValue,
    cli_args::{ArgError, ConfigField},
    diagnostics::{Diagnostic, DiagnosticList},
    formatter::EntryFormatter,
};

/// Path is like a string that represents a path value. The difference is that
/// when loading the configuration the value for this will be automatically
/// expanded relative to the path of the config file (or the home directory).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Path {
    /// No error if the file does not exist.
    Optional(EncodedOs),

    /// The file is required to exist.
    Required(EncodedOs),
}

impl Default for Path {
    fn default() -> Self {
        Path::Required(EncodedOs::new())
    }
}

/// Lexically resolve `base/path` like `std.fs.path.resolve`.
fn resolve_lexical(base: &EncodedOs, path: &EncodedOs) -> std::io::Result<EncodedOs> {
    let joined = base.to_path()?.join(path.to_path()?);
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(EncodedOs::from_path(&out))
}

impl Path {
    /// The inner path string.
    #[must_use]
    pub fn path(&self) -> &EncodedOs {
        match self {
            Path::Optional(p) | Path::Required(p) => p,
        }
    }

    fn path_mut(&mut self) -> &mut EncodedOs {
        match self {
            Path::Optional(p) | Path::Required(p) => p,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.path().is_empty()
    }

    /// Parse the input and return a Path. A leading `?` indicates that the path
    /// is _optional_ and an error should not be logged or displayed to the user
    /// if that path does not exist. Otherwise the path is required and an error
    /// should be logged if the path does not exist.
    ///
    /// Returns `Ok(None)` for an empty input.
    pub fn parse(input: Option<&str>) -> Result<Option<Path>, ArgError> {
        Self::parse_bytes(input.map(str::as_bytes))
    }

    pub fn parse_bytes(input: Option<&[u8]>) -> Result<Option<Path>, ArgError> {
        let mut value = input.ok_or(ArgError::ValueRequired)?;

        if value.is_empty() {
            return Ok(None);
        }

        let optional = if let Some(rest) = value.strip_prefix(b"?") {
            value = rest;
            true
        } else {
            false
        };

        if value.len() >= 2 && value.starts_with(b"\"") && value.ends_with(b"\"") {
            value = &value[1..value.len() - 1];
        }

        Ok(Some(if optional {
            Path::Optional(EncodedOs::from_bytes(value))
        } else {
            Path::Required(EncodedOs::from_bytes(value))
        }))
    }

    /// Expand relative paths or paths prefixed with `~/`. The path will be
    /// overwritten.
    ///
    /// `base` is the base directory to expand relative paths; it must be an
    /// absolute path. Errors are added to `diags` if they occur; an `Err` is
    /// only returned if the base directory cannot be opened.
    pub fn expand(&mut self, base: &EncodedOs, diags: &mut DiagnosticList) -> std::io::Result<()> {
        // Filesystem calls use the decoded OS paths.
        let base_path = base.to_path()?;
        assert!(base.is_zig_absolute());

        let path = self.path().clone();

        // If it is already absolute we can ignore it.
        if path.is_empty() || {
            path.to_path()?;
            path.is_zig_absolute()
        } {
            return Ok(());
        }

        // Check if the path starts with a tilde and expand it to the
        // home directory. We explicitly look for "~/" because we don't
        // support alternate users such as "~alice/"
        // (Windows isn't supported yet: Zig skips this there.)
        if !cfg!(windows) && path.starts_with_plain("~/") {
            match ghostty_os::expand_home(&path) {
                Ok(expanded) => {
                    log::debug!(
                        "expanding file path from home directory: path={}",
                        expanded.to_text_lossy()
                    );
                    *self.path_mut() = expanded;
                }
                Err(err) => {
                    diags.append(Diagnostic {
                        message: format!(
                            "error expanding home directory for path {}: {err}",
                            path.as_encoded()
                        ),
                        ..Diagnostic::default()
                    });

                    // Blank this path so that we don't attempt to resolve it
                    // again
                    *self = Path::Required(EncodedOs::new());
                }
            }
            return Ok(());
        }

        // Make sure the base directory can be opened.
        std::fs::read_dir(&base_path)?;

        let full = base_path.join(path.to_syscall_path()?);
        let abs = match realpath(&full) {
            Ok(p) => EncodedOs::from_path(&p),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // The file doesn't exist. Try to resolve the relative path
                // another way.
                resolve_lexical(base, &path)?
            }
            Err(err) => {
                diags.append(Diagnostic {
                    message: format!(
                        "error resolving file path {}: {}",
                        path.as_encoded(),
                        ghostty_os::zig_error::io_error_name(&err)
                    ),
                    ..Diagnostic::default()
                });

                // Blank this path so that we don't attempt to resolve it again
                *self = Path::Required(EncodedOs::new());
                return Ok(());
            }
        };

        log::debug!(
            "expanding file path relative={} abs={}",
            path.to_text_lossy(),
            abs.to_text_lossy()
        );
        *self.path_mut() = abs;
        Ok(())
    }
}

impl ConfigField for Path {
    fn c_value(&self) -> CValue {
        match self {
            Path::Optional(p) => CValue::Path {
                path: p.as_encoded().to_owned(),
                optional: true,
            },
            Path::Required(p) => CValue::Path {
                path: p.as_encoded().to_owned(),
                optional: false,
            },
        }
    }

    /// Parse CLI option. The value will be overwritten.
    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let Some(item) = Path::parse_bytes(input)? else {
            return Ok(());
        };
        if item.is_empty() {
            return Ok(());
        }
        *self = item;
        Ok(())
    }

    /// Used by formatter.
    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        match self {
            Path::Optional(p) => writeln!(f.writer, "{} = ?{}", f.name, p.as_encoded()),
            Path::Required(p) => writeln!(f.writer, "{} = {}", f.name, p.as_encoded()),
        }
    }
}

/// RepeatablePath is like repeatable string but represents a path value. The
/// difference is that when loading the configuration any values for this will
/// be automatically expanded relative to the path of the config file (or the home
/// directory).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepeatablePath {
    pub value: Vec<Path>,
}

impl RepeatablePath {
    /// Expand all the paths relative to the base directory.
    pub fn expand(&mut self, base: &EncodedOs, diags: &mut DiagnosticList) -> std::io::Result<()> {
        for path in &mut self.value {
            path.expand(base, diags)?;
        }
        Ok(())
    }
}

impl ConfigField for RepeatablePath {
    const KIND: crate::cli_args::FieldKind = crate::cli_args::FieldKind::RepeatablePath;

    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let Some(item) = Path::parse_bytes(input)? else {
            self.value.clear();
            return Ok(());
        };

        if item.is_empty() {
            // This handles the case of zero length paths after removing any ?
            // prefixes or surrounding quotes. In this case, we don't reset the
            // list.
            return Ok(());
        }

        self.value.push(item);
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        if self.value.is_empty() {
            return writeln!(f.writer, "{} = ", f.name);
        }
        for item in &self.value {
            item.format_entry(f)?;
        }
        Ok(())
    }
}

/// `posix.realpath`: the canonical path without the `\\?\` verbatim prefix
/// `std::fs::canonicalize` adds on Windows (Zig's `realpath` has none).
#[cfg(windows)]
pub(crate) fn realpath(path: impl AsRef<std::path::Path>) -> std::io::Result<std::path::PathBuf> {
    ghostty_os::path::realpath(path)
}

#[cfg(not(windows))]
pub(crate) fn realpath(path: impl AsRef<std::path::Path>) -> std::io::Result<std::path::PathBuf> {
    std::fs::canonicalize(path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::formatter::format_entry;

    fn e(s: &str) -> EncodedOs {
        EncodedOs::from_encoded(s)
    }

    fn fmt<T: ConfigField>(v: &T) -> String {
        let mut s = String::new();
        format_entry("a", v, &mut s).unwrap();
        s
    }

    #[test]
    fn path_parse() {
        let p = |s| Path::parse(Some(s)).unwrap().unwrap();
        assert_eq!(p("config.1"), Path::Required(e("config.1")));
        assert_eq!(p("?config.2"), Path::Optional(e("config.2")));
        assert_eq!(p("\"?config.3\""), Path::Required(e("?config.3")));
        assert_eq!(p("?\"config.4\""), Path::Optional(e("config.4")));
        assert_eq!(p("?"), Path::Optional(EncodedOs::new()));
        assert_eq!(p("\"\""), Path::Required(EncodedOs::new()));
        assert_eq!(p("?\"\""), Path::Optional(EncodedOs::new()));
        assert_eq!(Path::parse(None), Err(ArgError::ValueRequired));
        assert_eq!(Path::parse(Some("")), Ok(None));
    }

    #[test]
    fn path_parse_cli() {
        let mut item = Path::default();

        item.parse_cli(Some("config.1")).unwrap();
        assert_eq!(item, Path::Required(e("config.1")));

        item.parse_cli(Some("?config.2")).unwrap();
        assert_eq!(item, Path::Optional(e("config.2")));

        item.parse_cli(Some("\"?config.3\"")).unwrap();
        assert_eq!(item, Path::Required(e("?config.3")));

        // Zero-length values, ignored
        item.parse_cli(Some("?")).unwrap();
        assert_eq!(item, Path::Required(e("?config.3")));
        item.parse_cli(Some("\"\"")).unwrap();
        assert_eq!(item, Path::Required(e("?config.3")));
        item.parse_cli(Some("?\"\"")).unwrap();
        assert_eq!(item, Path::Required(e("?config.3")));
    }

    #[test]
    fn path_format_config_single_item() {
        let mut item = Path::default();
        item.parse_cli(Some("A")).unwrap();
        assert_eq!(fmt(&item), "a = A\n");
    }

    #[test]
    fn path_format_config_multiple_items() {
        let mut item = Path::default();
        item.parse_cli(Some("A")).unwrap();
        item.parse_cli(Some("?B")).unwrap();
        assert_eq!(fmt(&item), "a = ?B\n");
    }

    #[test]
    fn repeatable_path_parse_cli() {
        let mut list = RepeatablePath::default();
        list.parse_cli(Some("config.1")).unwrap();
        list.parse_cli(Some("?config.2")).unwrap();
        list.parse_cli(Some("\"?config.3\"")).unwrap();

        assert_eq!(list.value.len(), 3);

        // Zero-length values, ignored
        list.parse_cli(Some("?")).unwrap();
        list.parse_cli(Some("\"\"")).unwrap();

        assert_eq!(list.value.len(), 3);

        assert_eq!(list.value[0], Path::Required(e("config.1")));
        assert_eq!(list.value[1], Path::Optional(e("config.2")));
        assert_eq!(list.value[2], Path::Required(e("?config.3")));

        list.parse_cli(Some("")).unwrap();
        assert_eq!(list.value.len(), 0);
    }

    #[test]
    fn repeatable_path_format_config_empty() {
        assert_eq!(fmt(&RepeatablePath::default()), "a = \n");
    }

    #[test]
    fn repeatable_path_format_config_single_item() {
        let mut list = RepeatablePath::default();
        list.parse_cli(Some("A")).unwrap();
        assert_eq!(fmt(&list), "a = A\n");
    }

    #[test]
    fn repeatable_path_format_config_multiple_items() {
        let mut list = RepeatablePath::default();
        list.parse_cli(Some("A")).unwrap();
        list.parse_cli(Some("?B")).unwrap();
        assert_eq!(fmt(&list), "a = A\na = ?B\n");
    }

    #[test]
    fn path_expand_relative_nonexistent_and_absolute() {
        let mut diags = DiagnosticList::default();
        let mut p = Path::Required(e("../nope/x.conf"));
        p.expand(&e("/tmp"), &mut diags).unwrap();
        // Resolved lexically with the platform's separator (`\nope\x.conf`
        // on Windows, like Zig's `std.fs.path.resolve`).
        let expected = if cfg!(windows) {
            "\\nope\\x.conf"
        } else {
            "/nope/x.conf"
        };
        assert_eq!(p, Path::Required(e(expected)));

        let mut p = Path::Optional(e("/abs/path"));
        p.expand(&e("/tmp"), &mut diags).unwrap();
        assert_eq!(p, Path::Optional(e("/abs/path")));
        assert!(diags.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn expand_interior_nul_resolves_the_c_string_prefix_like_zig() {
        let tmp = ghostty_os::TempDir::init().unwrap();
        let base = realpath(tmp.path()).unwrap();
        std::fs::write(base.join("exists"), b"x").unwrap();

        // The syscall sees only the prefix, which exists: the real path of
        // the prefix replaces the value.
        let mut p = Path::Required(EncodedOs::from_bytes(b"exists\0tail"));
        let mut diags = DiagnosticList::default();
        p.expand(&EncodedOs::from_path(&base), &mut diags).unwrap();
        assert_eq!(diags.items().len(), 0);
        assert_eq!(p.path().to_path().unwrap(), base.join("exists"));

        // The prefix is missing: the whole value is resolved lexically.
        let mut p = Path::Required(EncodedOs::from_bytes(b"missing\0tail"));
        p.expand(&EncodedOs::from_path(&base), &mut diags).unwrap();
        assert_eq!(diags.items().len(), 0);
        assert_eq!(
            p.path().to_bytes(),
            [base.as_os_str().as_encoded_bytes(), b"/missing\0tail"].concat()
        );
    }

    #[cfg(unix)]
    #[test]
    fn expand_symlink_to_non_utf8_target_round_trips() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = ghostty_os::TempDir::init().unwrap();
        let base = realpath(tmp.path()).unwrap();
        let target = base.join(std::ffi::OsStr::from_bytes(b"t\xffarget"));
        std::fs::write(&target, b"x").unwrap();
        std::os::unix::fs::symlink(&target, base.join("link")).unwrap();

        let mut p = Path::Required(e("link"));
        let mut diags = DiagnosticList::default();
        p.expand(&EncodedOs::from_path(&base), &mut diags).unwrap();
        assert_eq!(diags.items().len(), 0);
        // The canonical (non-UTF-8) target is preserved byte for byte.
        assert_eq!(p.path().to_path().unwrap(), target);
        assert!(std::fs::metadata(p.path().to_path().unwrap()).is_ok());
    }
}
