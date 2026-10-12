//! `FileFormatter` (part of `config/formatter.zig`): outputs the config in a
//! file-like format, identical to Ghostty's `+show-config`.

use std::fmt;

use crate::{
    config::Config,
    key::Key,
    macros::{ConfigStruct, help_string},
};

/// FileFormatter is a formatter implementation that outputs the
/// config in a file-like format. This uses more generous whitespace,
/// can include comments, etc.
pub struct FileFormatter<'a> {
    pub config: &'a Config,
    /// Include comments for documentation of each key
    pub docs: bool,
    /// Only include changed values from the default.
    pub changed: bool,
}

impl<'a> FileFormatter<'a> {
    #[must_use]
    pub fn new(config: &'a Config) -> Self {
        Self {
            config,
            docs: false,
            changed: false,
        }
    }
}

impl fmt::Display for FileFormatter<'_> {
    fn fmt(&self, writer: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.format(writer)
    }
}

impl FileFormatter<'_> {
    pub fn format(&self, writer: &mut dyn fmt::Write) -> fmt::Result {
        // If we're change-tracking then we need the default config to
        // compare against.
        let default = if self.changed {
            Some(Config::default())
        } else {
            None
        };

        for field in Config::FIELDS {
            let do_format = match &default {
                Some(d) => d.changed(self.config, Key(field.name)),
                None => true,
            };

            if do_format {
                let do_docs = self.docs && !field.doc.is_empty();
                if do_docs {
                    let help = help_string(field.doc);
                    for line in help.split('\n') {
                        writeln!(writer, "# {line}")?;
                    }
                }

                (field.format)(self.config, field.name, writer)?;

                if do_docs {
                    writeln!(writer)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::field_reassign_with_default,
    clippy::assert_is_empty
)]
mod tests {
    use super::*;

    #[test]
    fn format_default_config() {
        let cfg = Config::default();
        let mut buf = String::new();
        FileFormatter::new(&cfg).format(&mut buf).unwrap();
        assert!(!buf.is_empty());
    }

    #[test]
    fn format_default_config_changed() {
        let mut cfg = Config::default();
        cfg.font_size = 42.0;
        let mut buf = String::new();
        let f = FileFormatter {
            config: &cfg,
            docs: false,
            changed: true,
        };
        f.format(&mut buf).unwrap();
        assert!(buf.contains("font-size = 42\n"));
    }
}
