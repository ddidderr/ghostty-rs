//! `Helpers/Extensions/FileHandle+Extension.swift`: `FileHandle` as a
//! `TextOutputStream`. Any `io::Write` already has `write_all`; this is the
//! string convenience.

use std::io::Write;

pub trait WriteStringExt: Write {
    /// Write a string to the handle.
    fn write_string(&mut self, string: &str) -> std::io::Result<()> {
        self.write_all(string.as_bytes())
    }
}

impl<W: Write + ?Sized> WriteStringExt for W {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_utf8() {
        let mut v = Vec::new();
        v.write_string("héllo").unwrap();
        assert_eq!(v, "héllo".as_bytes());
    }
}
