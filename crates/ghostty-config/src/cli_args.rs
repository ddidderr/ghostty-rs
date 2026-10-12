//! Generic CLI / config-line argument parser (port of `cli/args.zig`).
//!
//! The Zig implementation uses comptime reflection over the destination
//! struct. In Rust this is done with two traits:
//!
//! * [`ConfigField`]: implemented by every type that can be a field value
//!   (bool, ints, floats, strings, enums, repeatable lists, ...). It knows
//!   how to parse itself from a `--key=value` value and how to format itself
//!   in the `+show-config` format.
//! * [`CliTarget`]: implemented by the destination struct (usually generated
//!   with the [`config_struct!`](crate::config_struct) macro). It maps a key
//!   to a field and optionally provides diagnostics, compatibility handlers,
//!   a manual-parse hook and a `--help` hook.

use std::{fmt, io::BufRead};

use crate::{
    c_value::CValue,
    comma_splitter::CommaSplitter,
    diagnostics::{Diagnostic, DiagnosticList, Location},
    formatter::EntryFormatter,
};

/// For trimming
pub const WHITESPACE: &str = " \t";

/// Trim [`WHITESPACE`] characters from both ends.
#[must_use]
pub fn trim_ws(s: &str) -> &str {
    s.trim_matches(|c| c == ' ' || c == '\t')
}

/// Trim Zig's `std.ascii.whitespace`, including vertical tab.
pub fn trim_ascii_whitespace(value: &[u8]) -> &[u8] {
    let is_whitespace = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c);
    let start = value
        .iter()
        .position(|b| !is_whitespace(b))
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|b| !is_whitespace(b))
        .map_or(start, |i| i + 1);
    &value[start..end]
}

/// Validate at a text field's parsing boundary, never in the line/argument reader.
pub fn text_value(value: Option<&[u8]>) -> Result<Option<&str>, ArgError> {
    value
        .map(|v| std::str::from_utf8(v).map_err(|_| ArgError::InvalidValue))
        .transpose()
}

pub fn trim_bytes(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\t'))
        .map_or(start, |i| i + 1);
    &value[start..end]
}

/// The base errors for arg parsing. Additional errors can be returned due
/// to type-specific parsing but these are always possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgError {
    ValueRequired,
    InvalidField,
    InvalidValue,
    /// A type-specific error. The string is the (Zig) error name and is
    /// shown to the user as `unknown error <name>`.
    Other(String),
}

impl ArgError {
    #[must_use]
    pub fn other(name: &str) -> Self {
        ArgError::Other(name.to_string())
    }
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArgError::ValueRequired => f.write_str("ValueRequired"),
            ArgError::InvalidField => f.write_str("InvalidField"),
            ArgError::InvalidValue => f.write_str("InvalidValue"),
            ArgError::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for ArgError {}

impl From<crate::comma_splitter::Error> for ArgError {
    fn from(e: crate::comma_splitter::Error) -> Self {
        ArgError::Other(e.to_string())
    }
}

/// Coarse classification of a field's Zig type, used by the build-data
/// generator (shell completions) which switches on `@typeInfo(field.type)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// `bool`
    Bool,
    /// `?bool`
    OptionalBool,
    /// An enum with the given variant names.
    Enum(&'static [&'static str]),
    /// `?Enum`
    OptionalEnum(&'static [&'static str]),
    /// An optional of any other type (`?T`).
    OptionalOther,
    /// A packed struct of bools without `parseCLI`, with its field names.
    Packed(&'static [&'static str]),
    /// `RepeatablePath`
    RepeatablePath,
    /// Anything else.
    Other,
}

/// A type that can be the value of a configuration / CLI field.
pub trait ConfigField: Clone + PartialEq + Default + fmt::Debug {
    /// For enum types, the list of valid values (used in error messages).
    const VALID_VALUES: Option<&'static [&'static str]> = None;

    /// Classification of this type for completion generation.
    const KIND: FieldKind = match Self::VALID_VALUES {
        Some(v) => FieldKind::Enum(v),
        None => FieldKind::Other,
    };

    /// Parse `value` (the text after `=`, `None` if there was no `=`) into
    /// `self`. Repeatable types accumulate; scalar types overwrite.
    fn parse_bytes(&mut self, value: Option<&[u8]>) -> Result<(), ArgError>;

    /// Convenience for plain UTF-8 input. Never accepts the OS escape encoding.
    fn parse_cli(&mut self, value: Option<&str>) -> Result<(), ArgError> {
        self.parse_bytes(value.map(str::as_bytes))
    }

    /// Called when the value was set to the empty string: reset to the
    /// field's default value (Zig: the `init` function or the declared
    /// default).
    fn reset(&mut self, default: Self) {
        *self = default;
    }

    /// Write this value in the `+show-config` format.
    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result;

    /// The value in the form the C API exposes (Zig: `c_get.getValue`).
    /// Types the C API doesn't support yet return [`CValue::Unsupported`].
    fn c_value(&self) -> CValue {
        CValue::Unsupported
    }

    /// [`ConfigField::c_value`] when the value is wrapped in an `Option`
    /// (only `String` differs: `?[:0]const u8` is supported, `[]const u8`
    /// is not).
    fn c_value_optional(&self) -> CValue {
        self.c_value()
    }

    /// The C value of an `Option<Self>` that is `None`.
    fn c_value_none() -> CValue {
        if matches!(Self::default().c_value(), CValue::Unsupported) {
            CValue::Unsupported
        } else {
            CValue::Null
        }
    }
}

/// The destination of CLI parsing.
pub trait CliTarget {
    /// Parse a single key/value pair into the destination
    /// (Zig: `parseIntoField`). Returns `ArgError::InvalidField` if the key
    /// is unknown.
    fn parse_into_field(&mut self, key: &str, value: Option<&[u8]>) -> Result<(), ArgError>;

    /// For enum-typed fields, the valid values (used for diagnostics).
    fn valid_values(&self, _key: &str) -> Option<&'static [&'static str]> {
        None
    }

    /// The diagnostic list, if supported (Zig: `_diagnostics` field).
    fn diagnostics_mut(&mut self) -> Option<&mut DiagnosticList> {
        None
    }

    /// Manual parse hook (Zig: `parseManuallyHook`). Return `Ok(false)` to
    /// stop parsing.
    fn parse_manually_hook(
        &mut self,
        _arg: &[u8],
        _iter: &mut dyn ArgIter,
    ) -> Result<bool, ArgError> {
        Ok(true)
    }

    /// Whether `--help`/`-h` is supported (Zig: `help` decl). If so,
    /// [`CliTarget::help`] is called when seen.
    fn supports_help(&self) -> bool {
        false
    }

    fn help(&mut self) -> Result<(), ArgError> {
        Ok(())
    }

    /// Compatibility handler (Zig: `compatibility` decl). Called when
    /// parsing a field fails. Return true if handled.
    fn compatibility(&mut self, _key: &str, _value: Option<&[u8]>) -> bool {
        false
    }
}

