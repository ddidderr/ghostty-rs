//! `Sources/Features/Custom App Icon` (owner: MA4).
//! MA1 provides only the notification name and the `AppDelegate.updateAppIcon`
//! hook. TODO(MA4): AppIcon, ColorizedGhosttyIcon, DockTilePlugin, UserDefaults+AppIcon.

use crate::helpers::notification_center::Name;

/// `Notification.Name.ghosttyIconDidChange` (posted on the *distributed*
/// notification center).
pub const GHOSTTY_ICON_DID_CHANGE: Name = Name("com.mitchellh.ghostty.iconDidChange");

/// Posts `.ghosttyIconDidChange` on the distributed notification center
/// (`DistributedNotificationCenter.default().postNotificationName(...,
/// deliverImmediately: true)`), which the dock tile plugin observes.
#[cfg(target_vendor = "apple")]
#[allow(unsafe_code)]
pub fn post_icon_did_change() {
    use objc2_foundation::{NSDistributedNotificationCenter, NSString};
    let center = NSDistributedNotificationCenter::defaultCenter();
    // SAFETY: a name string, no object/userInfo.
    unsafe {
        center.postNotificationName_object_userInfo_deliverImmediately(
            &NSString::from_str(GHOSTTY_ICON_DID_CHANGE.raw_value()),
            None,
            None,
            true,
        );
    }
}
