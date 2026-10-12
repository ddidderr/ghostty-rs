//! Uniform type identifier lookups shared by `NSPasteboard+Extension` and
//! `NSWorkspace+Extension` (Swift uses `UTType(mimeType:)`,
//! `UTType(filenameExtension:)`, `UTType.plainText`, `.unixExecutable`).
//!
//! `UniformTypeIdentifiers.UTType` is a Swift-first API; the CoreServices C
//! functions below give the same identifiers.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use objc2_core_foundation::{CFRetained, CFString};

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    static kUTTagClassFilenameExtension: &'static CFString;
    static kUTTagClassMIMEType: &'static CFString;
    fn UTTypeCreatePreferredIdentifierForTag(
        tag_class: &CFString,
        tag: &CFString,
        conforming_to_uti: Option<&CFString>,
    ) -> *const CFString;
}

/// `UTType.plainText.identifier`.
pub const PLAIN_TEXT: &str = "public.plain-text";
/// `UTType.unixExecutable.identifier`.
pub const UNIX_EXECUTABLE: &str = "public.unix-executable";

#[allow(unsafe_code)]
fn identifier_for_tag(class: &CFString, tag: &str) -> Option<String> {
    let tag = CFString::from_str(tag);
    // SAFETY: both arguments are valid CFStrings; the result is +1 retained
    // or null and adopted by `CFRetained`.
    let raw = unsafe { UTTypeCreatePreferredIdentifierForTag(class, &tag, None) };
    let raw = std::ptr::NonNull::new(raw.cast_mut())?;
    // SAFETY: see above, ownership transferred.
    let s: CFRetained<CFString> = unsafe { CFRetained::from_raw(raw) };
    Some(s.to_string())
}

/// `UTType(filenameExtension:)?.identifier` (unknown extensions yield a
/// `dyn.*` identifier like `UTType` does).
#[allow(unsafe_code)]
#[must_use]
pub fn identifier_for_extension(ext: &str) -> Option<String> {
    // SAFETY: reading an immutable constant exported by CoreServices.
    identifier_for_tag(unsafe { kUTTagClassFilenameExtension }, ext)
}

/// `UTType(mimeType:)?.identifier`. Identifiers CoreServices makes up for
/// undeclared types (`dyn.*`) are reported as `None`, so callers use their
/// "unknown MIME type" fallback.
#[allow(unsafe_code)]
#[must_use]
pub fn identifier_for_mime_type(mime: &str) -> Option<String> {
    // SAFETY: reading an immutable constant exported by CoreServices.
    identifier_for_tag(unsafe { kUTTagClassMIMEType }, mime).filter(|id| !id.starts_with("dyn."))
}