/// An iterator over arguments (Zig: any iterator with `next`).
pub trait ArgIter {
    fn next_arg(&mut self) -> Option<Vec<u8>>;

    /// Location for diagnostics.
    fn location(&self) -> Option<Location> {
        None
    }
}

/// Parse the command line arguments from iter into dst.
///
/// If the destination reports diagnostics (see
/// [`CliTarget::diagnostics_mut`]) then errors for individual fields are
/// added to the list rather than returned.
pub fn parse<T: CliTarget + ?Sized>(dst: &mut T, iter: &mut dyn ArgIter) -> Result<(), ArgError> {
    while let Some(arg) = iter.next_arg() {
        // Do manual parsing if we have a hook for it.
        if !dst.parse_manually_hook(&arg, iter)? {
            return Ok(());
        }

        // If the destination supports help then we check for it, call
        // the help function and return.
        if dst.supports_help() && (arg == b"--help" || arg == b"-h") {
            dst.help()?;
            return Ok(());
        }

        // If this doesn't start with "--" then it isn't a config
        // flag. We don't support positional arguments or configuration
        // values set with spaces so this is an error.
        let Some(rest) = arg.strip_prefix(b"--") else {
            let loc = iter.location().unwrap_or_default();
            let Some(diags) = dst.diagnostics_mut() else {
                return Err(ArgError::InvalidField);
            };
            diags.append(Diagnostic {
                key: ghostty_os::osbytes::bytes_to_config_string(&arg),
                message: "invalid field".to_string(),
                location: loc,
            });
            continue;
        };

        let (key, value) = match rest.iter().position(|b| *b == b'=') {
            Some(idx) => (&rest[..idx], Some(&rest[idx + 1..])),
            None => (rest, None),
        };

        // Keys are ASCII names. Keep arbitrary unknown keys byte-faithful in diagnostics.
        let key = ghostty_os::osbytes::bytes_to_config_string(key);
        let key = key.as_str();
        if let Err(err) = dst.parse_into_field(key, value) {
            // Fall back to compatibility handlers if able.
            if dst.compatibility(key, value) {
                log::info!(
                    "compatibility handler for {key} handled error, you may be using a deprecated field: {err}"
                );
                continue;
            }

            let loc = iter.location().unwrap_or_default();
            let message = match &err {
                ArgError::InvalidField => "unknown field".to_string(),
                ArgError::ValueRequired => {
                    let mut m = String::from("value required");
                    format_values(dst.valid_values(key), &mut m);
                    m
                }
                ArgError::InvalidValue => {
                    let mut m = match value {
                        Some(v) => format!(
                            "invalid value \"{}\"",
                            ghostty_os::osbytes::bytes_to_config_string(v)
                        ),
                        None => "invalid value \"null\"".to_string(),
                    };
                    format_values(dst.valid_values(key), &mut m);
                    m
                }
                ArgError::Other(name) => format!("unknown error error.{name}"),
            };

            let Some(diags) = dst.diagnostics_mut() else {
                return Err(err);
            };
            diags.append(Diagnostic {
                key: key.to_string(),
                message,
                location: loc,
            });
        }
    }

    Ok(())
}

fn format_values(values: Option<&[&str]>, out: &mut String) {
    if let Some(values) = values {
        out.push_str(", valid values are: ");
        out.push_str(&values.join(", "));
    }
}

/// Convenience for a compatibility handler that renames a field: parses
/// `value` into the field `to` of `dst`. Returns true if it succeeded.
pub fn compatibility_renamed<T: CliTarget + ?Sized>(
    dst: &mut T,
    to: &str,
    value: Option<&[u8]>,
) -> bool {
    match dst.parse_into_field(to, value) {
        Ok(()) => true,
        Err(err) => {
            log::warn!("error parsing renamed field {to}: {err}");
            false
        }
    }
}

/// Split a tagged-union value `tag:value` (Zig: `parseTaggedUnion`). The tag
/// is trimmed of whitespace. If there is no colon the value is empty.
#[must_use]
pub fn split_tagged_union(v: &str) -> (&str, &str) {
    match v.find(':') {
        Some(idx) => (trim_ws(&v[..idx]), &v[idx + 1..]),
        None => (trim_ws(v), ""),
    }
}

/// Split an ASCII union tag while preserving arbitrary payload bytes.
pub fn split_tagged_union_bytes(v: &[u8]) -> (&[u8], &[u8]) {
    match v.iter().position(|b| *b == b':') {
        Some(i) => (trim_bytes(&v[..i]), &v[i + 1..]),
        None => (trim_bytes(v), b""),
    }
}

/// Parse an "auto struct" value like `a:hello,b:42` (Zig: `parseAutoStruct`).
///
/// Entries are split by commas (with quoting / escaping, see
/// [`CommaSplitter`]). Each entry must be `key:value`; the value is trimmed
/// and, if double-quoted, decoded as a Zig string literal. For every entry
/// `set(key, value)` is called; it must return `Ok(true)` if the key matched
/// a field, `Ok(false)` if the key is unknown (which yields `InvalidValue`).
/// The caller is responsible for tracking which fields were set and for
/// applying defaults / required-field checks afterwards.
pub fn parse_auto_struct_with(
    v: &str,
    mut set: impl FnMut(&str, &str) -> Result<bool, ArgError>,
) -> Result<(), ArgError> {
    parse_auto_struct_bytes_with(v.as_bytes(), |k, v| {
        set(
            k,
            std::str::from_utf8(v).map_err(|_| ArgError::InvalidValue)?,
        )
    })
}

