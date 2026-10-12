//! `ConfigField` implementations for types owned by `ghostty-input`
//! (the input crate cannot depend on the config crate).

use std::fmt;

use ghostty_input::{KeyRemapSet, OptionAsAlt};

use crate::{
    c_value::CValue,
    cli_args::{ArgError, ConfigField},
    formatter::EntryFormatter,
};

impl ConfigField for KeyRemapSet {
    fn parse_cli(&mut self, value: Option<&str>) -> Result<(), ArgError> {
        self.parse_cli(value).map_err(|_| ArgError::InvalidValue)
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        let entries = self.format_entries();
        if entries.is_empty() {
            return writeln!(f.writer, "{} = ", f.name);
        }
        for e in entries {
            f.format_entry(&e)?;
        }
        Ok(())
    }
}

impl ConfigField for OptionAsAlt {
    const VALID_VALUES: Option<&'static [&'static str]> = None;

    fn c_value(&self) -> CValue {
        CValue::EnumName(match self {
            OptionAsAlt::False => "false",
            OptionAsAlt::True => "true",
            OptionAsAlt::Left => "left",
            OptionAsAlt::Right => "right",
        })
    }

    fn parse_cli(&mut self, value: Option<&str>) -> Result<(), ArgError> {
        let v = value.ok_or(ArgError::ValueRequired)?;
        *self = match v {
            "false" => OptionAsAlt::False,
            "true" => OptionAsAlt::True,
            "left" => OptionAsAlt::Left,
            "right" => OptionAsAlt::Right,
            _ => return Err(ArgError::InvalidValue),
        };
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        let s = match self {
            OptionAsAlt::False => "false",
            OptionAsAlt::True => "true",
            OptionAsAlt::Left => "left",
            OptionAsAlt::Right => "right",
        };
        writeln!(f.writer, "{} = {}", f.name, s)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn fe<T: ConfigField>(name: &str, v: &T) -> String {
        let mut s = String::new();
        crate::formatter::format_entry(name, v, &mut s).unwrap();
        s
    }

    #[test]
    fn remap_set_format_entry_empty() {
        assert_eq!(fe("key-remap", &KeyRemapSet::default()), "key-remap = \n");
    }

    #[test]
    fn remap_set_format_entry_values() {
        let mut set = KeyRemapSet::default();
        ConfigField::parse_cli(&mut set, Some("ctrl=super")).unwrap();
        let out = fe("key-remap", &set);
        assert!(out.starts_with("key-remap = left_ctrl=left_super\n"));
    }

    #[test]
    fn option_as_alt_roundtrip() {
        let mut v = OptionAsAlt::default();
        v.parse_cli(Some("left")).unwrap();
        assert_eq!(fe("a", &v), "a = left\n");
        assert_eq!(v.parse_cli(Some("x")), Err(ArgError::InvalidValue));
    }
}
