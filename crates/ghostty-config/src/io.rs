//! Readable IO config values (port of `config/io.zig`).

use crate::cli_args::{ArgError, ConfigField, split_tagged_union};
use crate::formatter::EntryFormatter;
use crate::string;
use std::fmt;

/// ReadableIO is some kind of IO source that is readable.
///
/// It can be either a direct string or a filepath. The filepath will
/// be deferred and read later, so it won't be checked for existence
/// or readability at configuration time. This allows using a path that
/// might be produced in an intermediate state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadableIO {
    Raw(String),
    Path(String),
}

impl Default for ReadableIO {
    fn default() -> Self {
        ReadableIO::Raw(String::new())
    }
}

impl ReadableIO {
    /// Same as clone but also parses the values as Zig strings in
    /// the final resulting value all at once.
    #[must_use]
    pub fn clone_parsed(&self) -> Self {
        // Parsing can't fail because we validate it in parse_cli
        match self {
            ReadableIO::Raw(v) => ReadableIO::Raw(string::parse(v).unwrap_or_default()),
            ReadableIO::Path(v) => ReadableIO::Path(string::parse(v).unwrap_or_default()),
        }
    }
}

impl ConfigField for ReadableIO {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let input = input.ok_or(ArgError::ValueRequired)?;
        if input.is_empty() {
            return Err(ArgError::ValueRequired);
        }

        // We only validate that string parsing works. We store the value
        // as raw so that our formatting can recreate it.
        string::parse(input)?;

        // Next, parse the tagged union using normal rules. Invalid values in
        // the tagged union are interpreted as raw values.
        let (tag, rest) = split_tagged_union(input);
        *self = match tag {
            "raw" => ReadableIO::Raw(rest.to_string()),
            "path" => ReadableIO::Path(rest.to_string()),
            _ => ReadableIO::Raw(input.to_string()),
        };
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        let (tag, v) = match self {
            ReadableIO::Raw(v) => ("raw", v),
            ReadableIO::Path(v) => ("path", v),
        };
        writeln!(f.writer, "{} = {tag}:{v}", f.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepeatableReadableIO {
    pub list: Vec<ReadableIO>,
}

impl RepeatableReadableIO {
    /// See [`ReadableIO::clone_parsed`].
    #[must_use]
    pub fn clone_parsed(&self) -> Self {
        Self {
            list: self.list.iter().map(ReadableIO::clone_parsed).collect(),
        }
    }
}

impl ConfigField for RepeatableReadableIO {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let value = input.ok_or(ArgError::ValueRequired)?;

        // Empty value resets the list
        if value.is_empty() {
            self.list.clear();
            return Ok(());
        }

        let mut io = ReadableIO::default();
        io.parse_cli(Some(value))?;
        self.list.push(io);
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        if self.list.is_empty() {
            return writeln!(f.writer, "{} = ", f.name);
        }
        for value in &self.list {
            value.format_entry(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::formatter::format_entry;

    #[test]
    fn readable_io_parse_cli() {
        let mut io = ReadableIO::default();
        io.parse_cli(Some("foo")).unwrap();
        assert_eq!(io, ReadableIO::Raw("foo".into()));
        io.parse_cli(Some("raw:foo")).unwrap();
        assert_eq!(io, ReadableIO::Raw("foo".into()));
        io.parse_cli(Some("path:foo")).unwrap();
        assert_eq!(io, ReadableIO::Path("foo".into()));
    }

    #[test]
    fn readable_io_format_entry() {
        let mut v = ReadableIO::default();
        v.parse_cli(Some("raw:foo")).unwrap();
        let mut s = String::new();
        format_entry("a", &v, &mut s).unwrap();
        assert_eq!(s, "a = raw:foo\n");
    }

    #[test]
    fn repeatable_readable_io_parse_cli() {
        let mut list = RepeatableReadableIO::default();
        list.parse_cli(Some("raw:A")).unwrap();
        list.parse_cli(Some("path:B")).unwrap();
        assert_eq!(list.list.len(), 2);

        list.parse_cli(Some("")).unwrap();
        assert_eq!(list.list.len(), 0);
    }

    #[test]
    fn readable_io_errors_and_clone_parsed() {
        let mut io = ReadableIO::default();
        assert_eq!(io.parse_cli(None), Err(ArgError::ValueRequired));
        assert_eq!(io.parse_cli(Some("")), Err(ArgError::ValueRequired));
        assert!(io.parse_cli(Some("\\q")).is_err());
        io.parse_cli(Some("raw:a\\nb")).unwrap();
        assert_eq!(io.clone_parsed(), ReadableIO::Raw("a\nb".into()));
    }
}