pub fn parse_auto_struct_bytes_with(
    v: &[u8],
    mut set: impl FnMut(&str, &[u8]) -> Result<bool, ArgError>,
) -> Result<(), ArgError> {
    let mut iter = CommaSplitter::from_bytes(v);
    while let Some(entry) = iter.next_bytes()? {
        let idx = entry
            .iter()
            .position(|b| *b == b':')
            .ok_or(ArgError::InvalidValue)?;
        let key = trim_bytes(&entry[..idx]);
        let raw = trim_bytes(&entry[idx + 1..]);

        let decoded;
        let value = if raw.len() >= 2 && raw.starts_with(b"\"") && raw.ends_with(b"\"") {
            decoded = parse_zig_string_literal_bytes(raw).ok_or(ArgError::InvalidValue)?;
            decoded.as_slice()
        } else {
            raw
        };

        if !set(
            std::str::from_utf8(key).map_err(|_| ArgError::InvalidValue)?,
            value,
        )? {
            return Err(ArgError::InvalidValue);
        }
    }
    Ok(())
}

/// Decode a double-quoted Zig string literal (including the quotes).
/// Returns `None` on failure or invalid UTF-8. Byte fields use
/// [`parse_zig_string_literal_bytes`] to retain arbitrary literal and escaped bytes.
#[must_use]
pub fn parse_zig_string_literal(s: &str) -> Option<String> {
    String::from_utf8(parse_zig_string_literal_bytes(s)?).ok()
}

/// Byte-exact [`parse_zig_string_literal`] (Zig `std.zig.string_literal`
/// decoding yields a `[]u8`).
#[must_use]
pub fn parse_zig_string_literal_bytes(s: impl AsRef<[u8]>) -> Option<Vec<u8>> {
    let b = s.as_ref();
    if b.len() < 2 || b[0] != b'"' || b[b.len() - 1] != b'"' {
        return None;
    }
    let inner = &b[1..b.len() - 1];
    let mut out: Vec<u8> = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        let c = inner[i];
        match c {
            b'\\' => {
                i += 1;
                let e = *inner.get(i)?;
                match e {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'\\' => out.push(b'\\'),
                    b'\'' => out.push(b'\''),
                    b'"' => out.push(b'"'),
                    b'x' => {
                        let h = inner.get(i + 1..i + 3)?;
                        let h = std::str::from_utf8(h).ok()?;
                        out.push(u8::from_str_radix(h, 16).ok()?);
                        i += 2;
                    }
                    b'u' => {
                        if *inner.get(i + 1)? != b'{' {
                            return None;
                        }
                        let close = inner[i + 2..].iter().position(|&x| x == b'}')? + i + 2;
                        let h = std::str::from_utf8(&inner[i + 2..close]).ok()?;
                        if h.is_empty() {
                            return None;
                        }
                        let cp = u32::from_str_radix(h, 16).ok()?;
                        let ch = char::from_u32(cp)?;
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        i = close;
                    }
                    _ => return None,
                }
            }
            b'"' | b'\n' => return None,
            _ => out.push(c),
        }
        i += 1;
    }
    Some(out)
}

/// Parse a "packed struct" value (comma separated names, optionally
/// prefixed with `no-`, or a standalone boolean). `names` are the field
/// names; `set(index, value)` is called for each setting.
pub fn parse_packed_struct_with(
    v: &str,
    names: &[&str],
    mut set: impl FnMut(usize, bool),
) -> Result<(), ArgError> {
    // Allow standalone boolean values like "true" and "false" to
    // turn on or off all of the struct's fields.
    if let Ok(b) = parse_bool(v) {
        for i in 0..names.len() {
            set(i, b);
        }
        return Ok(());
    }

    for part_raw in v.split(',') {
        let trimmed = trim_ws(part_raw);
        let (part, value) = match trimmed.strip_prefix("no-") {
            Some(rest) => (rest, false),
            None => (trimmed, true),
        };
        match names.iter().position(|n| *n == part) {
            Some(i) => set(i, value),
            None => return Err(ArgError::InvalidValue),
        }
    }
    Ok(())
}

pub fn parse_bool(v: &str) -> Result<bool, ArgError> {
    match v {
        "1" | "t" | "T" | "true" => Ok(true),
        "0" | "f" | "F" | "false" => Ok(false),
        _ => Err(ArgError::InvalidValue),
    }
}

/// Parse an integer like Zig's `std.fmt.parseInt(T, s, 0)`: optional sign,
/// optional `0x`/`0o`/`0b` prefix, underscores between digits allowed.
pub fn parse_int_base0(s: &str) -> Option<i128> {
    let (neg, rest) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    let (base, digits) = if rest.len() >= 2 && rest.as_bytes()[0] == b'0' {
        match rest.as_bytes()[1] {
            b'x' => (16, &rest[2..]),
            b'o' => (8, &rest[2..]),
            b'b' => (2, &rest[2..]),
            _ => (10, rest),
        }
    } else {
        (10, rest)
    };
    if digits.is_empty() {
        return None;
    }
    let db = digits.as_bytes();
    if db[0] == b'_' || db[db.len() - 1] == b'_' || digits.contains("__") {
        return None;
    }
    let mut acc: i128 = 0;
    for &c in db {
        if c == b'_' {
            continue;
        }
        let d = i128::from((c as char).to_digit(base)?);
        acc = acc.checked_mul(i128::from(base))?.checked_add(d)?;
    }
    Some(if neg { -acc } else { acc })
}

// ---------------------------------------------------------------------
// ConfigField impls for primitives

impl ConfigField for bool {
    const KIND: FieldKind = FieldKind::Bool;

    fn c_value(&self) -> CValue {
        CValue::Bool(*self)
    }

    fn parse_bytes(&mut self, value: Option<&[u8]>) -> Result<(), ArgError> {
        let value = crate::cli_args::text_value(value)?;
        *self = parse_bool(value.unwrap_or("t"))?;
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        writeln!(f.writer, "{} = {}", f.name, self)
    }
}

macro_rules! impl_int_field {
    ($($t:ty => $cv:expr),*) => {$(
        impl ConfigField for $t {
            fn c_value(&self) -> CValue {
                let f: fn($t) -> CValue = $cv;
                f(*self)
            }

            fn parse_bytes(&mut self, value: Option<&[u8]>) -> Result<(), ArgError> {
        let value = crate::cli_args::text_value(value)?;
                let v = value.ok_or(ArgError::ValueRequired)?;
                let n = parse_int_base0(v).ok_or(ArgError::InvalidValue)?;
                *self = <$t>::try_from(n).map_err(|_| ArgError::InvalidValue)?;
                Ok(())
            }

            fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
                writeln!(f.writer, "{} = {}", f.name, self)
            }
        }
    )*};
}
impl_int_field!(
    u8 => |v| CValue::Uint(u32::from(v)),
    u16 => |_| CValue::Unsupported,
    u32 => CValue::Uint,
    u64 => |_| CValue::Unsupported,
    usize => |_| CValue::Unsupported,
    i8 => |_| CValue::Unsupported,
    i16 => CValue::Short,
    i32 => |_| CValue::Unsupported,
    i64 => |_| CValue::Unsupported,
    isize => |_| CValue::Unsupported
);

