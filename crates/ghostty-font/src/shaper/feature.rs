//! OpenType font feature settings. Port of `font/shaper/feature.zig`.

// The explicit `continue`s mirror the Zig state machine one-to-one.
#![allow(clippy::needless_continue, clippy::single_match_else)]

use std::fmt;

/// A minimal byte reader mirroring the `std.io.fixedBufferStream` reader
/// semantics that the Zig parser relies on: reading past the end yields a
/// `,` (the parser uses `reader.readByte() catch ','`).
pub struct Reader<'a> {
    buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    /// Read a byte; at end of stream returns `,` without advancing.
    fn read_byte(&mut self) -> u8 {
        match self.buf.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                b
            }
            None => b',',
        }
    }

    /// Skip until after the next `,` or end of stream.
    fn skip_until_delimiter_or_eof(&mut self) {
        while let Some(&b) = self.buf.get(self.pos) {
            self.pos += 1;
            if b == b',' {
                break;
            }
        }
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.buf.len()
    }
}

/// Represents an OpenType font feature setting, which consists of a tag and
/// a numeric parameter >= 0. Most features are boolean, so only parameters
/// of 0 and 1 make sense for them, but some (e.g. 'cv01'..'cv99') can take
/// parameters to choose between multiple variants of a given character or
/// characters.
///
/// Ref:
/// - https://learn.microsoft.com/en-us/typography/opentype/spec/chapter2#features-and-lookups
/// - https://harfbuzz.github.io/shaping-opentype-features.html
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Feature {
    pub tag: [u8; 4],
    pub value: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Initial state.
    Start,
    /// Parsing the tag.
    Tag,
    /// In the space between the tag and the value.
    Space,
    /// Parsing an integer parameter directly in to `value`.
    Int,
    /// Parsing a boolean keyword parameter ("on"/"off").
    Bool,
    /// Encountered an unrecoverable syntax error, advancing to boundary.
    Err,
    /// Done parsing feature, skip whitespace until end.
    Done,
    /// Successfully finished.
    Finished,
}

impl Feature {
    pub fn from_string(s: &str) -> Option<Feature> {
        Self::from_bytes(s.as_bytes())
    }

    pub fn from_bytes(s: &[u8]) -> Option<Feature> {
        Self::from_reader(&mut Reader::new(s))
    }

