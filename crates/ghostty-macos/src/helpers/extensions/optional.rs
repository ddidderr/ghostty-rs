//! `Helpers/Extensions/Optional+Extension.swift`.

use std::ffi::{CString, c_char};

/// `Optional<String>.withCString`: run `body` with a NUL terminated pointer,
/// or null for `None`.
pub fn with_c_string<T>(s: Option<&str>, body: impl FnOnce(*const c_char) -> T) -> T {
    match s {
        Some(s) => {
            let end = s.bytes().position(|b| b == 0).unwrap_or(s.len());
            let c = CString::new(&s.as_bytes()[..end]).unwrap_or_default();
            body(c.as_ptr())
        }
        None => body(std::ptr::null()),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::CStr;

    use super::*;

    #[test]
    fn some_and_none() {
        let got = with_c_string(Some("hi"), |p| {
            // SAFETY: p is a valid NUL terminated string for the call.
            #[allow(unsafe_code)]
            unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string()
        });
        assert_eq!(got, "hi");
        assert!(with_c_string(None, <*const c_char>::is_null));
    }
}