macro_rules! impl_float_field {
    ($($t:ty => $cv:expr),*) => {$(
        impl ConfigField for $t {
            fn c_value(&self) -> CValue {
                let f: fn($t) -> CValue = $cv;
                f(*self)
            }

            fn parse_bytes(&mut self, value: Option<&[u8]>) -> Result<(), ArgError> {
        let value = crate::cli_args::text_value(value)?;
                let v = value.ok_or(ArgError::ValueRequired)?;
                *self = v.parse::<$t>().map_err(|_| ArgError::InvalidValue)?;
                Ok(())
            }

            fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
                writeln!(f.writer, "{} = {}", f.name, self)
            }
        }
    )*};
}
impl_float_field!(f32 => CValue::F32, f64 => CValue::F64);

impl ConfigField for String {
    fn c_value_optional(&self) -> CValue {
        CValue::String(ghostty_os::osbytes::valid_str_to_config_string(self))
    }

    fn c_value_none() -> CValue {
        CValue::NullString
    }

    fn parse_bytes(&mut self, value: Option<&[u8]>) -> Result<(), ArgError> {
        let value = crate::cli_args::text_value(value)?;
        *self = value.ok_or(ArgError::ValueRequired)?.to_string();
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        writeln!(
            f.writer,
            "{} = {}",
            f.name,
            ghostty_os::osbytes::valid_str_to_config_string(self)
        )
    }
}
impl ConfigField for ghostty_os::osbytes::EncodedOs {
    fn c_value_optional(&self) -> CValue {
        CValue::String(self.as_encoded().to_owned())
    }

    fn c_value_none() -> CValue {
        CValue::NullString
    }

    fn parse_bytes(&mut self, value: Option<&[u8]>) -> Result<(), ArgError> {
        *self = Self::from_bytes(value.ok_or(ArgError::ValueRequired)?);
        Ok(())
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        writeln!(f.writer, "{} = {}", f.name, self.as_encoded())
    }
}

impl<T: ConfigField> ConfigField for Option<T> {
    const KIND: FieldKind = match T::KIND {
        FieldKind::Bool => FieldKind::OptionalBool,
        FieldKind::Enum(v) => FieldKind::OptionalEnum(v),
        _ => FieldKind::OptionalOther,
    };

    fn c_value(&self) -> CValue {
        match self {
            Some(v) => v.c_value_optional(),
            None => T::c_value_none(),
        }
    }

    fn parse_bytes(&mut self, value: Option<&[u8]>) -> Result<(), ArgError> {
        // For optional fields, we just treat it as the child type.
        if let Some(v) = self {
            v.parse_bytes(value)
        } else {
            let mut tmp = T::default();
            tmp.parse_bytes(value)?;
            *self = Some(tmp);
            Ok(())
        }
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        match self {
            Some(v) => v.format_entry(f),
            None => writeln!(f.writer, "{} = ", f.name),
        }
    }
}

// ---------------------------------------------------------------------
// Iterators

/// An iterator that considers its location to be CLI args. It
/// iterates through an underlying iterator and increments a counter
/// to track the current CLI arg index.
///
/// This also ignores any argument that starts with `+`. It assumes that
/// actions were parsed out before this iterator was created.
pub struct ArgsIterator<I: Iterator> {
    /// The underlying args iterator.
    pub iterator: I,
    /// Our current index into the iterator. This is 1-indexed because
    /// the 0 value is used to indicate that we haven't read any values yet.
    pub index: usize,
}

impl<I: Iterator> ArgsIterator<I>
where
    I::Item: AsRef<[u8]>,
{
    pub fn new(iterator: I) -> Self {
        Self { iterator, index: 0 }
    }

    pub fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            let value = self.iterator.next()?;
            self.index += 1;
            // We ignore any argument that starts with "+". This is used
            // to indicate actions and are expected to be parsed out before
            // this iterator is created.
            if value.as_ref().starts_with(b"+") {
                continue;
            }
            return Some(value.as_ref().to_vec());
        }
    }
}

impl<I: Iterator> ArgIter for ArgsIterator<I>
where
    I::Item: AsRef<[u8]>,
{
    fn next_arg(&mut self) -> Option<Vec<u8>> {
        self.next()
    }

    fn location(&self) -> Option<Location> {
        Some(Location::Cli(self.index))
    }
}

/// Create an args iterator for the process args. This will skip argv0.
#[must_use]
pub fn args_iterator() -> ArgsIterator<std::iter::Skip<std::vec::IntoIter<Vec<u8>>>> {
    // Keep original bytes until each field chooses whether to validate UTF-8.
    ArgsIterator::new(ghostty_os::args::collect_bytes().into_iter().skip(1))
}

/// An iterator valid for arg parsing from a slice.
pub struct SliceIterator<'a, T> {
    pub slice: &'a [T],
    pub idx: usize,
}

impl<T: AsRef<[u8]>> ArgIter for SliceIterator<'_, T> {
    fn next_arg(&mut self) -> Option<Vec<u8>> {
        let v = self.slice.get(self.idx)?;
        self.idx += 1;
        Some(v.as_ref().to_vec())
    }
}

/// Construct a [`SliceIterator`] from a slice.
#[must_use]
pub fn slice_iterator<T: AsRef<[u8]>>(slice: &[T]) -> SliceIterator<'_, T> {
    SliceIterator { slice, idx: 0 }
}

/// A simple whitespace-splitting iterator used in tests and for
/// string-based argument parsing.
pub struct VecIterator {
    items: std::vec::IntoIter<Vec<u8>>,
}

impl VecIterator {
    #[must_use]
    pub fn new<T: AsRef<[u8]>>(items: Vec<T>) -> Self {
        Self {
            items: items
                .into_iter()
                .map(|v| v.as_ref().to_vec())
                .collect::<Vec<_>>()
                .into_iter(),
        }
    }

    /// Split on spaces (no quoting except double quotes around a value).
    #[must_use]
    pub fn from_str_args(s: &str) -> Self {
        Self::new(split_args(s))
    }
}

impl ArgIter for VecIterator {
    fn next_arg(&mut self) -> Option<Vec<u8>> {
        self.items.next()
    }
}

