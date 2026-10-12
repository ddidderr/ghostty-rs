//! Terminfo source format. This can be used to encode terminfo files.
//! This cannot parse terminfo source files yet because it isn't something
//! I need to do but this can be added later.
//!
//! Background: <https://invisible-island.net/ncurses/man/terminfo.5.html>
//!
//! Port of `terminfo/Source.zig`.

use std::{collections::HashMap, io};

/// A terminfo source description.
#[derive(Debug, Clone, Copy)]
pub struct Source {
    /// The set of names for the terminal. These match the TERM environment
    /// variable and are used to look up this terminal. Historically, the final
    /// name in the list was the most common name for the terminal and contains
    /// spaces and other characters. See terminfo(5) for details.
    pub names: &'static [&'static str],

    /// The set of capabilities in this terminfo file.
    pub capabilities: &'static [Capability],
}

/// A capability in a terminfo file. This also includes any "use" capabilities
/// since they behave just like other capabilities as documented in terminfo(5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    /// The name of capability. This is the "Cap-name" value in terminfo(5).
    pub name: &'static str,
    pub value: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    /// Canceled value, i.e. suffixed with @
    Canceled,

    /// Boolean values are always true if they exist so there is no value.
    Boolean,

    /// Numeric values are always "unsigned decimal integers". The size
    /// of the integer is unspecified in terminfo(5). 32-bits is a common
    /// integer size but this may be wrong.
    Numeric(u32),

    String(&'static str),
}

impl Source {
    /// Encode as a terminfo source file. The encoding is always done in a
    /// human-readable format with whitespace. Fields are always written in the
    /// order of the slices on this struct; this will not do any reordering.
    pub fn encode<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        // Encode the names in the order specified
        for (i, name) in self.names.iter().enumerate() {
            if i != 0 {
                writer.write_all(b"|")?;
            }
            writer.write_all(name.as_bytes())?;
        }
        writer.write_all(b",\n")?;

        // Encode each of the capabilities in the order specified
        for cap in self.capabilities {
            writer.write_all(b"\t")?;
            writer.write_all(cap.name.as_bytes())?;
            match cap.value {
                Value::Canceled => writer.write_all(b"@")?,
                Value::Boolean => {}
                Value::Numeric(v) => write!(writer, "#{v}")?,
                Value::String(v) => write!(writer, "={v}")?,
            }
            writer.write_all(b",\n")?;
        }
        Ok(())
    }

    /// Encode to an owned `String`.
    pub fn encode_to_string(&self) -> String {
        let mut buf = Vec::new();
        // Writing to a Vec cannot fail.
        let _ = self.encode(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// Returns a map for all of the capabilities in this terminfo. The key is
    /// the hex-encoded (uppercase) capability name and the value is what
    /// should be sent as a response to XTGETTCAP.
    /// Important: the value is the FULL response including the escape
    /// sequences.
    ///
    /// Panics on canceled capabilities (unsupported, like the Zig
    /// `@compileError`) and on `^` control characters that are not at the
    /// start of a string.
    pub fn xtgettcap_map(&self) -> HashMap<String, String> {
        let mut kvs: Vec<(String, String)> = Vec::with_capacity(self.capabilities.len() + 3);

        // We have all of our capabilities plus TN, Co, and RGB which aren't
        // in the capabilities list but are query-able.
        kvs.push(("TN".into(), self.names[0].into()));
        kvs.push(("Co".into(), "256".into()));
        kvs.push(("RGB".into(), "8".into()));
        for cap in self.capabilities {
            let value = match cap.value {
                Value::Canceled => panic!("canceled not handled yet"),
                Value::Boolean => String::new(),
                Value::String(v) => {
                    // If a string contains parameters, then we do not escape
                    // anything within the string. I BELIEVE the history here is
                    // xterm initially only supported specific capabilities and
                    // none had parameters so it returned the tcap encoded form.
                    // Later, Kitty added support for more capabilities some of
                    // which have parameters. But Kitty returned them in
                    // terminfo source format. So we need to handle both cases.
                    if v.contains('%') {
                        v.to_string()
                    } else {
                        // No-parameters. Encode and return.
                        // First replace \E with the escape char (0x1B)
                        let mut result = v.replace("\\E", "\x1b");
                        // Replace '^' with the control char version of that char.
                        while let Some(idx) = result.find('^') {
                            assert!(idx == 0, "handle control-char in middle of string");
                            let c = result.as_bytes()[idx + 1];
                            let replacement = match c {
                                b'?' => 0x7F, // DEL, special cased from ncurses
                                c => c - 64,
                            };
                            let pat = result[idx..idx + 2].to_string();
                            result = result.replace(&pat, &char::from(replacement).to_string());
                        }
                        result
                    }
                }
                Value::Numeric(v) => v.to_string(),
            };
            kvs.push((cap.name.to_string(), value));
        }

        // Now go through and convert them all to hex-encoded strings.
        kvs.into_iter()
            .map(|(k, v)| {
                let key = hexencode(k.as_bytes());
                let out = if v.is_empty() {
                    format!("\x1bP1+r{key}\x1b\\")
                } else {
                    format!("\x1bP1+r{key}={}\x1b\\", hexencode(v.as_bytes()))
                };
                (key, out)
            })
            .collect()
    }
}

/// Uppercase hex encoding (`std.fmt.bytesToHex(.., .upper)`).
pub fn hexencode(input: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(input.len() * 2);
    for b in input {
        let _ = write!(s, "{b:02X}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn cap(name: &'static str, value: Value) -> Capability {
        Capability { name, value }
    }

    #[test]
    fn xtgettcap_map() {
        static SRC: Source = Source {
            names: &["ghostty", "xterm-ghostty", "Ghostty"],
            capabilities: &[
                cap("am", Value::Boolean),
                cap("colors", Value::Numeric(256)),
                cap("kx", Value::String("^?")),
                cap("kbs", Value::String("^H")),
                cap("kf1", Value::String("\\EOP")),
                cap("Smulx", Value::String("\\E[4:%p1%dm")),
            ],
        };

        let map = SRC.xtgettcap_map();
        assert_eq!("\x1bP1+r616D\x1b\\", map[&hexencode(b"am")]);
        assert_eq!("\x1bP1+r6B78=7F\x1b\\", map[&hexencode(b"kx")]);
        assert_eq!("\x1bP1+r6B6273=08\x1b\\", map[&hexencode(b"kbs")]);
        assert_eq!("\x1bP1+r6B6631=1B4F50\x1b\\", map[&hexencode(b"kf1")]);
        assert_eq!(
            "\x1bP1+r536D756C78=5C455B343A25703125646D\x1b\\",
            map[&hexencode(b"Smulx")]
        );
    }

    #[test]
    fn encode() {
        static SRC: Source = Source {
            names: &["ghostty", "xterm-ghostty", "Ghostty"],
            capabilities: &[
                cap("am", Value::Boolean),
                cap("ccc", Value::Canceled),
                cap("colors", Value::Numeric(256)),
                cap("bel", Value::String("^G")),
            ],
        };

        let expected = "ghostty|xterm-ghostty|Ghostty,\n\tam,\n\tccc@,\n\tcolors#256,\n\tbel=^G,\n";
        assert_eq!(expected, SRC.encode_to_string());
    }
}
