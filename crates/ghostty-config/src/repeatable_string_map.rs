//! RepeatableStringMap is a key/value that can be repeated to accumulate a
//! string map. This isn't called "StringMap" because I find that sometimes
//! leads to confusion that it _accepts_ a map such as JSON dict.

use std::fmt;

use ghostty_os::osbytes::EncodedOs;

use crate::{
    cli_args::{ArgError, ConfigField},
    formatter::EntryFormatter,
};

/// An insertion-ordered string map.
#[derive(Debug, Clone, Default)]
pub struct RepeatableStringMap {
    map: Vec<(EncodedOs, EncodedOs)>,
}

impl PartialEq for RepeatableStringMap {
    /// Order-insensitive comparison (Zig: `equal`).
    fn eq(&self, other: &Self) -> bool {
        self.map.len() == other.map.len()
            && self.map.iter().all(|(k, v)| {
                other
                    .map
                    .iter()
                    .find(|(key, _)| key == k)
                    .map(|(_, v)| v)
                    .is_some_and(|ov| ov == v)
            })
    }
}

impl RepeatableStringMap {
    /// The number of items in the map
    #[must_use]
    pub fn count(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&EncodedOs> {
        self.map
            .iter()
            .find(|(k, _)| k == &EncodedOs::from_plain_text(key))
            .map(|(_, v)| v)
    }

    /// Insert or replace an entry (Zig: `map.put`).
    pub fn put(&mut self, key: &str, value: &str) {
        self.put_bytes(key.as_bytes(), value.as_bytes());
    }

    pub fn put_bytes(&mut self, key: &[u8], value: &[u8]) {
        let key = EncodedOs::from_bytes(key);
        let value = EncodedOs::from_bytes(value);
        match self.map.iter_mut().find(|(k, _)| k == &key) {
            Some((_, v)) => *v = value,
            None => self.map.push((key, value)),
        }
    }

    /// Iterator over the entries in the map (insertion order).
    pub fn iter(&self) -> impl Iterator<Item = (&EncodedOs, &EncodedOs)> {
        self.map.iter().map(|(k, v)| (k, v))
    }
}

impl ConfigField for RepeatableStringMap {
    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let value = input.ok_or(ArgError::ValueRequired)?;

        // Empty value resets the list.
        if value.is_empty() {
            self.map.clear();
            return Ok(());
        }

        let index = value
            .iter()
            .position(|b| *b == b'=')
            .ok_or(ArgError::ValueRequired)?;
        let key = EncodedOs::from_bytes(crate::cli_args::trim_ascii_whitespace(&value[..index]));
        let val =
            EncodedOs::from_bytes(crate::cli_args::trim_ascii_whitespace(&value[index + 1..]));

        // Empty value removes the key from the map.
        if val.is_empty() {
            if let Some(pos) = self.map.iter().position(|(k, _)| k == &key) {
                self.map.remove(pos);
            }
            return Ok(());
        }

        match self.map.iter_mut().find(|(k, _)| k == &key) {
            Some(entry) => entry.1 = val,
            None => self.map.push((key, val)),
        }
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        // If no items, we want to render an empty field.
        if self.map.is_empty() {
            return writeln!(f.writer, "{} = ", f.name);
        }
        for (k, v) in &self.map {
            writeln!(
                f.writer,
                "{} = {}={}",
                f.name,
                k.as_encoded(),
                v.as_encoded()
            )?;
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
        assert_eq!(map.get("A").map(EncodedOs::as_encoded), Some("C"));

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
