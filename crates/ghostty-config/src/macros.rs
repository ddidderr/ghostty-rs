//! Declarative macros used to define config structs, enums and packed
//! (bitfield-like) structs with Zig-compatible names and parsing.

use std::fmt;

/// Compute the help string for a field from its raw doc lines
/// (Zig: `helpgen.zig` `extractDocComments`). The `///` prefix has already
/// been removed (rustc's `#[doc]` value keeps the leading space just like the
/// Zig token slice `[3..]`). The common leading-space prefix is stripped and
/// the lines are joined with `\n`.
#[must_use]
pub fn help_string(lines: &[&str]) -> String {
    let mut m = usize::MAX;
    for line in lines {
        let n = line.bytes().position(|c| c != b' ').unwrap_or(usize::MAX);
        m = m.min(n);
    }
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&line[m.min(line.len())..]);
    }
    out
}

/// Metadata and accessors for a single field of a config struct.
pub struct FieldInfo<T> {
    /// The field name as used in config files (e.g. `font-family`).
    pub name: &'static str,
    /// Raw doc comment lines.
    pub doc: &'static [&'static str],
    /// Deep equality of this field between two structs.
    pub eq: fn(&T, &T) -> bool,
    /// Format this field as in `+show-config`.
    pub format: fn(&T, &str, &mut dyn fmt::Write) -> fmt::Result,
}

impl<T> FieldInfo<T> {
    /// The help text for this field (empty string if undocumented).
    #[must_use]
    pub fn help(&self) -> String {
        help_string(self.doc)
    }
}

/// Implemented by structs declared with [`config_struct!`](crate::config_struct).
pub trait ConfigStruct: Sized + 'static {
    const FIELDS: &'static [FieldInfo<Self>];
}

/// Declare a configuration struct.
///
/// ```ignore
/// config_struct! {
///     pub struct Foo {
///         /// doc comment
///         "some-key" => some_key: bool = false,
///     }
///     extra { diags: DiagnosticList = DiagnosticList::default(), }
/// }
/// ```
///
/// Generates the struct (all fields `pub`), `raw_default()`,
/// `parse_field(key, value)` (Zig: `parseIntoField`), `valid_values_for(key)`,
/// and a [`ConfigStruct`] impl with per-field metadata.
#[macro_export]
macro_rules! config_struct {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[doc = $doc:literal])*
                $key:literal => $field:ident : $ty:ty = $default:expr
            ),* $(,)?
        }
        $(
            extra {
                $( $(#[$emeta:meta])* $efield:ident : $ety:ty = $edefault:expr ),* $(,)?
            }
        )?
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug)]
        #[allow(dead_code)]
        $vis struct $name {
            $( pub $field: $ty, )*
            $( $( $(#[$emeta])* pub $efield: $ety, )* )?
        }

        impl $name {
            /// The struct with every field set to its declared default.
            #[allow(clippy::too_many_lines, dead_code)]
            pub fn raw_default() -> Self {
                Self {
                    $( $field: $default, )*
                    $( $( $efield: $edefault, )* )?
                }
            }

            #[allow(dead_code)]
            /// Parse a single key/value into the matching field. Empty
            /// values reset the field to its default.
            pub fn parse_field(
                &mut self,
                key: &str,
                value: ::core::option::Option<&str>,
            ) -> ::core::result::Result<(), $crate::cli_args::ArgError> {
                match key {
                    $(
                        $key => {
                            if value == ::core::option::Option::Some("") {
                                $crate::cli_args::ConfigField::reset(&mut self.$field, $default);
                                return ::core::result::Result::Ok(());
                            }
                            $crate::cli_args::ConfigField::parse_cli(&mut self.$field, value)
                        }
                    )*
                    _ => ::core::result::Result::Err($crate::cli_args::ArgError::InvalidField),
                }
            }

            /// For enum-typed fields, the valid values.
            #[allow(dead_code)]
            pub fn valid_values_for(key: &str) -> ::core::option::Option<&'static [&'static str]> {
                match key {
                    $(
                        $key => <$ty as $crate::cli_args::ConfigField>::VALID_VALUES,
                    )*
                    _ => ::core::option::Option::None,
                }
            }
        }

        impl $crate::macros::ConfigStruct for $name {
            const FIELDS: &'static [$crate::macros::FieldInfo<$name>] = &[
                $(
                    $crate::macros::FieldInfo {
                        name: $key,
                        doc: &[$($doc),*],
                        eq: |a: &$name, b: &$name| a.$field == b.$field,
                        format: |a: &$name, name: &str, w: &mut dyn ::core::fmt::Write| {
                            let mut f = $crate::formatter::EntryFormatter::new(name, w);
                            $crate::cli_args::ConfigField::format_entry(&a.$field, &mut f)
                        },
                    },
                )*
            ];
        }
    };
}

