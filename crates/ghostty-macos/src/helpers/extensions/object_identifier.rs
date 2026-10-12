//! `Helpers/Extensions/ObjectIdentifier+Extension.swift`.

/// `ObjectIdentifier.hexString`: the identity (object address) in lowercase
/// hex without prefix.
#[must_use]
pub fn hex_string(identity: usize) -> String {
    format!("{identity:x}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex() {
        assert_eq!(super::hex_string(255), "ff");
        assert_eq!(super::hex_string(0), "0");
    }
}
