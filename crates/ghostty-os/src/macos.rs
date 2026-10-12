//! macOS-specific helpers. Port of `os/macos.zig`.
//!
//! The platform-independent items ([`QosClass`], [`join_dir`]) compile
//! everywhere so they can be unit tested on any host; everything that talks
//! to Foundation / libpthread is gated on `target_vendor = "apple"`.

use std::path::{Path, PathBuf};

/// Quality-of-service classes (`pthread_set_qos_class_self_np`).
///
/// <https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/PrioritizeWorkAtTheTaskLevel.html>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QosClass {
    UserInteractive = 0x21,
    UserInitiated = 0x19,
    Default = 0x15,
    Utility = 0x11,
    Background = 0x09,
    Unspecified = 0x00,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SetQosClassError {
    /// The thread can't have its QoS class changed usually because a
    /// different pthread API was called that makes it an invalid target.
    #[error("thread incompatible")]
    ThreadIncompatible,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DirError {
    #[error("apple api failed")]
    AppleApiFailed,
}

/// `NSSearchPathDirectory` values we use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum SearchPathDirectory {
    CachesDirectory = 13,
    ApplicationSupportDirectory = 14,
}

/// Join `base_dir` with the given sub paths like `std.fs.path.join`
/// (`commonDir`'s tail): empty components are skipped.
pub fn join_dir(base_dir: &str, sub_paths: &[&str]) -> PathBuf {
    let mut parts = vec![base_dir];
    parts.extend_from_slice(sub_paths);
    PathBuf::from(crate::path_join(&parts))
}

/// Return the path to the application support directory for Ghostty
/// with the given sub path joined.
#[cfg(target_vendor = "apple")]
pub fn app_support_dir(sub_path: &str) -> Result<PathBuf, DirError> {
    common_dir(
        SearchPathDirectory::ApplicationSupportDirectory,
        &[crate::i18n::BUNDLE_ID, sub_path],
    )
}

/// Return the path to the system cache directory with the given sub path
/// joined.
#[cfg(target_vendor = "apple")]
pub fn cache_dir(sub_path: &str) -> Result<PathBuf, DirError> {
    common_dir(
        SearchPathDirectory::CachesDirectory,
        &[crate::i18n::BUNDLE_ID, sub_path],
    )
}

#[cfg(target_vendor = "apple")]
fn common_dir(directory: SearchPathDirectory, sub_paths: &[&str]) -> Result<PathBuf, DirError> {
    use objc2_foundation::{NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask};

    let manager = NSFileManager::defaultManager();
    let url = manager
        .URLForDirectory_inDomain_appropriateForURL_create_error(
            NSSearchPathDirectory(directory as usize),
            NSSearchPathDomainMask::UserDomainMask,
            None,
            true,
        )
        .map_err(|_| DirError::AppleApiFailed)?;
    let path = url.path().ok_or(DirError::AppleApiFailed)?;
    Ok(join_dir(&path.to_string(), sub_paths))
}

/// Verifies that the running macOS system version is at least the given
/// version.
#[cfg(target_vendor = "apple")]
pub fn is_at_least_version(major: isize, minor: isize, patch: isize) -> bool {
    use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};

    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: major,
        minorVersion: minor,
        patchVersion: patch,
    })
}

/// Set the QoS class of the running thread.
///
/// <https://developer.apple.com/documentation/apple-silicon/tuning-your-code-s-performance-for-apple-silicon?preferredLanguage=occ>
#[cfg(target_vendor = "apple")]
#[allow(unsafe_code)]
pub fn set_qos_class(class: QosClass) -> Result<(), SetQosClassError> {
    // SAFETY: plain libpthread call with a valid QoS value and no pointers.
    let rc = unsafe { libc::pthread_set_qos_class_self_np(class.to_libc(), 0) };
    match rc {
        0 => Ok(()),
        libc::EPERM => Err(SetQosClassError::ThreadIncompatible),
        // EPERM is the only known error that can happen based on the man
        // pages for pthread_set_qos_class_self_np.
        e => panic!("unexpected pthread_set_qos_class_self_np error: {e}"),
    }
}

#[cfg(target_vendor = "apple")]
impl QosClass {
    fn to_libc(self) -> libc::qos_class_t {
        match self {
            QosClass::UserInteractive => libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE,
            QosClass::UserInitiated => libc::qos_class_t::QOS_CLASS_USER_INITIATED,
            QosClass::Default => libc::qos_class_t::QOS_CLASS_DEFAULT,
            QosClass::Utility => libc::qos_class_t::QOS_CLASS_UTILITY,
            QosClass::Background => libc::qos_class_t::QOS_CLASS_BACKGROUND,
            QosClass::Unspecified => libc::qos_class_t::QOS_CLASS_UNSPECIFIED,
        }
    }
}

/// Name the *current* thread (`pthread_setname_np` on Darwin can only name
/// the calling thread).
#[cfg(target_vendor = "apple")]
#[allow(unsafe_code)]
pub fn pthread_setname_np(name: &std::ffi::CStr) {
    // SAFETY: `name` is a valid NUL-terminated string for the duration of
    // the call; Darwin copies it.
    unsafe {
        libc::pthread_setname_np(name.as_ptr());
    }
}

/// Convenience for non-Darwin callers/tests: is `p` below a `Caches`
/// directory.
pub fn is_cache_path(p: &Path) -> bool {
    p.components().any(|c| c.as_os_str() == "Caches")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qos_class_values() {
        assert_eq!(QosClass::UserInteractive as u32, 0x21);
        assert_eq!(QosClass::UserInitiated as u32, 0x19);
        assert_eq!(QosClass::Default as u32, 0x15);
        assert_eq!(QosClass::Utility as u32, 0x11);
        assert_eq!(QosClass::Background as u32, 0x09);
        assert_eq!(QosClass::Unspecified as u32, 0);
    }

    #[test]
    fn search_path_values() {
        assert_eq!(SearchPathDirectory::CachesDirectory as u64, 13);
        assert_eq!(SearchPathDirectory::ApplicationSupportDirectory as u64, 14);
    }

    #[test]
    fn cache_dir_paths() {
        let id = crate::i18n::BUNDLE_ID;
        // Test base path.
        let p = join_dir("/Users/x/Library/Caches", &[id, ""]);
        assert!(is_cache_path(&p));
        assert!(p.to_string_lossy().contains(id));
        // Test with subdir.
        let p = join_dir("/Users/x/Library/Caches", &[id, "test"]);
        assert!(is_cache_path(&p));
        assert!(p.to_string_lossy().contains(&format!("{id}/test")));
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn cache_dir_real() {
        let p = cache_dir("test").unwrap();
        assert!(is_cache_path(&p));
    }
}
