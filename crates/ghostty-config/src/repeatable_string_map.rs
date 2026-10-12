//! RepeatableStringMap is a key/value that can be repeated to accumulate a
//! string map. This isn't called "StringMap" because I find that sometimes
//! leads to confusion that it _accepts_ a map such as JSON dict.

use std::fmt;

use crate::{
    cli_args::{ArgError, ConfigField},
    formatter::EntryFormatter,
};

/// std.ascii.whitespace
fn is_ascii_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

/// An insertion-ordered string map.
#[derive(Debug, Clone, Default)]
pub struct RepeatableStringMap {
    map: Vec<(String, String)>,
}

impl PartialEq for RepeatableStringMap {
    /// Order-insensitive comparison (Zig: `equal`).
    fn eq(&self, other: &Self) -> bool {
        self.map.len() == other.map.len()
            && self
                .map
                .iter()
                .all(|(k, v)| other.get(k).is_some_and(|ov| ov == v))
    }
}

impl RepeatableStringMap {
    /// The number of items in the map
    #[must_use]
    pub fn count(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Iterator over the entries in the map (insertion order).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl ConfigField for RepeatableStringMap {
    fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ArgError> {
        let value = input.ok_or(ArgError::ValueRequired)?;

        // Empty value resets the list.
        if value.is_empty() {
            self.map.clear();
            return Ok(());
        }

        let index = value.find('=').ok_or(ArgError::ValueRequired)?;
        let key = value[..index].trim_matches(is_ascii_ws);
        let val = value[index + 1..].trim_matches(is_ascii_ws);

        // Empty value removes the key from the map.
        if val.is_empty() {
            if let Some(pos) = self.map.iter().position(|(k, _)| k == key) {
                self.map.remove(pos);
            }
            return Ok(());
        }

        match self.map.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = val.to_string(),
            None => self.map.push((key.to_string(), val.to_string())),
        }
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        // If no items, we want to render an empty field.
        if self.map.is_empty() {
            return writeln!(f.writer, "{} = ", f.name);
        }
        for (k, v) in &self.map {
            writeln!(f.writer, "{} = {k}={v}", f.name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::formatter::format_entry;

    fn fmt(m: &RepeatableStringMap) -> String {
        let mut s = String::new();
        format_entry("a", m, &mut s).unwrap();
        s
    }

    #[test]
    fn repeatable_string_map_parse_cli() {
        let mut map = RepeatableStringMap::default();

        assert_eq!(map.parse_cli(Some("A")), Err(ArgError::ValueRequired));

        map.parse_cli(Some("A=B")).unwrap();
        map.parse_cli(Some("B=C")).unwrap();
        assert_eq!(map.count(), 2);

        map.parse_cli(Some("")).unwrap();
        assert_eq!(map.count(), 0);

        map.parse_cli(Some("A=B")).unwrap();
        assert_eq!(map.count(), 1);
        map.parse_cli(Some("A=C")).unwrap();
        assert_eq!(map.count(), 1);
        assert_eq!(map.get("A"), Some("C"));

        // Empty value removes the key
        map.parse_cli(Some("A=")).unwrap();
        assert_eq!(map.count(), 0);
    }

    #[test]
    fn repeatable_string_map_format_config_empty() {
        assert_eq!(fmt(&RepeatableStringMap::default()), "a = \n");
    }

    #[test]
    fn repeatable_string_map_format_config_single_item() {
        let mut map = RepeatableStringMap::default();
        map.parse_cli(Some("A=B")).unwrap();
        assert_eq!(fmt(&map), "a = A=B\n");

        let mut map = RepeatableStringMap::default();
        map.parse_cli(Some(" A = B ")).unwrap();
        assert_eq!(fmt(&map), "a = A=B\n");
    }

    #[test]
    fn repeatable_string_map_format_config_multiple_items() {
        let mut list = RepeatableStringMap::default();
        list.parse_cli(Some("A=B")).unwrap();
        list.parse_cli(Some("B = C")).unwrap();
        assert_eq!(fmt(&list), "a = A=B\na = B=C\n");
    }

    #[test]
    fn repeatable_string_map_equal_is_order_insensitive() {
        let mut a = RepeatableStringMap::default();
        a.parse_cli(Some("A=B")).unwrap();
        a.parse_cli(Some("C=D")).unwrap();
        let mut b = RepeatableStringMap::default();
        b.parse_cli(Some("C=D")).unwrap();
        b.parse_cli(Some("A=B")).unwrap();
        assert_eq!(a, b);
    }
}
