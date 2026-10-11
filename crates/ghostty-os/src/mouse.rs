/// The system-configured double-click interval if its available.
///
/// Only macOS can report this; on Linux this is always `None`.
pub fn click_interval() -> Option<u32> {
    None
}
