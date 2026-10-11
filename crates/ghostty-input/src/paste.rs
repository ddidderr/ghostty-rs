//! Paste encoding (ZIG/src/input/paste.zig).

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Returned if encoding requires a mutable copy of the data. This
    /// can only be returned if the input data type is const.
    #[error("mutable copy of the data is required")]
    MutableRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// True if bracketed paste mode is on.
    pub bracketed: bool,
}

impl Options {
    // TODO(integration): Zig has `fromTerminal(t: *const Terminal)` which
    // reads `t.modes.get(.bracketed_paste)`. Construct `Options` from the
    // terminal mode in ghostty-terminal instead.
}

/// These are the set of byte values that are always replaced by
/// a space (per xterm's behavior) for any text insertion method e.g.
/// a paste, drag and drop, etc. These are copied directly from xterm's
/// source.
const STRIP: &[u8] = &[
    0x00, // NUL
    0x08, // BS
    0x05, // ENQ
    0x04, // EOT
    0x1B, // ESC
    0x7F, // DEL
    // These can be overridden by the running terminal program
    // via tcsetattr, so they aren't totally safe to hardcode like
    // this. In practice, I haven't seen modern programs change these
    // and its a much bigger architectural change to pass these through
    // so for now they're hardcoded.
    0x03, // VINTR (Ctrl+C)
    0x1C, // VQUIT (Ctrl+\)
    0x15, // VKILL (Ctrl+U)
    0x1A, // VSUSP (Ctrl+Z)
    0x11, // VSTART (Ctrl+Q)
    0x13, // VSTOP (Ctrl+S)
    0x17, // VWERASE (Ctrl+W)
    0x16, // VLNEXT (Ctrl+V)
    0x12, // VREPRINT (Ctrl+R)
    0x0F, // VDISCARD (Ctrl+O)
];

/// Encode immutable (const) data for pasting. The resulting value can be
/// written to the pty to perform a paste of the input data.
///
/// This returns `Error::MutableRequired` if the data needs modification
/// (strip bytes or newline replacement); the caller can then make a mutable
/// copy and use `encode_mut`. This calling style allows initially const data
/// to be passed in without an allocation, since it is rare in normal use
/// cases that the data will need to be modified.
///
/// The data is returned as a set of slices to limit allocations. The caller
/// can combine the slices into a single buffer if desired.
///
/// WARNING: The input data is not checked for safety. See the `is_safe`
/// function to check if the data is safe to paste.
pub fn encode(data: &[u8], opts: Options) -> Result<[&[u8]; 3], Error> {
    let mut result: [&[u8]; 3] = [b"", data, b""];

    if data.iter().any(|b| STRIP.contains(b)) {
        return Err(Error::MutableRequired);
    }

    // Bracketed paste mode (mode 2004) wraps pasted data in
    // fenceposts so that the terminal can ignore things like newlines.
    if opts.bracketed {
        result[0] = b"\x1b[200~";
        result[2] = b"\x1b[201~";
        return Ok(result);
    }

    // Non-bracketed. We have to replace newline with `\r`.
    if data.contains(&b'\n') {
        return Err(Error::MutableRequired);
    }

    Ok(result)
}

/// Like `encode` but modifies the data in place, so it can't fail.
pub fn encode_mut(data: &mut [u8], opts: Options) -> [&[u8]; 3] {
    // If we have any of the strip values, then we need to replace them
    // with spaces. This is what xterm does and it does it regardless
    // of bracketed paste mode. This is a security measure to prevent pastes
    // from containing bytes that could be used to inject commands.
    for b in data.iter_mut() {
        if STRIP.contains(b) {
            *b = b' ';
        }
    }

    let data: &[u8] = if opts.bracketed {
        data
    } else {
        // Non-bracketed. We have to replace newline with `\r`. This matches
        // the behavior of xterm and other terminals. For `\r\n` this will
        // result in `\r\r` which does match xterm.
        for b in data.iter_mut() {
            if *b == b'\n' {
                *b = b'\r';
            }
        }
        data
    };

    let mut result: [&[u8]; 3] = [b"", data, b""];
    if opts.bracketed {
        result[0] = b"\x1b[200~";
        result[2] = b"\x1b[201~";
    }
    result
}