    /// Parse a single font feature setting from a reader, with a version
    /// of the syntax of HarfBuzz's font feature strings. Stops at end of stream
    /// or when a ',' is encountered.
    ///
    /// This parsing aims to be as error-tolerant as possible while avoiding any
    /// assumptions in ambiguous scenarios. When invalid syntax is encountered,
    /// the reader is advanced to the next boundary (end-of-stream or ',') so
    /// that further features may be read.
    ///
    /// Ref: https://harfbuzz.github.io/harfbuzz-hb-common.html#hb-feature-from-string
    pub fn from_reader(reader: &mut Reader<'_>) -> Option<Feature> {
        let mut tag = [0u8; 4];
        let mut tag_len = 0usize;
        let mut value: Option<u32> = None;
        let mut state = State::Start;

        loop {
            match state {
                // If we're done then we skip whitespace until we see a ','.
                State::Finished => break,
                State::Done => loop {
                    match reader.read_byte() {
                        b' ' | b'\t' => continue,
                        b',' => {
                            state = State::Finished;
                            break;
                        }
                        // If we see something other than whitespace or a ','
                        // then this is an error since the intent is unclear.
                        _ => {
                            state = State::Err;
                            break;
                        }
                    }
                },

                // If we're fast-forwarding from an error we just wanna
                // stop at the first boundary and ignore all other bytes.
                State::Err => {
                    reader.skip_until_delimiter_or_eof();
                    return None;
                }

                State::Start => loop {
                    match reader.read_byte() {
                        // Ignore leading whitespace.
                        b' ' | b'\t' => continue,
                        // Empty feature string.
                        b',' => return None,
                        // '+' prefix to explicitly enable feature.
                        b'+' => {
                            value = Some(1);
                            state = State::Tag;
                            break;
                        }
                        // '-' prefix to explicitly disable feature.
                        b'-' => {
                            value = Some(0);
                            state = State::Tag;
                            break;
                        }
                        // Quote mark introducing a tag.
                        b'"' | b'\'' => {
                            state = State::Tag;
                            break;
                        }
                        // First letter of tag.
                        byte => {
                            tag_len = 1;
                            tag[0] = byte;
                            state = State::Tag;
                            break;
                        }
                    }
                },

                State::Tag => loop {
                    match reader.read_byte() {
                        // If the tag is interrupted by a comma it's invalid.
                        b',' => return None,
                        // Ignore quote marks. This does technically ignore cases like
                        // "'k'e'r'n' = 0", but it's unambiguous so if someone really
                        // wants to do that in their config then... sure why not.
                        b'"' | b'\'' => continue,
                        // In all other cases we add the byte to our tag.
                        byte => {
                            tag[tag_len] = byte;
                            tag_len += 1;
                            if tag_len == 4 {
                                state = State::Space;
                                break;
                            }
                        }
                    }
                },

                State::Space => {
                    loop {
                        match reader.read_byte() {
                            b' ' | b'\t' => continue,
                            // Ignore quote marks since we might have a
                            // closing quote from the tag still ahead.
                            b'"' | b'\'' => continue,
                            // Allow an '=' (which we can safely ignore)
                            // only if we don't already have a value due
                            // to a '+' or '-' prefix.
                            b'=' => {
                                if value.is_some() {
                                    state = State::Err;
                                    break;
                                }
                            }
                            b',' => {
                                // Specifying only a tag turns a feature on.
                                if value.is_none() {
                                    value = Some(1);
                                }
                                state = State::Finished;
                                break;
                            }
                            byte @ b'0'..=b'9' => {
                                // If we already have value because of a
                                // '+' or '-' prefix then this is an error.
                                if value.is_some() {
                                    state = State::Err;
                                } else {
                                    value = Some(u32::from(byte - b'0'));
                                    state = State::Int;
                                }
                                break;
                            }
                            b'o' | b'O' => {
                                // If we already have value because of a
                                // '+' or '-' prefix then this is an error.
                                if value.is_some() {
                                    state = State::Err;
                                } else {
                                    state = State::Bool;
                                }
                                break;
                            }
                            _ => {
                                state = State::Err;
                                break;
                            }
                        }
                    }
                }

                State::Int => {
                    loop {
                        match reader.read_byte() {
                            b',' => {
                                state = State::Finished;
                                break;
                            }
                            byte @ b'0'..=b'9' => {
                                // If our value gets too big while
                                // parsing we consider it an error.
                                let v = value.unwrap_or(0);
                                match v
                                    .checked_mul(10)
                                    .and_then(|v| v.checked_add(u32::from(byte - b'0')))
                                {
                                    Some(n) => value = Some(n),
                                    None => {
                                        state = State::Err;
                                        break;
                                    }
                                }
                            }
                            _ => {
                                state = State::Err;
                                break;
                            }
                        }
                    }
                }

                State::Bool => loop {
                    match reader.read_byte() {
                        b',' => return None,
                        b'n' | b'N' => {
                            // "ofn"
                            if value.is_some() {
                                debug_assert_eq!(value, Some(0));
                                state = State::Err;
                                break;
                            }
                            value = Some(1);
                            state = State::Done;
                            break;
                        }
                        b'f' | b'F' => {
                            // To make sure we consume two 'f's.
                            if value.is_none() {
                                value = Some(0);
                            } else {
                                debug_assert_eq!(value, Some(0));
                                state = State::Done;
                                break;
                            }
                        }
                        _ => {
                            state = State::Err;
                            break;
                        }
                    }
                },
            }

            if state == State::Finished {
                break;
            }
        }

        debug_assert!(value.is_some());
        debug_assert_eq!(tag_len, 4);

        Some(Feature { tag, value: value? })
    }
}

impl fmt::Display for Feature {
    /// Serialize this feature. The string that this produces should be valid
    /// to parse.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = String::from_utf8_lossy(&self.tag);
        if self.value <= 1 {
            // Format boolean options as "+tag" for on and "-tag" for off.
            write!(f, "{}{}", if self.value == 1 { '+' } else { '-' }, tag)
        } else {
            // Format non-boolean tags as "tag=value".
            write!(f, "{tag}={}", self.value)
        }
    }
}

/// A list of font feature settings (see `Feature` for more documentation).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeatureList {
    pub features: Vec<Feature>,
}

