//! SIMD-optimized routines (ZIG/src/simd). The Rust port is scalar-only:
//! `memchr` provides the vectorized byte search, and UTF-8 decoding
//! reproduces the observable behaviour of the C++ SIMD path (`vt.cpp`:
//! simdutf + utfcpp `replace_invalid` + `trim_partial_utf8`), which is what a
//! default Ghostty build runs. (It differs from Zig's scalar fallback
//! `utf8DecodeUntilControlSeqScalar` for ill-formed input; found by the
//! differential fuzzer, see docs/rewrite/notes/difftest-DF.md.)
//!
//! Not ported: `codepoint_width` (belongs to `ghostty-unicode`) and
//! `base64` (the `base64` crate is used by `kitty::graphics_command`).

#![allow(clippy::single_match_else)]

/// Find the first occurrence of `needle` in `input`.
pub fn index_of(input: &[u8], needle: u8) -> Option<usize> {
    memchr::memchr(needle, input)
}

/// Result of [`utf8_decode_until_control_seq`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeResult {
    /// Number of input bytes consumed.
    pub consumed: usize,
    /// Number of codepoints written to the output.
    pub decoded: usize,
}

/// `simdutf::scalar::utf8::trim_partial_utf8`: the length of `input` with a
/// trailing (possibly) incomplete UTF-8 sequence removed. Purely byte-value
/// based (it does not validate), exactly like the C++.
fn trim_partial_utf8(input: &[u8]) -> usize {
    let length = input.len();
    if length < 3 {
        match length {
            2 => {
                if input[1] >= 0xc0 {
                    return 1;
                }
                if input[0] >= 0xe0 {
                    return 0;
                }
                return length;
            }
            1 => {
                if input[0] >= 0xc0 {
                    return 0;
                }
                return length;
            }
            _ => return length,
        }
    }
    if input[length - 1] >= 0xc0 {
        return length - 1;
    }
    if input[length - 2] >= 0xe0 {
        return length - 2;
    }
    if input[length - 3] >= 0xf0 {
        return length - 3;
    }
    length
}

/// Decode `input` into `output` replacing invalid sequences with U+FFFD the
/// way `DecodeUTF8` in `vt.cpp` does: simdutf for the all-valid fast path and
/// `utf8::replace_invalid` (utfcpp) otherwise. Because replacement output is
/// valid UTF-8, this is equivalent to a single utfcpp pass:
///
/// - an invalid lead byte (`0x80..=0xC1`-style continuation bytes, `0xF8..`)
///   becomes one U+FFFD and only that byte is skipped;
/// - an incomplete sequence, overlong encoding, surrogate or out-of-range
///   code point becomes ONE U+FFFD and the lead byte plus every following
///   continuation byte is skipped;
/// - a sequence truncated by the end of input becomes one U+FFFD.
fn decode_utf8_replace(input: &[u8], output: &mut [u32]) -> usize {
    #[inline]
    fn is_trail(b: u8) -> bool {
        b & 0xC0 == 0x80
    }
    let mut count = 0;
    let mut i = 0;
    while i < input.len() {
        let lead = input[i];
        let len = if lead < 0x80 {
            1
        } else if lead >> 5 == 0x6 {
            2
        } else if lead >> 4 == 0xe {
            3
        } else if lead >> 3 == 0x1e {
            4
        } else {
            // INVALID_LEAD
            output[count] = 0xFFFD;
            count += 1;
            i += 1;
            continue;
        };
        // Gather the sequence, tracking utfcpp's error classes.
        let mut cp = if len == 1 {
            u32::from(lead)
        } else {
            u32::from(lead) & (0x7f >> len)
        };
        let mut err = None; // Some(true) = NOT_ENOUGH_ROOM, Some(false) = other
        for k in 1..len {
            match input.get(i + k) {
                None => {
                    err = Some(true);
                    break;
                }
                Some(&b) if !is_trail(b) => {
                    err = Some(false);
                    break;
                }
                Some(&b) => cp = (cp << 6) | u32::from(b & 0x3f),
            }
        }
        if err.is_none() {
            let valid = cp <= 0x10_FFFF && !(0xD800..=0xDFFF).contains(&cp);
            let overlong = match cp {
                0..=0x7f => len != 1,
                0x80..=0x7ff => len != 2,
                0x800..=0xffff => len != 3,
                _ => false,
            };
            if valid && !overlong {
                output[count] = cp;
                count += 1;
                i += len;
                continue;
            }
            err = Some(false);
        }
        output[count] = 0xFFFD;
        count += 1;
        if err == Some(true) {
            // NOT_ENOUGH_ROOM: one replacement for the truncated tail.
            break;
        }
        i += 1;
        while i < input.len() && is_trail(input[i]) {
            i += 1;
        }
    }
    count
}

