//! `Helpers/CodableBridge.swift`: a wrapper that allows a Swift `Codable` to
//! implement `NSSecureCoding`.
//!
//! Swift archives the value with `NSKeyedArchiver.encodeEncodable` into a
//! nested archive stored as `NSData` under the key `"data"` (inner key
//! `"value"`). Rust has no `Codable`; the equivalent is the [`Codable`] trait
//! (a byte encoding the type chooses, e.g. a hand-written plist/JSON), stored
//! under the same keys in the outer coder. Archives are therefore not
//! byte-compatible with the Swift app's (documented deviation); the bridge is
//! used for window-restoration state only.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use objc2::{ClassType, rc::Retained};
use objc2_foundation::{NSCoder, NSData, NSString};

/// The Rust equivalent of Swift's `Codable`.
pub trait Codable: Sized {
    fn encode(&self) -> Vec<u8>;
    fn decode(bytes: &[u8]) -> Option<Self>;
}

/// The key under which the encoded bytes are stored.
pub const DATA_KEY: &str = "data";

/// `CodableBridge.supportsSecureCoding`.
pub const SUPPORTS_SECURE_CODING: bool = true;

/// `CodableBridge(coder:)` failable decode.
pub fn decode_from_coder<T: Codable>(coder: &NSCoder) -> Option<T> {
    // SAFETY: the class passed matches the expected NSData.
    let data: Retained<NSData> =
        unsafe { coder.decodeObjectOfClass_forKey(NSData::class(), &NSString::from_str(DATA_KEY)) }
            .and_then(|o| o.downcast::<NSData>().ok())?;
    T::decode(&data.to_vec())
}

/// `CodableBridge.encode(with:)`.
pub fn encode_to_coder<T: Codable>(value: &T, coder: &NSCoder) {
    let data = NSData::with_bytes(&value.encode());
    // SAFETY: NSData is a valid NSCoding object for the key.
    #[allow(unsafe_code)]
    unsafe {
        coder.encodeObject_forKey(Some(&data), &NSString::from_str(DATA_KEY));
    }
}