/// Declare a Zig-style enum whose variants parse from / print as the given
/// strings.
#[macro_export]
macro_rules! config_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[doc = $doc:literal])*
                $variant:ident => $str:literal
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[allow(clippy::enum_variant_names)]
        $vis enum $name {
            $( $(#[doc = $doc])* $variant, )*
        }

        impl $name {
            /// All variants in declaration order.
            pub const ALL: &'static [$name] = &[ $( $name::$variant, )* ];
            /// All variant names in declaration order.
            pub const NAMES: &'static [&'static str] = &[ $( $str, )* ];

            /// The user-facing name of this variant.
            #[must_use]
            pub fn as_str(&self) -> &'static str {
                match self { $( $name::$variant => $str, )* }
            }

            /// Zig: `std.meta.stringToEnum`.
            #[must_use]
            pub fn from_name(s: &str) -> ::core::option::Option<Self> {
                match s {
                    $( $str => ::core::option::Option::Some($name::$variant), )*
                    _ => ::core::option::Option::None,
                }
            }
        }

        impl ::core::default::Default for $name {
            fn default() -> Self {
                Self::ALL[0]
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl $crate::cli_args::ConfigField for $name {
            const VALID_VALUES: ::core::option::Option<&'static [&'static str]> =
                ::core::option::Option::Some(Self::NAMES);

            fn parse_cli(
                &mut self,
                value: ::core::option::Option<&str>,
            ) -> ::core::result::Result<(), $crate::cli_args::ArgError> {
                let v = value.ok_or($crate::cli_args::ArgError::ValueRequired)?;
                *self = Self::from_name(v).ok_or($crate::cli_args::ArgError::InvalidValue)?;
                ::core::result::Result::Ok(())
            }

            fn format_entry(
                &self,
                f: &mut $crate::formatter::EntryFormatter<'_>,
            ) -> ::core::fmt::Result {
                ::core::writeln!(f.writer, "{} = {}", f.name, self.as_str())
            }
        }
    };
}

/// Declare a Zig `packed struct` of bools: parsed from a comma separated
/// list of names (optionally prefixed with `no-`) or a plain boolean.
#[macro_export]
macro_rules! config_packed {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[doc = $doc:literal])*
                $key:literal => $field:ident : $default:expr
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        $vis struct $name {
            $( $(#[doc = $doc])* pub $field: bool, )*
        }

        impl $name {
            /// Field names (user-facing) in declaration order.
            pub const NAMES: &'static [&'static str] = &[ $( $key, )* ];

            fn set_index(&mut self, idx: usize, value: bool) {
                let mut i = 0usize;
                $(
                    if idx == i { self.$field = value; return; }
                    i += 1;
                )*
                let _ = i;
            }
        }

        impl ::core::default::Default for $name {
            fn default() -> Self {
                Self { $( $field: $default, )* }
            }
        }

        impl $crate::cli_args::ConfigField for $name {
            fn parse_cli(
                &mut self,
                value: ::core::option::Option<&str>,
            ) -> ::core::result::Result<(), $crate::cli_args::ArgError> {
                let v = value.ok_or($crate::cli_args::ArgError::ValueRequired)?;
                let mut result = <$name as ::core::default::Default>::default();
                $crate::cli_args::parse_packed_struct_with(v, Self::NAMES, |i, b| {
                    result.set_index(i, b);
                })?;
                *self = result;
                ::core::result::Result::Ok(())
            }

            fn format_entry(
                &self,
                f: &mut $crate::formatter::EntryFormatter<'_>,
            ) -> ::core::fmt::Result {
                ::core::write!(f.writer, "{} = ", f.name)?;
                let mut _first = true;
                $(
                    if !_first { ::core::write!(f.writer, ",")?; }
                    _first = false;
                    ::core::write!(f.writer, "{}{}", if self.$field { "" } else { "no-" }, $key)?;
                )*
                ::core::writeln!(f.writer)
            }
        }
    };
}
