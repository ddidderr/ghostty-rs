//! `Helpers/Extensions/UUID+Extension.swift`.

use objc2::{AnyThread, rc::Retained};
use objc2_core_foundation::CFUUID;
use objc2_foundation::{NSString, NSUUID};

/// `UUID(_ cfuuid: CFUUID)`.
#[must_use]
pub fn uuid_from_cfuuid(cfuuid: &CFUUID) -> Option<Retained<NSUUID>> {
    let s = CFUUID::new_string(None, Some(cfuuid))?;
    NSUUID::initWithUUIDString(NSUUID::alloc(), &NSString::from_str(&s.to_string()))
}