/// Returns true if the data looks safe to paste. Data is considered
/// unsafe if it contains any of the following:
///
/// - `\n`: Newlines can be used to inject commands.
/// - `\x1b[201~`: This is the end of a bracketed paste. This cane be used
///   to exit a bracketed paste and inject commands.
///
/// We consider any scenario unsafe regardless of current terminal state.
/// For example, even if bracketed paste mode is not active, we still
/// consider `\x1b[201~` unsafe. The existence of these types of bytes
/// should raise suspicion that the producer of the paste data is
/// acting strangely.
pub fn is_safe(data: &[u8]) -> bool {
    !data.contains(&b'\n') && !data.windows(6).any(|w| w == b"\x1b[201~")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn is_safe_test() {
        assert!(is_safe(b"hello"));
        assert!(!is_safe(b"hello\n"));
        assert!(!is_safe(b"hello\nworld"));
        assert!(!is_safe(b"he\x1b[201~llo"));
    }

    #[test]
    fn encode_bracketed() {
        let r = encode(b"hello", Options { bracketed: true }).unwrap();
        assert_eq!(r[0], b"\x1b[200~");
        assert_eq!(r[1], b"hello");
        assert_eq!(r[2], b"\x1b[201~");
    }

    #[test]
    fn encode_unbracketed_no_newlines() {
        let r = encode(b"hello", Options { bracketed: false }).unwrap();
        assert_eq!(r[0], b"");
        assert_eq!(r[1], b"hello");
        assert_eq!(r[2], b"");
    }

    #[test]
    fn encode_unbracketed_newlines_const() {
        assert_eq!(
            encode(b"hello\nworld", Options { bracketed: false }),
            Err(Error::MutableRequired)
        );
    }

    #[test]
    fn encode_unbracketed_newlines() {
        let mut data = b"hello\nworld".to_vec();
        let r = encode_mut(&mut data, Options { bracketed: false });
        assert_eq!(r[0], b"");
        assert_eq!(r[1], b"hello\rworld");
        assert_eq!(r[2], b"");
    }

    #[test]
    fn encode_unbracketed_windows_stye_newline() {
        let mut data = b"hello\r\nworld".to_vec();
        let r = encode_mut(&mut data, Options { bracketed: false });
        assert_eq!(r[0], b"");
        assert_eq!(r[1], b"hello\r\rworld");
        assert_eq!(r[2], b"");
    }

    #[test]
    fn encode_strip_unsafe_bytes_const() {
        assert_eq!(
            encode(b"hello\x00world", Options { bracketed: true }),
            Err(Error::MutableRequired)
        );
    }

    #[test]
    fn encode_strip_unsafe_bytes_mutable_bracketed() {
        let mut data = b"hel\x1blo\x00world".to_vec();
        let r = encode_mut(&mut data, Options { bracketed: true });
        assert_eq!(r[0], b"\x1b[200~");
        assert_eq!(r[1], b"hel lo world");
        assert_eq!(r[2], b"\x1b[201~");
    }

    #[test]
    fn encode_strip_unsafe_bytes_mutable_unbracketed() {
        let mut data = b"hel\x03lo".to_vec();
        let r = encode_mut(&mut data, Options { bracketed: false });
        assert_eq!(r[0], b"");
        assert_eq!(r[1], b"hel lo");
        assert_eq!(r[2], b"");
    }

    #[test]
    fn encode_strip_multiple_unsafe_bytes() {
        let mut data = b"\x00\x08\x7f".to_vec();
        let r = encode_mut(&mut data, Options { bracketed: true });
        assert_eq!(r[1], b"   ");
    }
}
