//! SIMD-optimized routines (ZIG/src/simd). The Rust port is scalar-only:
//! `memchr` provides the vectorized byte search, and UTF-8 decoding follows
//! the Zig scalar fallback (`utf8DecodeUntilControlSeqScalar`), which is
//! tested to match the SIMD behavior.
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

/// Length of a UTF-8 sequence given its leading byte, `None` if the byte
/// cannot start a sequence (Zig `std.unicode.utf8ByteSequenceLength`).
fn utf8_byte_sequence_length(first: u8) -> Option<usize> {
    match first {
        0x00..=0x7F => Some(1),
        0xC0..=0xDF => Some(2),
        0xE0..=0xEF => Some(3),
        0xF0..=0xF7 => Some(4),
        _ => None,
    }
}

/// Decode `bytes` (exactly one sequence whose length matches its leading
/// byte) strictly; `None` if ill-formed (overlong, surrogate, out of range or
/// a bad continuation byte). Equivalent to `std::str::from_utf8` on one char.
#[inline]
fn utf8_decode(bytes: &[u8]) -> Option<u32> {
    #[inline]
    fn cont(b: u8) -> bool {
        b & 0xC0 == 0x80
    }
    match *bytes {
        [b0] if b0 < 0x80 => Some(u32::from(b0)),
        [b0 @ 0xC2..=0xDF, b1] if cont(b1) => {
            Some((u32::from(b0 & 0x1F) << 6) | u32::from(b1 & 0x3F))
        }
        [b0 @ 0xE0..=0xEF, b1, b2] if cont(b1) && cont(b2) => {
            let ok = match b0 {
                0xE0 => b1 >= 0xA0,
                0xED => b1 < 0xA0,
                _ => true,
            };
            ok.then(|| {
                (u32::from(b0 & 0x0F) << 12) | (u32::from(b1 & 0x3F) << 6) | u32::from(b2 & 0x3F)
            })
        }
        [b0 @ 0xF0..=0xF4, b1, b2, b3] if cont(b1) && cont(b2) && cont(b3) => {
            let ok = match b0 {
                0xF0 => b1 >= 0x90,
                0xF4 => b1 < 0x90,
                _ => true,
            };
            ok.then(|| {
                (u32::from(b0 & 0x07) << 18)
                    | (u32::from(b1 & 0x3F) << 12)
                    | (u32::from(b2 & 0x3F) << 6)
                    | u32::from(b3 & 0x3F)
            })
        }
        _ => None,
    }
}

/// Decode UTF-8 from `input` into `output` until an ESC (0x1B) is found.
/// `output.len()` must be at least `input.len()`.
///
/// Invalid sequences have their leading byte replaced with U+FFFD and
/// decoding continues after that byte. Decoding stops (without consuming)
/// at an incomplete trailing sequence or at a byte that can't start a
/// sequence (e.g. 0xFF or a stray continuation byte); the caller must handle
/// those bytes itself (see `stream.rs`).
pub fn utf8_decode_until_control_seq(input: &[u8], output: &mut [u32]) -> DecodeResult {
    assert!(output.len() >= input.len());

    let idx = index_of(input, 0x1B).unwrap_or(input.len());
    let decode = &input[..idx];

    let mut decode_offset = 0;
    let mut decode_count = 0;
    while decode_offset < decode.len() {
        // ASCII fast path: a run of single-byte sequences.
        let rem = &decode[decode_offset..];
        if rem[0] < 0x80 {
            let run = rem.iter().take_while(|&&b| b < 0x80).count();
            for (o, &b) in output[decode_count..decode_count + run].iter_mut().zip(rem) {
                *o = u32::from(b);
            }
            decode_count += run;
            decode_offset += run;
            continue;
        }
        let Some(cp_len) = utf8_byte_sequence_length(rem[0]) else {
            break;
        };

        // Partial input: defer to the future.
        if rem.len() < cp_len {
            break;
        }

        match utf8_decode(&rem[..cp_len]) {
            Some(cp) => {
                output[decode_count] = cp;
                decode_offset += cp_len;
            }
            None => {
                // Replace only the leading byte and continue after it.
                output[decode_count] = 0xFFFD;
                decode_offset += 1;
            }
        }
        decode_count += 1;
    }

    DecodeResult {
        consumed: decode_offset,
        decoded: decode_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_decode_matches_std() {
        fn std_decode(bytes: &[u8]) -> Option<u32> {
            std::str::from_utf8(bytes)
                .ok()
                .and_then(|s| s.chars().next())
                .map(|c| c as u32)
        }
        for b0 in 0..=255u8 {
            let Some(len) = utf8_byte_sequence_length(b0) else {
                continue;
            };
            match len {
                1 => assert_eq!(utf8_decode(&[b0]), std_decode(&[b0])),
                2 => {
                    for b1 in 0..=255u8 {
                        assert_eq!(
                            utf8_decode(&[b0, b1]),
                            std_decode(&[b0, b1]),
                            "{b0:x} {b1:x}"
                        );
                    }
                }
                3 => {
                    for b1 in 0..=255u8 {
                        for b2 in 0..=255u8 {
                            assert_eq!(
                                utf8_decode(&[b0, b1, b2]),
                                std_decode(&[b0, b1, b2]),
                                "{b0:x} {b1:x} {b2:x}"
                            );
                        }
                    }
                }
                _ => {
                    for b1 in 0..=255u8 {
                        for b2 in [0x00, 0x7F, 0x80, 0xA0, 0xBF, 0xC0] {
                            for b3 in [0x00, 0x7F, 0x80, 0xBF, 0xC0] {
                                assert_eq!(
                                    utf8_decode(&[b0, b1, b2, b3]),
                                    std_decode(&[b0, b1, b2, b3]),
                                    "{b0:x} {b1:x} {b2:x} {b3:x}"
                                );
                            }
                        }
                    }
                }
            }
        }
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
