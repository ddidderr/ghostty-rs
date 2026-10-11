//! DFA-based non-allocating error-replacing UTF-8 decoder
//! (ZIG/src/terminal/UTF8Decoder.zig).
//!
//! This implementation is based largely on the excellent work of
//! Bjoern Hoehrmann, with slight modifications to support error-
//! replacement.
//!
//! For details on Bjoern's DFA-based UTF-8 decoder, see
//! http://bjoern.hoehrmann.de/utf-8/decoder/dfa (MIT licensed)

#[rustfmt::skip]
const CHAR_CLASSES: [u8; 256] = [
   0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
   0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
   0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
   0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,  0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
   1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,  9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,
   7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,  7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,
   8,8,2,2,2,2,2,2,2,2,2,2,2,2,2,2,  2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,
  10,3,3,3,3,3,3,3,3,3,3,3,3,4,3,3, 11,6,6,6,5,8,8,8,8,8,8,8,8,8,8,8,
];

#[rustfmt::skip]
const TRANSITIONS: [u8; 108] = [
   0,12,24,36,60,96,84,12,12,12,48,72, 12,12,12,12,12,12,12,12,12,12,12,12,
  12, 0,12,12,12,12,12, 0,12, 0,12,12, 12,24,12,12,12,12,12,24,12,24,12,12,
  12,12,12,12,12,12,12,24,12,12,12,12, 12,24,12,12,12,12,12,12,12,24,12,12,
  12,12,12,12,12,12,12,36,12,36,12,12, 12,36,12,12,12,12,12,36,12,36,12,12,
  12,36,12,12,12,12,12,12,12,12,12,12,
];

// DFA states
const ACCEPT_STATE: u8 = 0;
const REJECT_STATE: u8 = 12;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Utf8Decoder {
    // This is where we accumulate our current codepoint.
    accumulator: u32,
    // The internal state of the DFA.
    state: u8,
}

impl Utf8Decoder {
    pub const fn new() -> Self {
        Self {
            accumulator: 0,
            state: ACCEPT_STATE,
        }
    }

    /// Takes the next byte in the utf-8 sequence and emits a tuple of
    /// - The codepoint that was generated, if there is one.
    /// - A boolean that indicates whether the provided byte was consumed.
    ///
    /// The only case where the byte is not consumed is if an ill-formed
    /// sequence is reached, in which case a replacement character will be
    /// emitted and the byte will not be consumed.
    ///
    /// If the byte is not consumed, the caller is responsible for calling
    /// again with the same byte before continuing.
    #[inline]
    pub fn next(&mut self, byte: u8) -> (Option<u32>, bool) {
        let char_class = CHAR_CLASSES[byte as usize];

        let initial_state = self.state;

        if self.state == ACCEPT_STATE {
            self.accumulator = (0xFFu32 >> char_class) & (byte as u32);
        } else {
            self.accumulator <<= 6;
            self.accumulator |= (byte & 0x3F) as u32;
        }

        self.state = TRANSITIONS[(self.state + char_class) as usize];

        if self.state == ACCEPT_STATE {
            let cp = self.accumulator;
            self.accumulator = 0;

            // Emit the fully decoded codepoint.
            (Some(cp), true)
        } else if self.state == REJECT_STATE {
            self.accumulator = 0;
            self.state = ACCEPT_STATE;
            // Emit a replacement character. If we rejected the first byte
            // in a sequence, then it was consumed, otherwise it was not.
            (Some(0xFFFD), initial_state == ACCEPT_STATE)
        } else {
            // Emit nothing, we're in the middle of a sequence.
            (None, true)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn ascii() {
        let mut d = Utf8Decoder::new();
        let mut out = Vec::new();
        for &byte in b"Hello, World!" {
            let res = d.next(byte);
            assert!(res.1);
            if let Some(cp) = res.0 {
                out.push(cp as u8);
            }
        }
        assert_eq!(out, b"Hello, World!");
    }

    #[test]
    fn well_formed_utf8() {
        let mut d = Utf8Decoder::new();
        let mut out = Vec::new();
        // 4 bytes, 3 bytes, 2 bytes, 1 byte
        for &byte in "😄✤ÁA".as_bytes() {
            let mut consumed = false;
            while !consumed {
                let res = d.next(byte);
                consumed = res.1;
                // There are no errors in this sequence, so
                // every byte should be consumed first try.
                assert!(consumed);
                if let Some(cp) = res.0 {
                    out.push(cp);
                }
            }
        }
        assert_eq!(out, vec![0x1F604, 0x2724, 0xC1, 0x41]);
    }

    #[test]
    fn partially_invalid_utf8() {
        let mut d = Utf8Decoder::new();
        let mut out = Vec::new();
        // Illegally terminated sequence, valid sequence, illegal surrogate pair.
        let mut input = vec![0xF0, 0x9F];
        input.extend("😄".as_bytes());
        input.extend([0xED, 0xA0, 0x80]);
        for byte in input {
            let mut consumed = false;
            while !consumed {
                let res = d.next(byte);
                consumed = res.1;
                if let Some(cp) = res.0 {
                    out.push(cp);
                }
            }
        }
        assert_eq!(out, vec![0xFFFD, 0x1F604, 0xFFFD, 0xFFFD, 0xFFFD]);
    }
}
