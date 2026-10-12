//! Iterator to split a string into fields by commas, taking into account
//! quotes and escapes.
//!
//! Supports the same escapes as in Zig literal strings.
//!
//! Quotes must begin and end with a double quote (`"`). It is an error to not
//! end a quote that was begun. To include a double quote inside a quote (or to
//! not have a double quote start a quoted section) escape it with a backslash.
//!
//! Single quotes (`'`) are not special, they do not begin a quoted block.
//!
//! Zig multiline string literals are NOT supported.
//!
//! Quotes and escapes are not stripped or decoded, that must be handled as a
//! separate step!

/// Errors the splitter can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    UnclosedQuote,
    UnfinishedEscape,
    IllegalEscape,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Error::UnclosedQuote => "UnclosedQuote",
            Error::UnfinishedEscape => "UnfinishedEscape",
            Error::IllegalEscape => "IllegalEscape",
        };
        f.write_str(s)
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Quoted,
    Escape,
    HexEscape,
    UnicodeEscape,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UnicodeState {
    Start,
    Digits,
}

/// Splits a string on commas (see module docs).
#[derive(Debug, Clone)]
pub struct CommaSplitter<'a> {
    /// the string that we are splitting
    str: &'a [u8],
    /// how much of the string has been consumed so far
    index: usize,
}

impl<'a> CommaSplitter<'a> {
    pub fn new(value: &'a str) -> Self {
        Self::from_bytes(value.as_bytes())
    }

