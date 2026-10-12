//! `Helpers/Extensions/Array+Extension.swift`.

use std::ffi::CString;

/// `extension Array`.
pub trait ArrayExt<T> {
    /// `subscript(safe:)`.
    fn get_safe(&self, index: isize) -> Option<&T>;
    /// Returns the index before i, with wraparound. Assumes i is a valid index.
    fn index_wrapping_before(&self, i: usize) -> usize;
    /// Returns the index after i, with wraparound. Assumes i is a valid index.
    fn index_wrapping_after(&self, i: usize) -> usize;
}

impl<T> ArrayExt<T> for [T] {
    fn get_safe(&self, index: isize) -> Option<&T> {
        usize::try_from(index).ok().and_then(|i| self.get(i))
    }

    fn index_wrapping_before(&self, i: usize) -> usize {
        if i == 0 {
            return self.len() - 1;
        }
        i - 1
    }

    fn index_wrapping_after(&self, i: usize) -> usize {
        if i == self.len() - 1 {
            return 0;
        }
        i + 1
    }
}

/// `Array<String>.withCStrings`: NUL terminated copies of the strings (the
/// returned `CString`s own the memory the pointers of `as_ptr()` refer to).
/// A string containing an interior NUL is truncated at it, as a C consumer
/// would read it.
#[must_use]
pub fn to_c_strings<S: AsRef<str>>(strings: &[S]) -> Vec<CString> {
    strings
        .iter()
        .map(|s| {
            let bytes = s.as_ref().as_bytes();
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            CString::new(&bytes[..end]).unwrap_or_default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_subscript() {
        let a = [1, 2, 3];
        assert_eq!(a.get_safe(0), Some(&1));
        assert_eq!(a.get_safe(3), None);
        assert_eq!(a.get_safe(-1), None);
    }

    #[test]
    fn wrapping_indices() {
        let a = [1, 2, 3];
        assert_eq!(a.index_wrapping_before(0), 2);
        assert_eq!(a.index_wrapping_before(2), 1);
        assert_eq!(a.index_wrapping_after(2), 0);
        assert_eq!(a.index_wrapping_after(0), 1);
    }

    #[test]
    fn c_strings() {
        let c = to_c_strings(&["a", "b\0c", ""]);
        assert_eq!(c[0].to_str().unwrap(), "a");
        assert_eq!(c[1].to_str().unwrap(), "b");
        assert_eq!(c[2].as_bytes().len(), 0);
        assert_eq!(to_c_strings::<&str>(&[]).len(), 0);
    }
}