/// Decode UTF-8 from `input` into `output` until an ESC (0x1B) is found,
/// faithfully following `ghostty_simd_decode_utf8_until_control_seq`
/// (`ZIG/src/simd/vt.cpp`, the real SIMD path of a default Ghostty build):
///
/// - if an ESC is found, everything before it is decoded (invalid and
///   truncated sequences become U+FFFD, see [`decode_utf8_replace`]) and the
///   ESC position is returned;
/// - otherwise a trailing partial sequence is trimmed with
///   `trim_partial_utf8` (a byte-value heuristic, it may trim valid trailing
///   bytes too) and the rest is decoded; the caller feeds the untouched tail
///   to the scalar decoder.
///
/// `output.len()` must be at least `input.len()`.
pub fn utf8_decode_until_control_seq(input: &[u8], output: &mut [u32]) -> DecodeResult {
    assert!(output.len() >= input.len());
    if let Some(esc) = index_of(input, 0x1B) {
        let decoded = decode_utf8_replace(&input[..esc], output);
        return DecodeResult {
            consumed: esc,
            decoded,
        };
    }
    let trimmed = trim_partial_utf8(input);
    let decoded = decode_utf8_replace(&input[..trimmed], output);
    DecodeResult {
        consumed: trimmed,
        decoded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_valid_matches_std() {
        let mut out = [0u32; 8];
        for cp in (0..=0x10FFFFu32)
            .step_by(7)
            .chain([0x7f, 0x80, 0x7ff, 0x800, 0xffff, 0x10000, 0x10FFFF])
        {
            let Some(c) = char::from_u32(cp) else {
                continue;
            };
            if c == '\x1b' {
                continue;
            }
            let mut b = [0u8; 4];
            let s = c.encode_utf8(&mut b);
            assert_eq!(decode_utf8_replace(s.as_bytes(), &mut out), 1);
            assert_eq!(out[0], cp);
        }
    }

    // Regressions found by the differential fuzzer (utfcpp replace_invalid
    // semantics: ONE U+FFFD per invalid sequence, trail bytes swallowed).
    #[test]
    fn decode_replace_swallows_trail_bytes() {
        let mut out = [0u32; 16];
        // overlong lead + trail
        assert_eq!(decode_utf8_replace(b"\xc0\xa9", &mut out), 1);
        assert_eq!(out[0], 0xFFFD);
        // truncated 3-byte sequence followed by a non-trail byte
        assert_eq!(decode_utf8_replace(b"\xe4\xa9\x41", &mut out), 2);
        assert_eq!(&out[..2], &[0xFFFD, 0x41]);
        // surrogate
        assert_eq!(decode_utf8_replace(b"\xed\xa0\x80", &mut out), 1);
        // invalid lead only skips itself
        assert_eq!(decode_utf8_replace(b"\xff\x80", &mut out), 2);
        // stray trail bytes are each invalid leads
        assert_eq!(decode_utf8_replace(b"\x80\x80", &mut out), 2);
        // truncated at end of input: one replacement
        assert_eq!(decode_utf8_replace(b"\xf0\x9f", &mut out), 1);
    }

    #[test]
    fn trim_partial_is_byte_value_based() {
        assert_eq!(trim_partial_utf8(b"ab\xe4\x41"), 2);
        assert_eq!(trim_partial_utf8(b"abc"), 3);
        assert_eq!(trim_partial_utf8(b"ab\xff"), 2);
        assert_eq!(trim_partial_utf8(b"\xe4\xb8"), 0);
        assert_eq!(trim_partial_utf8(b""), 0);
    }

    #[test]
    fn index_of_basic() {
        assert_eq!(index_of(b"hello", b' '), None);
        assert_eq!(index_of(b"hi lo", b' '), Some(2));
        assert_eq!(
            index_of(
                b"XXXXX XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX\nXXXXXXXXXXXX XXXXXXXXXXX XXXXXXXXXXXXXXX",
                b' '
            ),
            Some(5)
        );
        assert_eq!(
            index_of(
                b"XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX\nXXXXXXXXXXXX XXXXXXXXXXX XXXXXXXXXXXXXXX",
                b' '
            ),
            Some(53)
        );
    }

    #[test]
    fn decode_no_escape() {
        let mut output = [0u32; 1024];
        let s = "hello".repeat(128);
        assert_eq!(
            utf8_decode_until_control_seq(s.as_bytes(), &mut output),
            DecodeResult {
                consumed: s.len(),
                decoded: s.len()
            }
        );
    }

    #[test]
    fn decode_ascii_to_escape() {
        let mut output = [0u32; 1024];
        let prefix = "hello".repeat(64);
        let s = format!("{prefix}\x1bworld");
        assert_eq!(
            utf8_decode_until_control_seq(s.as_bytes(), &mut output),
            DecodeResult {
                consumed: prefix.len(),
                decoded: prefix.len()
            }
        );
    }

    #[test]
    fn decode_immediate_esc_sequence() {
        let mut output = [0u32; 64];
        assert_eq!(
            utf8_decode_until_control_seq(b"\x1b[?5s", &mut output),
            DecodeResult {
                consumed: 0,
                decoded: 0
            }
        );
    }

    #[test]
    fn decode_incomplete_utf8() {
        let mut output = [0u32; 64];
        for s in [
            &b"hello\xc2"[..],
            &b"hello\xe0\x00"[..],
            &b"hello\xf0\x90"[..],
        ] {
            assert_eq!(
                utf8_decode_until_control_seq(s, &mut output),
                DecodeResult {
                    consumed: 5,
                    decoded: 5
                }
            );
        }
    }

    #[test]
    fn decode_invalid_utf8() {
        let mut output = [0u32; 64];
        assert_eq!(
            utf8_decode_until_control_seq(b"hello\xc2\x01", &mut output),
            DecodeResult {
                consumed: 7,
                decoded: 7
            }
        );
        // Replacement only replaces the invalid leading byte.
        assert_eq!(output[5], 0xFFFD);
        assert_eq!(output[6], 0x01);
    }

    #[test]
    fn decode_invalid_leading_byte_isnt_consumed_or_replaced() {
        let mut output = [0u32; 64];
        assert_eq!(
            utf8_decode_until_control_seq(b"hello\xFF", &mut output),
            DecodeResult {
                consumed: 5,
                decoded: 5
            }
        );
    }

    #[test]
    fn decode_multibyte() {
        let mut output = [0u32; 64];
        let r = utf8_decode_until_control_seq("a\u{e9}\u{4e00}\u{1f600}".as_bytes(), &mut output);
        assert_eq!(r.decoded, 4);
        assert_eq!(&output[..4], &[0x61, 0xE9, 0x4E00, 0x1F600]);
    }
}