    pub fn next_field(&mut self) -> Result<Option<&'a str>, Error> {
        self.next_bytes()?
            .map(|b| std::str::from_utf8(b).map_err(|_| Error::IllegalEscape))
            .transpose()
    }

    /// initialize a splitter with the given string
    #[must_use]
    pub fn from_bytes(str: &'a [u8]) -> Self {
        Self { str, index: 0 }
    }

    /// return the next field, `Ok(None)` if no more fields
    pub fn next_bytes(&mut self) -> Result<Option<&'a [u8]>, Error> {
        let bytes = self.str;
        if self.index >= bytes.len() {
            return Ok(None);
        }

        let start = self.index;
        let mut state = State::Normal;
        // state to return to when done processing an escape sequence
        let mut last = State::Normal;
        let mut hexescape_digits = 0usize;
        let mut unicodeescape_state = UnicodeState::Start;
        let mut unicodeescape_digits = 0usize;
        let mut unicodeescape_value = 0usize;

        loop {
            match state {
                State::Normal => {
                    if self.index >= bytes.len() {
                        return Ok(Some(&self.str[start..]));
                    }
                    match bytes[self.index] {
                        b',' => {
                            self.index += 1;
                            return Ok(Some(&self.str[start..self.index - 1]));
                        }
                        b'"' => {
                            self.index += 1;
                            state = State::Quoted;
                        }
                        b'\\' => {
                            self.index += 1;
                            last = State::Normal;
                            state = State::Escape;
                        }
                        _ => self.index += 1,
                    }
                }
                State::Quoted => {
                    if self.index >= bytes.len() {
                        return Err(Error::UnclosedQuote);
                    }
                    match bytes[self.index] {
                        b'"' => {
                            self.index += 1;
                            state = State::Normal;
                        }
                        b'\\' => {
                            self.index += 1;
                            last = State::Quoted;
                            state = State::Escape;
                        }
                        _ => self.index += 1,
                    }
                }
                State::Escape => {
                    if self.index >= bytes.len() {
                        return Err(Error::UnfinishedEscape);
                    }
                    match bytes[self.index] {
                        b'n' | b'r' | b't' | b'\\' | b'\'' | b'"' => {
                            self.index += 1;
                            state = last;
                        }
                        b'x' => {
                            self.index += 1;
                            hexescape_digits = 0;
                            state = State::HexEscape;
                        }
                        b'u' => {
                            self.index += 1;
                            unicodeescape_state = UnicodeState::Start;
                            unicodeescape_digits = 0;
                            unicodeescape_value = 0;
                            state = State::UnicodeEscape;
                        }
                        _ => return Err(Error::IllegalEscape),
                    }
                }
                State::HexEscape => {
                    if self.index >= bytes.len() {
                        return Err(Error::UnfinishedEscape);
                    }
                    if bytes[self.index].is_ascii_hexdigit() {
                        self.index += 1;
                        hexescape_digits += 1;
                        if hexescape_digits == 2 {
                            state = last;
                        }
                    } else {
                        return Err(Error::IllegalEscape);
                    }
                }
                State::UnicodeEscape => {
                    if self.index >= bytes.len() {
                        return Err(Error::UnfinishedEscape);
                    }
                    match unicodeescape_state {
                        UnicodeState::Start => {
                            if bytes[self.index] == b'{' {
                                self.index += 1;
                                unicodeescape_value = 0;
                                unicodeescape_state = UnicodeState::Digits;
                            } else {
                                return Err(Error::IllegalEscape);
                            }
                        }
                        UnicodeState::Digits => {
                            let c = bytes[self.index];
                            // Note: the Zig implementation does not add 10 for
                            // the a-f digits; this is preserved.
                            let d = match c {
                                b'}' => {
                                    self.index += 1;
                                    if unicodeescape_digits == 0 {
                                        return Err(Error::IllegalEscape);
                                    }
                                    state = last;
                                    continue;
                                }
                                b'0'..=b'9' => c - b'0',
                                b'a'..=b'f' => c - b'a',
                                b'A'..=b'F' => c - b'A',
                                _ => return Err(Error::IllegalEscape),
                            };
                            self.index += 1;
                            unicodeescape_digits += 1;
                            unicodeescape_value <<= 4;
                            unicodeescape_value += usize::from(d);
                            if unicodeescape_value > 0x10ffff {
                                return Err(Error::IllegalEscape);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Return any remaining string data, whether it has a comma or not.
    pub fn rest(&mut self) -> Option<&'a str> {
        if self.index >= self.str.len() {
            return None;
        }
        let r = &self.str[self.index..];
        self.index = self.str.len();
        std::str::from_utf8(r).ok()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn all(s: &str) -> Result<Vec<&str>, Error> {
        let mut sp = CommaSplitter::new(s);
        let mut v = vec![];
        while let Some(x) = sp.next_field()? {
            v.push(x);
        }
        Ok(v)
    }

    fn first_err(s: &str) -> Error {
        CommaSplitter::new(s).next_field().unwrap_err()
    }

    #[test]
    fn splitter_1() {
        assert_eq!(all("a,b,c").unwrap(), ["a", "b", "c"]);
    }
    #[test]
    fn splitter_2() {
        assert_eq!(all("").unwrap(), Vec::<&str>::new());
    }
    #[test]
    fn splitter_3() {
        assert_eq!(all("a").unwrap(), ["a"]);
    }
    #[test]
    fn splitter_4() {
        assert_eq!(all("\\x5a").unwrap(), ["\\x5a"]);
    }
    #[test]
    fn splitter_5() {
        assert_eq!(all("'a',b").unwrap(), ["'a'", "b"]);
    }
    #[test]
    fn splitter_6() {
        assert_eq!(all("'a,b',c").unwrap(), ["'a", "b'", "c"]);
    }
    #[test]
    fn splitter_7() {
        assert_eq!(all("\"a,b\",c").unwrap(), ["\"a,b\"", "c"]);
    }
    #[test]
    fn splitter_8() {
        assert_eq!(all(" a , b ").unwrap(), [" a ", " b "]);
    }
    #[test]
    fn splitter_9() {
        assert_eq!(first_err("\\x"), Error::UnfinishedEscape);
    }
    #[test]
    fn splitter_10() {
        assert_eq!(first_err("\\x5"), Error::UnfinishedEscape);
    }
    #[test]
    fn splitter_11() {
        assert_eq!(first_err("\\u"), Error::UnfinishedEscape);
    }
    #[test]
    fn splitter_12() {
        assert_eq!(first_err("\\u{"), Error::UnfinishedEscape);
    }
    #[test]
    fn splitter_13() {
        assert_eq!(first_err("\\u{}"), Error::IllegalEscape);
    }
    #[test]
    fn splitter_14() {
        assert_eq!(first_err("\\u{h1}"), Error::IllegalEscape);
    }
    #[test]
    fn splitter_15() {
        assert_eq!(all("\\u{10ffff}").unwrap(), ["\\u{10ffff}"]);
    }
    #[test]
    fn splitter_16() {
        assert_eq!(first_err("\\u{110000}"), Error::IllegalEscape);
    }
    #[test]
    fn splitter_17() {
        assert_eq!(first_err("\\d"), Error::IllegalEscape);
    }
    #[test]
    fn splitter_18() {
        assert_eq!(
            all("\\n\\r\\t\\\"\\'\\\\").unwrap(),
            ["\\n\\r\\t\\\"\\'\\\\"]
        );
    }
    #[test]
    fn splitter_19() {
        assert_eq!(all("\"abc'def'ghi\"").unwrap(), ["\"abc'def'ghi\""]);
    }
    #[test]
    fn splitter_20() {
        assert_eq!(all("\",\",abc").unwrap(), ["\",\"", "abc"]);
    }
    #[test]
    fn splitter_21() {
        assert_eq!(all("'a','b', 'c'").unwrap(), ["'a'", "'b'", " 'c'"]);
    }
    #[test]
    fn splitter_22() {
        assert_eq!(first_err("abc\"def"), Error::UnclosedQuote);
    }
    #[test]
    fn splitter_23() {
        assert_eq!(
            all("title:\"Focus Split: Up\",description:\"Focus the split above, if it exists.\",action:goto_split:up").unwrap(),
            [
                "title:\"Focus Split: Up\"",
                "description:\"Focus the split above, if it exists.\"",
                "action:goto_split:up"
            ]
        );
    }
    #[test]
    fn splitter_24() {
        let mut s = CommaSplitter::new("a,b,c,def");
        assert_eq!(s.next_field().unwrap(), Some("a"));
        assert_eq!(s.next_field().unwrap(), Some("b"));
        assert_eq!(s.rest(), Some("c,def"));
        assert_eq!(s.next_field().unwrap(), None);
    }
    #[test]
    fn splitter_25() {
        let mut s = CommaSplitter::new("a,\\u{10,df}");
        assert_eq!(s.next_field().unwrap(), Some("a"));
        assert_eq!(s.next_field(), Err(Error::IllegalEscape));
    }
}
