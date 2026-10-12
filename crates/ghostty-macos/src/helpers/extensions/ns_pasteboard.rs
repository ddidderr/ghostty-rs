//! `Helpers/Extensions/NSPasteboard+Extension.swift` (+ `NSPasteboardTests`).

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use std::ffi::c_int;

use objc2::{ClassType, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::{NSArray, NSCopying, NSString, NSURL};

use super::uti;
use crate::ghostty::{
    capi::consts::{GHOSTTY_CLIPBOARD_SELECTION, GHOSTTY_CLIPBOARD_STANDARD},
    shell,
};

/// `NSPasteboard.PasteboardType(mimeType:)` (a pasteboard type is an
/// `NSString`). Never fails: unknown MIME types are used directly as the
/// identifier, like the Swift fallback.
#[must_use]
pub fn pasteboard_type_from_mime(mime_type: &str) -> Retained<NSString> {
    // Explicit mappings for common MIME types
    if mime_type == "text/plain" {
        // SAFETY: reading AppKit's immutable string constant.
        #[allow(unsafe_code)]
        return unsafe { NSPasteboardTypeString }.copy();
    }

    // Try to get UTType from MIME type; otherwise use the MIME type directly.
    match uti::identifier_for_mime_type(mime_type) {
        Some(identifier) => NSString::from_str(&identifier),
        None => NSString::from_str(mime_type),
    }
}

thread_local! {
    /// `NSPasteboard.ghosttySelection`.
    static GHOSTTY_SELECTION: Retained<NSPasteboard> =
        NSPasteboard::pasteboardWithName(&NSString::from_str("com.mitchellh.ghostty.selection"));
}

/// The pasteboard to used for Ghostty selection.
#[must_use]
pub fn ghostty_selection() -> Retained<NSPasteboard> {
    GHOSTTY_SELECTION.with(Clone::clone)
}

/// `NSPasteboard.ghostty(_:)`: the pasteboard for the Ghostty enum type.
#[must_use]
pub fn ghostty(clipboard: c_int) -> Option<Retained<NSPasteboard>> {
    match clipboard {
        GHOSTTY_CLIPBOARD_STANDARD => Some(NSPasteboard::generalPasteboard()),
        GHOSTTY_CLIPBOARD_SELECTION => Some(ghostty_selection()),
        _ => None,
    }
}

pub trait NSPasteboardExt {
    /// Gets the contents of the pasteboard as a string following a specific
    /// set of semantics. Does these things in order:
    /// - Tries to get the absolute filesystem path of the file in the
    ///   pasteboard if there is one and ensures the file path is properly escaped.
    /// - Tries to get any string from the pasteboard.
    ///
    /// If all of the above fail, returns None.
    fn get_opinionated_string_contents(&self) -> Option<String>;
}

impl NSPasteboardExt for NSPasteboard {
    #[allow(unsafe_code)]
    fn get_opinionated_string_contents(&self) -> Option<String> {
        let classes = NSArray::from_slice(&[NSURL::class()]);
        // SAFETY: `readObjectsForClasses:options:` takes an array of classes.
        let objects: Option<Retained<NSArray<AnyObject>>> = unsafe {
            msg_send![self, readObjectsForClasses: &*classes, options: std::ptr::null::<AnyObject>()]
        };
        if let Some(objects) = objects {
            let urls: Vec<String> = objects
                .iter()
                .filter_map(|o| o.downcast::<NSURL>().ok())
                .map(|url| {
                    if url.isFileURL() {
                        url.path()
                            .map(|p| shell::escape(&p.to_string()))
                            .unwrap_or_default()
                    } else {
                        url.absoluteString()
                            .map(|s| s.to_string())
                            .unwrap_or_default()
                    }
                })
                .collect();
            if !urls.is_empty() {
                return Some(urls.join(" "));
            }
        }

        // SAFETY: reading AppKit's immutable string constant.
        self.stringForType(unsafe { NSPasteboardTypeString })
            .map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use objc2_app_kit::{NSPasteboardTypeHTML, NSPasteboardTypePNG};

    use super::*;

    /// Test text/plain MIME type converts to .string
    #[test]
    fn text_plain_mime_type() {
        // SAFETY: reading AppKit constants.
        #[allow(unsafe_code)]
        let expected = unsafe { NSPasteboardTypeString }.to_string();
        assert_eq!(
            pasteboard_type_from_mime("text/plain").to_string(),
            expected
        );
    }

    /// Test text/html MIME type converts to .html
    #[test]
    fn text_html_mime_type() {
        #[allow(unsafe_code)]
        let expected = unsafe { NSPasteboardTypeHTML }.to_string();
        assert_eq!(pasteboard_type_from_mime("text/html").to_string(), expected);
    }

    /// Test image/png MIME type
    #[test]
    fn image_png_mime_type() {
        #[allow(unsafe_code)]
        let expected = unsafe { NSPasteboardTypePNG }.to_string();
        assert_eq!(pasteboard_type_from_mime("image/png").to_string(), expected);
    }
}
