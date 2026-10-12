//! `ghostty_paste_is_safe` (ZIG/src/terminal/c/paste.zig).
#![allow(unsafe_code)]

/// Whether pasting `data` is safe (no newline, no bracketed-paste end).
///
/// # Safety
/// `data` must be null or point to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_paste_is_safe(data: *const u8, len: usize) -> bool {
    let slice: &[u8] = if data.is_null() {
        &[]
    } else {
        // SAFETY: caller guarantees `data` points to `len` readable bytes.
        unsafe { std::slice::from_raw_parts(data, len) }
    };
    ghostty_input::paste::is_safe(slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn safe(s: &[u8]) -> bool {
        // SAFETY: valid slice.
        unsafe { ghostty_paste_is_safe(s.as_ptr(), s.len()) }
    }

    #[test]
    fn is_safe_with_safe_data() {
        assert!(safe(b"hello world"));
    }

    #[test]
    fn is_safe_with_newline() {
        assert!(!safe(b"hello\nworld"));
    }

    #[test]
    fn is_safe_with_bracketed_paste_end() {
        assert!(!safe(b"hello\x1b[201~world"));
    }

    #[test]
    fn is_safe_with_empty_data() {
        assert!(safe(b""));
    }

    #[test]
    fn is_safe_with_null_empty_data() {
        // SAFETY: null with len 0 is explicitly allowed.
        assert!(unsafe { ghostty_paste_is_safe(std::ptr::null(), 0) });
    }
}