/// Split a string into args like `std.process.ArgIteratorGeneral`: whitespace
/// separated, double quotes group (and are removed).
#[must_use]
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut in_arg = false;
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                in_arg = true;
            }
            c if c.is_whitespace() && !quoted => {
                if in_arg {
                    out.push(std::mem::take(&mut cur));
                    in_arg = false;
                }
            }
            c => {
                cur.push(c);
                in_arg = true;
            }
        }
    }
    if in_arg {
        out.push(cur);
    }
    out
}

/// Reads CLI args by line. Each CLI arg is expected to be a single line.
/// This is used to implement configuration files.
pub struct LineIterator<R: BufRead> {
    /// Our stateful reader.
    pub r: R,
    /// Filepath that is used for diagnostics. It is only used for
    /// diagnostic messages so it can be formatted however you want.
    pub filepath: String,
    /// The current line that we're on. 1-indexed; zero if we haven't read
    /// any lines yet.
    pub line: usize,
}

impl<R: BufRead> LineIterator<R> {
    /// The maximum size a single line can be. We don't expect any
    /// CLI arg to exceed this size.
    pub const MAX_LINE_SIZE: usize = 4096;

    pub fn new(r: R) -> Self {
        Self {
            r,
            filepath: String::new(),
            line: 0,
        }
    }

    pub fn with_path(r: R, filepath: impl Into<String>) -> Self {
        Self {
            r,
            filepath: filepath.into(),
            line: 0,
        }
    }

    pub fn next(&mut self) -> Option<Vec<u8>> {
        let mut buf = Vec::new();
        let entry: Vec<u8> = loop {
            buf.clear();
            match self.r.read_until(b'\n', &mut buf) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) => {
                    log::warn!("cannot read from \"{}\": {e}", self.filepath);
                    return None;
                }
            }
            if buf.last() == Some(&b'\n') {
                buf.pop();
            }
            if buf.len() > Self::MAX_LINE_SIZE - 2 {
                log::warn!("cannot read from \"{}\": StreamTooLong", self.filepath);
                return None;
            }

            self.line += 1;

            // Zig trims spaces, tabs and carriage returns, without decoding text.
            let start = buf
                .iter()
                .position(|b| !matches!(b, b' ' | b'\t' | b'\r'))
                .unwrap_or(buf.len());
            let end = buf
                .iter()
                .rposition(|b| !matches!(b, b' ' | b'\t' | b'\r'))
                .map_or(start, |i| i + 1);
            let trim = &buf[start..end];

            // Ignore blank lines and comments
            if trim.is_empty() || trim.starts_with(b"#") {
                continue;
            }
            break trim.to_vec();
        };

        let mut result = b"--".to_vec();
        if let Some(idx) = entry.iter().position(|b| *b == b'=') {
            let key = trim_bytes(&entry[..idx]);
            let mut value = trim_bytes(&entry[idx + 1..]);

            // Detect a quoted string.
            if value.len() >= 2 && value.starts_with(b"\"") && value.ends_with(b"\"") {
                // Trim quotes since our CLI args processor expects
                // quotes to already be gone.
                value = &value[1..value.len() - 1];
            }

            result.extend_from_slice(key);
            result.push(b'=');
            result.extend_from_slice(value);
        } else {
            result.extend_from_slice(&entry);
        }
        Some(result)
    }
}

impl<R: BufRead> ArgIter for LineIterator<R> {
    fn next_arg(&mut self) -> Option<Vec<u8>> {
        self.next()
    }