impl FeatureList {
    /// Parse a comma separated list of features.
    /// See `Feature::from_reader` for more docs.
    pub fn from_string(s: &str) -> FeatureList {
        let mut this = FeatureList::default();
        this.append_from_string(s);
        this
    }

    /// Append features to this list from a string with a comma separated list.
    /// See `Feature::from_reader` for more docs.
    pub fn append_from_string(&mut self, s: &str) {
        let mut reader = Reader::new(s.as_bytes());
        while !reader.at_end() {
            let i = reader.pos;
            if let Some(feature) = Feature::from_reader(&mut reader) {
                self.features.push(feature);
            } else {
                log::warn!(
                    "failed to parse font feature setting: \"{}\"",
                    String::from_utf8_lossy(&s.as_bytes()[i..reader.pos])
                );
            }
        }
    }
}

impl fmt::Display for FeatureList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, feature) in self.features.iter().enumerate() {
            if i != 0 {
                f.write_str(", ")?;
            }
            write!(f, "{feature}")?;
        }
        Ok(())
    }
}

/// These features are hardcoded to always be on by default. Users
/// can turn them off by setting the features to "-liga" for example.
pub const DEFAULT_FEATURES: [Feature; 1] = [Feature {
    tag: *b"liga",
    value: 1,
}];

#[cfg(test)]
mod tests {
    use super::*;

    fn f(tag: [u8; 4], value: u32) -> Feature {
        Feature { tag, value }
    }

    #[test]
    fn feature_from_string() {
        // This is not *complete* coverage of every possible
        // combination of syntax, but it covers quite a few.

        // Boolean settings (on)
        let kern_on = Some(f(*b"kern", 1));
        for s in [
            "kern",
            "kern, ",
            "kern on",
            "kern on, ",
            "+kern",
            "+kern, ",
            "\"kern\" = 1",
            "\"kern\" = 1, ",
        ] {
            assert_eq!(kern_on, Feature::from_string(s), "{s}");
        }

        // Boolean settings (off)
        let kern_off = Some(f(*b"kern", 0));
        for s in [
            "kern off",
            "kern off, ",
            "-'kern'",
            "-'kern', ",
            "\"kern\" = 0",
            "\"kern\" = 0, ",
        ] {
            assert_eq!(kern_off, Feature::from_string(s), "{s}");
        }

        // Non-boolean settings
        let aalt_2 = Some(f(*b"aalt", 2));
        for s in ["aalt=2", "aalt=2, ", "'aalt' 2", "'aalt' 2, "] {
            assert_eq!(aalt_2, Feature::from_string(s), "{s}");
        }

        // Various ambiguous/error cases which should be null
        for s in [
            "aalt=2x",   // bad number
            "toolong",   // tag too long
            "sht",       // tag too short
            "-kern 1",   // redundant/conflicting
            "-kern on",  // redundant/conflicting
            "aalt=o,",   // bad keyword
            "aalt=ofn,", // bad keyword
        ] {
            assert_eq!(None, Feature::from_string(s), "{s}");
        }
    }

    #[test]
    fn feature_list_from_string() {
        let s = concat!(
            "  kern, kern on , +kern, \"kern\"  = 1,", // Boolean settings (on)
            "kern    off, -'kern' , \"kern\"=0,",      // Boolean settings (off)
            "aalt=2,  'aalt'\t2,",                     // Non-boolean settings
            "aalt=2x, toolong, sht, -kern 1, -kern on, aalt=o, aalt=ofn,", // Invalid cases
            "last", // To ensure final element is included correctly.
        );
        let feats = FeatureList::from_string(s);
        let mut expected = vec![f(*b"kern", 1); 4];
        expected.extend(vec![f(*b"kern", 0); 3]);
        expected.extend(vec![f(*b"aalt", 2); 2]);
        expected.push(f(*b"last", 1));
        assert_eq!(expected, feats.features);
    }

    #[test]
    fn feature_display() {
        assert_eq!("+kern", f(*b"kern", 1).to_string());
        assert_eq!("-kern", f(*b"kern", 0).to_string());
        assert_eq!("aalt=3", f(*b"aalt", 3).to_string());
        assert_eq!(
            "+liga, ss01=2",
            FeatureList::from_string("liga, ss01=2").to_string()
        );
    }

    #[test]
    fn default_features_has_liga() {
        assert_eq!(DEFAULT_FEATURES, [f(*b"liga", 1)]);
    }
}
