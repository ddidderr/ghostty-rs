//! Config formatting (port of `config/formatter.zig`). Output matches
//! Ghostty's `+show-config`.

use crate::cli_args::ConfigField;
use std::fmt;

/// A single entry formatter for the given field name and writer.
pub struct EntryFormatter<'a> {
    pub name: &'a str,
    pub writer: &'a mut dyn fmt::Write,
}

impl<'a> EntryFormatter<'a> {
    pub fn new(name: &'a str, writer: &'a mut dyn fmt::Write) -> Self {
        Self { name, writer }
    }

    /// Format a value with this formatter's name.
    pub fn format_entry<T: ConfigField>(&mut self, value: &T) -> fmt::Result {
        value.format_entry(self)
    }
}

/// Format a single value with the given name.
pub fn format_entry<T: ConfigField>(
    name: &str,
    value: &T,
    writer: &mut dyn fmt::Write,
) -> fmt::Result {
    let mut f = EntryFormatter::new(name, writer);
    value.format_entry(&mut f)
}

/// `void` values (Zig) print as `name = \n`.
pub fn format_void(name: &str, writer: &mut dyn fmt::Write) -> fmt::Result {
    writeln!(writer, "{name} = ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config_packed;

    fn fe<T: ConfigField>(name: &str, v: &T) -> String {
        let mut s = String::new();
        format_entry(name, v, &mut s).unwrap();
        s
    }

    #[test]
    fn format_entry_bool() {
        assert_eq!(fe("a", &true), "a = true\n");
        assert_eq!(fe("a", &false), "a = false\n");
    }

    #[test]
    fn format_entry_int() {
        assert_eq!(fe("a", &123u8), "a = 123\n");
    }

    #[test]
    fn format_entry_float() {
        assert_eq!(fe("a", &0.7f64), "a = 0.7\n");
    }

    #[test]
    fn format_entry_enum() {
        crate::config_enum! { enum E { One => "one", Two => "two", Three => "three" } }
        assert_eq!(fe("a", &E::Two), "a = two\n");
    }

    #[test]
    fn format_entry_void() {
        let mut s = String::new();
        format_void("a", &mut s).unwrap();
        assert_eq!(s, "a = \n");
    }

    #[test]
    fn format_entry_optional() {
        assert_eq!(fe("a", &None::<bool>), "a = \n");
        assert_eq!(fe("a", &Some(false)), "a = false\n");
    }

    #[test]
    fn format_entry_string() {
        assert_eq!(fe("a", &"hello".to_string()), "a = hello\n");
    }

    #[test]
    fn format_entry_packed_struct() {
        config_packed! { struct V { "one" => one: true, "two" => two: false } }
        assert_eq!(fe("a", &V::default()), "a = one,no-two\n");
    }
}