    fn location(&self) -> Option<Location> {
        // If we have no filepath then we have no location.
        if self.filepath.is_empty() {
            return None;
        }
        Some(Location::File {
            path: self.filepath.clone(),
            line: self.line,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::{config_enum, config_packed, config_struct};

    fn parse_str<T: CliTarget>(dst: &mut T, s: &str) -> Result<(), ArgError> {
        let mut it = VecIterator::from_str_args(s);
        parse(dst, &mut it)
    }

    // ---- test structs -------------------------------------------------

    config_struct! {
        pub struct Simple {
            "a" => a: String = String::new(),
            "b" => b: bool = false,
            "b-f" => b_f: bool = true,
        }
        extra {
            diags: DiagnosticList = DiagnosticList::default(),
        }
    }
    impl Default for Simple {
        fn default() -> Self {
            Self::raw_default()
        }
    }
    impl CliTarget for Simple {
        fn parse_into_field(&mut self, key: &str, value: Option<&[u8]>) -> Result<(), ArgError> {
            self.parse_field_bytes(key, value)
        }
    }

    config_struct! {
        pub struct Quoted {
            "a" => a: u8 = 0,
            "b" => b: String = String::new(),
        }
    }
    impl CliTarget for Quoted {
        fn parse_into_field(&mut self, key: &str, value: Option<&[u8]>) -> Result<(), ArgError> {
            self.parse_field_bytes(key, value)
        }
    }

    config_struct! {
        pub struct Reset {
            "a" => a: u8 = 42,
            "b" => b: bool = false,
        }
    }
    impl CliTarget for Reset {
        fn parse_into_field(&mut self, key: &str, value: Option<&[u8]>) -> Result<(), ArgError> {
            self.parse_field_bytes(key, value)
        }
    }

    config_enum! {
        pub enum OneTwo { One => "one", Two => "two" }
    }

    config_struct! {
        pub struct Diag {
            "a" => a: String = String::new(),
            "b" => b: OneTwo = OneTwo::One,
        }
        extra {
            diags: DiagnosticList = DiagnosticList::default(),
        }
    }
    impl CliTarget for Diag {
        fn parse_into_field(&mut self, key: &str, value: Option<&[u8]>) -> Result<(), ArgError> {
            self.parse_field_bytes(key, value)
        }
        fn valid_values(&self, key: &str) -> Option<&'static [&'static str]> {
            Self::valid_values_for(key)
        }
        fn diagnostics_mut(&mut self) -> Option<&mut DiagnosticList> {
            Some(&mut self.diags)
        }
    }

    #[test]
    fn parse_simple() {
        let mut data = Simple::raw_default();
        parse_str(&mut data, "--a=42 --b --b-f=false").unwrap();
        assert_eq!(data.a, "42");
        assert!(data.b);
        assert!(!data.b_f);

        // Reparsing works
        parse_str(&mut data, "--a=84").unwrap();
        assert_eq!(data.a, "84");
        assert!(data.b);
        assert!(!data.b_f);
    }

    #[test]
    fn parse_quoted_value() {
        let mut data = Quoted::raw_default();
        parse_str(&mut data, "--a=\"42\" --b=\"hello!\"").unwrap();
        assert_eq!(data.a, 42);
        assert_eq!(data.b, "hello!");
    }

    #[test]
    fn parse_empty_value_resets_to_default() {
        let mut data = Reset::raw_default();
        parse_str(&mut data, "--a= --b=").unwrap();
        assert_eq!(data.a, 42);
        assert!(!data.b);
    }

    #[test]
    fn parse_positional_arguments_are_invalid() {
        let mut data = Reset::raw_default();
        data.a = 0;
        let r = parse_str(&mut data, "--a=84 what");
        assert_eq!(r, Err(ArgError::InvalidField));
        assert_eq!(data.a, 84);
    }

    #[test]
    fn parse_diagnostic_tracking() {
        let mut data = Diag::raw_default();
        parse_str(&mut data, "--what --a=42").unwrap();
        assert_eq!(data.a, "42");
        assert_eq!(data.diags.items().len(), 1);
        let diag = &data.diags.items()[0];
        assert_eq!(diag.location, Location::None);
        assert_eq!(diag.key, "what");
        assert_eq!(diag.message, "unknown field");
    }

    #[test]
    fn parse_diagnostic_location() {
        let mut data = Diag::raw_default();
        let mut iter = LineIterator::with_path(Cursor::new("a=42\nwhat\nb=two"), "test");
        parse(&mut data, &mut iter).unwrap();
        assert_eq!(data.a, "42");
        assert_eq!(data.b, OneTwo::Two);
        assert_eq!(data.diags.items().len(), 1);
        let diag = &data.diags.items()[0];
        assert_eq!(diag.key, "what");
        assert_eq!(diag.message, "unknown field");
        assert_eq!(
            diag.location,
            Location::File {
                path: "test".into(),
                line: 2
            }
        );
    }

    #[test]
    fn parse_diagnostic_invalid_enum_value_lists_valid_values() {
        let mut data = Diag::raw_default();
        parse_str(&mut data, "--b=three").unwrap();
        assert_eq!(
            data.diags.items()[0].message,
            "invalid value \"three\", valid values are: one, two"
        );
    }

    pub struct Compat {
        pub a: bool,
    }
    impl CliTarget for Compat {
        fn parse_into_field(&mut self, key: &str, value: Option<&[u8]>) -> Result<(), ArgError> {
            if key == "a" {
                // Fails for anything but a valid bool.
                return self.a.parse_bytes(value);
            }
            Err(ArgError::InvalidField)
        }
        fn compatibility(&mut self, key: &str, value: Option<&[u8]>) -> bool {
            if key == "a" && value == Some(b"yuh") {
                self.a = true;
                return true;
            }
            false
        }
    }

    #[test]
    fn parse_compatibility_handler() {
        let mut data = Compat { a: false };
        parse_str(&mut data, "--a=yuh").unwrap();
        assert!(data.a);
    }

    pub struct Renamed {
        pub a: bool,
        pub b: bool,
    }
    impl CliTarget for Renamed {
        fn parse_into_field(&mut self, key: &str, value: Option<&[u8]>) -> Result<(), ArgError> {
            match key {
                "a" => self.a.parse_bytes(value),
                "b" => self.b.parse_bytes(value),
                _ => Err(ArgError::InvalidField),
            }
        }
        fn compatibility(&mut self, key: &str, value: Option<&[u8]>) -> bool {
            if key == "old" {
                return compatibility_renamed(self, "a", value);
            }
            false
        }
    }

    #[test]
    fn parse_compatibility_renamed() {
        let mut data = Renamed { a: false, b: false };
        parse_str(&mut data, "--old=true --b=true").unwrap();
        assert!(data.a);
        assert!(data.b);
    }

    config_struct! {
        pub struct Underscore {
            "a" => a: String = "12".to_string(),
        }
    }
    impl CliTarget for Underscore {
        fn parse_into_field(&mut self, key: &str, value: Option<&[u8]>) -> Result<(), ArgError> {
            // Underscore-prefixed fields are never parseable.
            if key.starts_with('_') {
                return Err(ArgError::InvalidField);
            }
            self.parse_field_bytes(key, value)
        }
    }

    #[test]
    fn parse_into_field_ignore_underscore_prefixed_fields() {
        let mut data = Underscore::raw_default();
        assert_eq!(
            data.parse_into_field("_a", Some(b"42")),
            Err(ArgError::InvalidField)
        );
        assert_eq!(data.a, "12");
    }

    #[derive(Clone, PartialEq, Debug)]
    pub struct WithInit {
        pub v: String,
    }
    impl Default for WithInit {
        fn default() -> Self {
            Self {
                v: "default".into(),
            }
        }
    }
    impl ConfigField for WithInit {
        fn parse_bytes(&mut self, _value: Option<&[u8]>) -> Result<(), ArgError> {
            let _value = crate::cli_args::text_value(_value)?;
            Ok(())
        }
        fn reset(&mut self, _default: Self) {
            // `init`
            self.v = "HELLO!".into();
        }
        fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
            writeln!(f.writer, "{} = {}", f.name, self.v)
        }
    }

    config_struct! {
        pub struct HasInit {
            "a" => a: WithInit = WithInit::default(),
        }
    }

    #[test]
    fn parse_into_field_struct_with_init_func() {
        let mut data = HasInit::raw_default();
        data.parse_field("a", Some("")).unwrap();
        assert_eq!(data.a.v, "HELLO!");
    }

    macro_rules! one_field {
        ($name:ident, $ty:ty, $def:expr) => {
            config_struct! {
                pub struct $name {
                    "v" => v: $ty = $def,
                }
            }
        };
    }

    #[test]
    fn parse_into_field_string() {
        one_field!(S, String, String::new());
        let mut d = S::raw_default();
        d.parse_field("v", Some("42")).unwrap();
        assert_eq!(d.v, "42");
    }

    #[test]
    fn parse_into_field_bool() {
        one_field!(S, bool, false);
        let mut d = S::raw_default();
        for t in ["1", "t", "T", "true"] {
            d.parse_field("v", Some(t)).unwrap();
            assert!(d.v);
            d.v = false;
        }
        for f in ["0", "f", "F", "false"] {
            d.v = true;
            d.parse_field("v", Some(f)).unwrap();
            assert!(!d.v);
        }
    }

    #[test]
    fn parse_into_field_unsigned_numbers() {
        one_field!(S, u8, 0);
        let mut d = S::raw_default();
        d.parse_field("v", Some("1")).unwrap();
        assert_eq!(d.v, 1);
        d.parse_field("v", Some("0x10")).unwrap();
        assert_eq!(d.v, 16);
        assert_eq!(d.parse_field("v", Some("256")), Err(ArgError::InvalidValue));
        assert_eq!(d.parse_field("v", Some("-1")), Err(ArgError::InvalidValue));
    }

    #[test]
    fn parse_into_field_floats() {
        one_field!(S, f64, 0.0);
        let mut d = S::raw_default();
        d.parse_field("v", Some("1")).unwrap();
        assert!((d.v - 1.0).abs() < f64::EPSILON);
    }

    config_enum! {
        pub enum Three { One => "one", Two => "two", Three => "three" }
    }

    #[test]
    fn parse_into_field_enums() {
        one_field!(S, Three, Three::One);
        let mut d = S::raw_default();
        d.parse_field("v", Some("two")).unwrap();
        assert_eq!(d.v, Three::Two);
    }

    config_packed! {
        pub struct Pk {
            "a" => a: false,
            "b" => b: true,
        }
    }

    #[test]
    fn parse_into_field_packed_struct() {
        one_field!(S, Pk, Pk::default());
        let mut d = S::raw_default();
        d.parse_field("v", Some("b")).unwrap();
        assert!(!d.v.a);
        assert!(d.v.b);
    }

    #[test]
    fn parse_into_field_packed_struct_negation() {
        one_field!(S, Pk, Pk::default());
        let mut d = S::raw_default();
        d.parse_field("v", Some("a,no-b")).unwrap();
        assert!(d.v.a);
        assert!(!d.v.b);
    }

    #[test]
    fn parse_into_field_packed_struct_true_false() {
        one_field!(S, Pk, Pk::default());
        let mut d = S::raw_default();
        d.parse_field("v", Some("true")).unwrap();
        assert!(d.v.a && d.v.b);
        d.parse_field("v", Some("false")).unwrap();
        assert!(!d.v.a && !d.v.b);
        assert_eq!(
            d.parse_field("v", Some("true,a")),
            Err(ArgError::InvalidValue)
        );
    }

    #[test]
    fn parse_into_field_packed_struct_whitespace() {
        one_field!(S, Pk, Pk::default());
        let mut d = S::raw_default();
        d.parse_field("v", Some(" a, no-b ")).unwrap();
        assert!(d.v.a);
        assert!(!d.v.b);
    }

    #[test]
    fn parse_into_field_optional_field() {
        one_field!(S, Option<bool>, None);
        let mut d = S::raw_default();
        d.parse_field("v", Some("1")).unwrap();
        assert_eq!(d.v, Some(true));
        // Unset
        d.parse_field("v", Some("")).unwrap();
        assert_eq!(d.v, None);
    }

    #[derive(Clone, PartialEq, Debug, Default)]
    pub struct WithParse {
        pub v: String,
    }
    impl ConfigField for WithParse {
        fn parse_bytes(&mut self, _value: Option<&[u8]>) -> Result<(), ArgError> {
            let _value = crate::cli_args::text_value(_value)?;
            self.v = "HELLO!".into();
            Ok(())
        }
        fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
            writeln!(f.writer, "{} = {}", f.name, self.v)
        }
    }

