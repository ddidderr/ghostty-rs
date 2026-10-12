//! `Helpers/ObjCExceptionCatcher.{h,m}`: wrappers around ObjC calls that may
//! throw `NSException`s, which cannot cross into safe code.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use std::{fmt, panic::AssertUnwindSafe};

use objc2::exception;
use objc2_app_kit::{NSWindow, NSWindowOrderingMode};
use objc2_foundation::NSException;

/// The Swift version reports an `NSError` in domain `Ghostty.ObjCException`
/// (code 1, `NSLocalizedDescriptionKey` = reason, `exception_name`). In Rust
/// that is this struct (same fields).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjCException {
    /// `exception.name` (`"exception_name"` in the NSError's userInfo).
    pub name: String,
    /// `exception.reason` or "Unknown Objective-C exception".
    pub reason: String,
}

impl ObjCException {
    pub const DOMAIN: &'static str = "Ghostty.ObjCException";
    pub const CODE: isize = 1;
}

impl fmt::Display for ObjCException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for ObjCException {}

/// `GhosttyAddTabbedWindowSafely`: `[parent addTabbedWindow:child ordered:]`
/// catching an `NSException`.
///
/// AppKit occasionally throws NSException while adding tabbed windows, in
/// particular when creating tabs from the tab overview page since some macOS
/// update recently in 2025/2026 (unclear).
pub fn add_tabbed_window_safely(
    parent: &NSWindow,
    child: &NSWindow,
    ordered: NSWindowOrderingMode,
) -> Result<(), ObjCException> {
    exception::catch(AssertUnwindSafe(|| {
        parent.addTabbedWindow_ordered(child, ordered);
    }))
    .map_err(|exception| {
        let ns = exception.and_then(|e| e.downcast::<NSException>().ok());
        ObjCException {
            name: ns
                .as_ref()
                .map(|e| e.name().to_string())
                .unwrap_or_default(),
            reason: ns.and_then(|e| e.reason()).map_or_else(
                || "Unknown Objective-C exception".to_string(),
                |r| r.to_string(),
            ),
        }
    })
}
