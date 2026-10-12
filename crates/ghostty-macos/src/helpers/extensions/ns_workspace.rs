//! `Helpers/Extensions/NSWorkspace+Extension.swift`.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code)]

use std::{ffi::c_void, ptr::NonNull};

use objc2::rc::Retained;
use objc2_app_kit::NSWorkspace;
use objc2_core_foundation::CFString;
use objc2_foundation::NSURL;

use super::uti;

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    /// Returns a +1 retained CFURL (toll-free bridged with NSURL) or null.
    fn LSCopyDefaultApplicationURLForContentType(
        content_type: &CFString,
        role_mask: u32,
        out_error: *mut c_void,
    ) -> *mut c_void;
}

/// `LSRolesMask.all`.
const LS_ROLES_ALL: u32 = 0xFFFF_FFFF;

pub trait NSWorkspaceExt {
    /// The URL of the default text editor application, if any.
    fn default_text_editor(&self) -> Option<Retained<NSURL>>;

    /// The URL of the default terminal (Unix Executable) application, if any.
    fn default_terminal(&self) -> Option<Retained<NSURL>>;

    /// The URL of the default application for opening files with the
    /// specified content type (UTI identifier).
    fn default_application_url_for_content_type(
        &self,
        content_type: &str,
    ) -> Option<Retained<NSURL>>;

    /// The URL of the default application for opening files with the
    /// specified file extension.
    fn default_application_url_for_extension(&self, ext: &str) -> Option<Retained<NSURL>>;
}

impl NSWorkspaceExt for NSWorkspace {
    fn default_text_editor(&self) -> Option<Retained<NSURL>> {
        self.default_application_url_for_content_type(uti::PLAIN_TEXT)
    }

    fn default_terminal(&self) -> Option<Retained<NSURL>> {
        self.default_application_url_for_content_type(uti::UNIX_EXECUTABLE)
    }

    #[allow(unsafe_code)]
    fn default_application_url_for_content_type(
        &self,
        content_type: &str,
    ) -> Option<Retained<NSURL>> {
        let content_type = CFString::from_str(content_type);
        // SAFETY: valid CFString; a null error out-parameter is allowed.
        let raw = unsafe {
            LSCopyDefaultApplicationURLForContentType(
                &content_type,
                LS_ROLES_ALL,
                std::ptr::null_mut(),
            )
        };
        let raw = NonNull::new(raw)?;
        // SAFETY: the +1 retained CFURL is toll-free bridged to NSURL and we
        // adopt that reference.
        unsafe { Retained::from_raw(raw.as_ptr().cast::<NSURL>()) }
    }

    fn default_application_url_for_extension(&self, ext: &str) -> Option<Retained<NSURL>> {
        let uti = uti::identifier_for_extension(ext)?;
        self.default_application_url_for_content_type(&uti)
    }
}