    #[test]
    fn parse_into_field_struct_with_parse_func() {
        one_field!(S, WithParse, WithParse::default());
        let mut d = S::raw_default();
        d.parse_field("v", Some("42")).unwrap();
        assert_eq!(d.v.v, "HELLO!");
    }

    #[test]
    fn parse_into_field_optional_struct_with_parse_func() {
        one_field!(S, Option<WithParse>, None);
        let mut d = S::raw_default();
        d.parse_field("v", Some("42")).unwrap();
        assert_eq!(d.v.unwrap().v, "HELLO!");
    }

    #[derive(Clone, PartialEq, Debug)]
    pub struct Basic {
        pub a: String,
        pub b: u32,
        pub c: u8,
    }
    impl Default for Basic {
        fn default() -> Self {
            Self {
                a: String::new(),
                b: 0,
                c: 12,
            }
        }
    }
    impl ConfigField for Basic {
        fn parse_bytes(&mut self, value: Option<&[u8]>) -> Result<(), ArgError> {
            let value = crate::cli_args::text_value(value)?;
            let v = value.ok_or(ArgError::ValueRequired)?;
            let mut r = Basic::default();
            let mut set = [false; 3];
            parse_auto_struct_with(v, |k, val| {
                match k {
                    "a" => {
                        r.a.parse_cli(Some(val))?;
                        set[0] = true;
                    }
                    "b" => {
                        r.b.parse_cli(Some(val))?;
                        set[1] = true;
                    }
                    "c" => {
                        r.c.parse_cli(Some(val))?;
                        set[2] = true;
                    }
                    _ => return Ok(false),
                }
                Ok(true)
            })?;
            // a and b are required
            if !set[0] || !set[1] {
                return Err(ArgError::InvalidValue);
            }
            *self = r;
            Ok(())
        }
        fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
            writeln!(
                f.writer,
                "{} = a:{},b:{},c:{}",
                f.name, self.a, self.b, self.c
            )
        }
    }

    #[test]
    fn parse_into_field_struct_with_basic_fields() {
        one_field!(S, Basic, Basic::default());
        let mut d = S::raw_default();
        d.parse_field("v", Some("a:hello,b:42")).unwrap();
        assert_eq!(d.v.a, "hello");
        assert_eq!(d.v.b, 42);
        assert_eq!(d.v.c, 12);

        d.parse_field("v", Some("a:world,b:84,c:24")).unwrap();
        assert_eq!(d.v.a, "world");
        assert_eq!(d.v.b, 84);
        assert_eq!(d.v.c, 24);

        assert_eq!(
            d.parse_field("v", Some("a:hello")),
            Err(ArgError::InvalidValue)
        );
    }

    #[derive(Clone, PartialEq, Debug, Default)]
    pub enum Tu {
        #[default]
        None,
        A(u8),
        B(u8),
        C,
        D(String),
    }
    impl ConfigField for Tu {
        fn parse_bytes(&mut self, value: Option<&[u8]>) -> Result<(), ArgError> {
            let value = crate::cli_args::text_value(value)?;
            let v = value.ok_or(ArgError::ValueRequired)?;
            let (tag, rest) = split_tagged_union(v);
            *self = match tag {
                "a" => {
                    let mut x = 0u8;
                    x.parse_cli(Some(rest))?;
                    Tu::A(x)
                }
                "b" => {
                    let mut x = 0u8;
                    x.parse_cli(Some(rest))?;
                    Tu::B(x)
                }
                "c" => {
                    if !rest.is_empty() {
                        return Err(ArgError::InvalidValue);
                    }
                    Tu::C
                }
                "d" => Tu::D(rest.to_string()),
                _ => return Err(ArgError::InvalidValue),
            };
            Ok(())
        }
        fn format_entry(&self, _f: &mut EntryFormatter<'_>) -> fmt::Result {
            Ok(())
        }
    }

    #[test]
    fn parse_into_field_tagged_union() {
        one_field!(S, Tu, Tu::None);
        let mut d = S::raw_default();
        d.parse_field("v", Some("a:1")).unwrap();
        assert_eq!(d.v, Tu::A(1));
        d.parse_field("v", Some("b:2")).unwrap();
        assert_eq!(d.v, Tu::B(2));
        d.parse_field("v", Some("c")).unwrap();
        assert_eq!(d.v, Tu::C);
        d.parse_field("v", Some("d:hello")).unwrap();
        assert_eq!(d.v, Tu::D("hello".into()));
    }

    #[test]
    fn parse_into_field_tagged_union_unknown_field() {
        one_field!(S, Tu, Tu::None);
        let mut d = S::raw_default();
        assert_eq!(d.parse_field("v", Some("x:1")), Err(ArgError::InvalidValue));
    }

    #[test]
    fn parse_into_field_tagged_union_invalid_field_value() {
        one_field!(S, Tu, Tu::None);
        let mut d = S::raw_default();
        assert_eq!(
            d.parse_field("v", Some("a:hello")),
            Err(ArgError::InvalidValue)
        );
    }

    #[test]
    fn parse_into_field_tagged_union_missing_tag() {
        one_field!(S, Tu, Tu::None);
        let mut d = S::raw_default();
        assert_eq!(d.parse_field("v", Some("a")), Err(ArgError::InvalidValue));
        assert_eq!(d.parse_field("v", Some(":a")), Err(ArgError::InvalidValue));
    }

    #[test]
    fn args_iterator() {
        let child = split_args("--what +list-things --a=42").into_iter();
        let mut iter = ArgsIterator::new(child);
        assert_eq!(iter.next().unwrap(), b"--what");
        assert_eq!(iter.next().unwrap(), b"--a=42");
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next(), None);
    }

    fn line_iter(s: &str) -> LineIterator<Cursor<Vec<u8>>> {
        LineIterator::new(Cursor::new(s.as_bytes().to_vec()))
    }

    #[test]
    fn line_iterator() {
        let mut iter = line_iter(
            "A\nB=42\nC\n\n# A comment\nD\n\n  # An indented comment\n  E\n\n# A quoted string with whitespace\nF=  \"value \"",
        );
        assert_eq!(iter.next().unwrap(), b"--A");
        assert_eq!(iter.next().unwrap(), b"--B=42");
        assert_eq!(iter.next().unwrap(), b"--C");
        assert_eq!(iter.next().unwrap(), b"--D");
        assert_eq!(iter.next().unwrap(), b"--E");
        assert_eq!(iter.next().unwrap(), b"--F=value ");
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn line_iterator_end_in_newline() {
        let mut iter = line_iter("A\n\n");
        assert_eq!(iter.next().unwrap(), b"--A");
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn line_iterator_spaces_around_equals() {
        let mut iter = line_iter("A = B\n\n");
        assert_eq!(iter.next().unwrap(), b"--A=B");
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn line_iterator_no_value() {
        let mut iter = line_iter("A = \n\n");
        assert_eq!(iter.next().unwrap(), b"--A=");
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn line_iterator_with_crlf_line_endings() {
        let mut iter = line_iter("A\r\nB = C\r\n");
        assert_eq!(iter.next().unwrap(), b"--A");
        assert_eq!(iter.next().unwrap(), b"--B=C");
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn line_iterator_with_buffered_reader() {
        let r = std::io::BufReader::with_capacity(2, Cursor::new(b"A\nB = C\n".to_vec()));
        let mut iter = LineIterator::new(r);
        assert_eq!(iter.next().unwrap(), b"--A");
        assert_eq!(iter.next().unwrap(), b"--B=C");
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn line_iterator_with_buffered_and_primed_reader() {
        use std::io::BufRead;
        let mut r = std::io::BufReader::with_capacity(2, Cursor::new(b"A\nB = C\n".to_vec()));
        r.fill_buf().unwrap();
        let mut iter = LineIterator::new(r);
        assert_eq!(iter.next().unwrap(), b"--A");
        assert_eq!(iter.next().unwrap(), b"--B=C");
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn parse_int_base0_forms() {
        assert_eq!(parse_int_base0("0x_1"), None);
        assert_eq!(parse_int_base0("1_000"), Some(1000));
        assert_eq!(parse_int_base0("-0b11"), Some(-3));
        assert_eq!(parse_int_base0("0o17"), Some(15));
        assert_eq!(parse_int_base0(""), None);
    }

    #[test]
    fn zig_string_literal_decode() {
        assert_eq!(
            parse_zig_string_literal("\"a\\n\\x41\\u{1F47B}\"").unwrap(),
            "a\nA\u{1F47B}"
        );
        assert_eq!(parse_zig_string_literal("\"\\q\""), None);
    }

    #[test]
    fn zig_string_literal_keeps_raw_bytes() {
        // `\xFF` is not valid UTF-8 and must stay the single byte 0xFF.
        assert_eq!(
            parse_zig_string_literal_bytes("\"a\\xFFb\"").unwrap(),
            vec![b'a', 0xff, b'b']
        );
        assert!(parse_zig_string_literal("\"a\\xFFb\"").is_none());
        // Valid multi-byte sequences spelled with escapes still decode.
        assert_eq!(
            parse_zig_string_literal("\"\\xC3\\xA9\"").unwrap(),
            "\u{e9}"
        );
        // Raw (unescaped) invalid input bytes survive literal parsing.
        let raw = b"\"x\xfey\"";
        assert_eq!(
            parse_zig_string_literal_bytes(raw).unwrap(),
            vec![b'x', 0xfe, b'y']
        );
    }

    #[test]
    fn line_iterator_keeps_invalid_utf8_bytes() {
        let data: &[u8] = b"title = a\xffb\n";
        let mut it = LineIterator::new(data);
        let line = it.next().unwrap();
        assert_eq!(line, b"--title=a\xffb".to_vec());
    }
}
