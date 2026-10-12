//! Path config values (port of `config/path.zig`).

use std::{
    fmt,
    path::{Component, Path as StdPath, PathBuf},
};

use crate::{
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
    Optional(String),

    /// The file is required to exist.
    Required(String),
}

impl Default for Path {
    fn default() -> Self {
        Path::Required(String::new())
    }
}

/// Lexically resolve `base/path` like `std.fs.path.resolve`.
fn resolve_lexical(base: &str, path: &str) -> String {
    let joined = PathBuf::from(base).join(path);
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
    out.to_string_lossy().into_owned()
}

impl Path {
    /// The inner path string.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Path::Optional(p) | Path::Required(p) => p,
        }
    }

    fn path_mut(&mut self) -> &mut String {
        match self {
            Path::Optional(p) | Path::Required(p) => p,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.path().len()
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
        let mut value = input.ok_or(ArgError::ValueRequired)?;

        if value.is_empty() {
            return Ok(None);
        }

        let optional = if let Some(rest) = value.strip_prefix('?') {
            value = rest;
            true
        } else {
            false
        };

        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            value = &value[1..value.len() - 1];
        }

        Ok(Some(if optional {
            Path::Optional(value.to_string())
        } else {
            Path::Required(value.to_string())
        }))
    }

    /// Expand relative paths or paths prefixed with `~/`. The path will be
    /// overwritten.
    ///
    /// `base` is the base directory to expand relative paths; it must be an
    /// absolute path. Errors are added to `diags` if they occur; an `Err` is
    /// only returned if the base directory cannot be opened.
    pub fn expand(&mut self, base: &str, diags: &mut DiagnosticList) -> std::io::Result<()> {
        assert!(StdPath::new(base).is_absolute());

        let path = self.path().to_string();

        // If it is already absolute we can ignore it.
        if path.is_empty() || StdPath::new(&path).is_absolute() {
            return Ok(());
        }

        // Check if the path starts with a tilde and expand it to the
        // home directory. We explicitly look for "~/" because we don't
        // support alternate users such as "~alice/"
        // (Windows isn't supported yet: Zig skips this there.)
        if !cfg!(windows) && path.starts_with("~/") {
            match ghostty_os::expand_home(&path) {
                Ok(expanded) => {
                    log::debug!("expanding file path from home directory: path={expanded}");
                    *self.path_mut() = expanded;
                }
                Err(err) => {
                    diags.append(Diagnostic {
                        message: format!("error expanding home directory for path {path}: {err}"),
                        ..Diagnostic::default()
                    });

                    // Blank this path so that we don't attempt to resolve it
                    // again
                    *self = Path::Required(String::new());
                }
            }
            return Ok(());
        }

        // Make sure the base directory can be opened.
        std::fs::read_dir(base)?;

        let full = StdPath::new(base).join(&path);
        let abs = match std::fs::canonicalize(&full) {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // The file doesn't exist. Try to resolve the relative path
                // another way.
                resolve_lexical(base, &path)
            }
            Err(err) => {
                diags.append(Diagnostic {
                    message: format!("error resolving file path {path}: {err}"),
                    ..Diagnostic::default()
                });

                // Blank this path so that we don't attempt to resolve it again
                *self = Path::Required(String::new());
                return Ok(());
            }
        };

        log::debug!("expanding file path relative={path} abs={abs}");
        *self.path_mut() = abs;
        Ok(())
    }
}

impl ConfigField for Path {
    /// Parse CLI option. The value will be overwritten.
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let Some(item) = Path::parse(input)? else {
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
            Path::Optional(p) => writeln!(f.writer, "{} = ?{p}", f.name),
            Path::Required(p) => writeln!(f.writer, "{} = {p}", f.name),
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
    pub fn expand(&mut self, base: &str, diags: &mut DiagnosticList) -> std::io::Result<()> {
        for path in &mut self.value {
            path.expand(base, diags)?;
        }
        Ok(())
    }
}

impl ConfigField for RepeatablePath {
    const KIND: crate::cli_args::FieldKind = crate::cli_args::FieldKind::RepeatablePath;

    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let Some(item) = Path::parse(input)? else {
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::formatter::format_entry;

    fn fmt<T: ConfigField>(v: &T) -> String {
        let mut s = String::new();
        format_entry("a", v, &mut s).unwrap();
        s
    }

    #[test]
    fn path_parse() {
        let p = |s| Path::parse(Some(s)).unwrap().unwrap();
        assert_eq!(p("config.1"), Path::Required("config.1".into()));
        assert_eq!(p("?config.2"), Path::Optional("config.2".into()));
        assert_eq!(p("\"?config.3\""), Path::Required("?config.3".into()));
        assert_eq!(p("?\"config.4\""), Path::Optional("config.4".into()));
        assert_eq!(p("?"), Path::Optional(String::new()));
        assert_eq!(p("\"\""), Path::Required(String::new()));
        assert_eq!(p("?\"\""), Path::Optional(String::new()));
        assert_eq!(Path::parse(None), Err(ArgError::ValueRequired));
        assert_eq!(Path::parse(Some("")), Ok(None));
    }

    #[test]
    fn path_parse_cli() {
        let mut item = Path::default();

        item.parse_cli(Some("config.1")).unwrap();
        assert_eq!(item, Path::Required("config.1".into()));

        item.parse_cli(Some("?config.2")).unwrap();
        assert_eq!(item, Path::Optional("config.2".into()));

        item.parse_cli(Some("\"?config.3\"")).unwrap();
        assert_eq!(item, Path::Required("?config.3".into()));

        // Zero-length values, ignored
        item.parse_cli(Some("?")).unwrap();
        assert_eq!(item, Path::Required("?config.3".into()));
        item.parse_cli(Some("\"\"")).unwrap();
        assert_eq!(item, Path::Required("?config.3".into()));
        item.parse_cli(Some("?\"\"")).unwrap();
        assert_eq!(item, Path::Required("?config.3".into()));
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

        assert_eq!(list.value[0], Path::Required("config.1".into()));
        assert_eq!(list.value[1], Path::Optional("config.2".into()));
        assert_eq!(list.value[2], Path::Required("?config.3".into()));

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
        let mut p = Path::Required("../nope/x.conf".into());
        p.expand("/tmp", &mut diags).unwrap();
        assert_eq!(p, Path::Required("/nope/x.conf".into()));

        let mut p = Path::Optional("/abs/path".into());
        p.expand("/tmp", &mut diags).unwrap();
        assert_eq!(p, Path::Optional("/abs/path".into()));
        assert!(diags.is_empty());
    }
}
