//! `Helpers/Extensions/NSApplication+Extension.swift`.

use std::{cell::RefCell, collections::HashMap};

use objc2_app_kit::{
    NSApplication,
    NSApplicationPresentationOptions,
    NSRunningApplication,
    NSWorkspace,
};
use objc2_foundation::NSBundle;

thread_local! {
    /// `NSApplication.presentationOptionCounts`, keyed by the option's raw value.
    static PRESENTATION_OPTION_COUNTS: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
}

/// Pure ref counting behind `acquire/releasePresentationOption`.
#[derive(Debug, Default)]
pub struct OptionCounts(HashMap<usize, u64>);

/// What the caller must do to the app's presentation options.
#[derive(Debug, PartialEq, Eq)]
pub enum OptionChange {
    Insert,
    Remove,
    Nothing,
}

impl OptionCounts {
    /// `acquirePresentationOption`: always (re)inserts the option.
    pub fn acquire(&mut self, option: usize) -> OptionChange {
        *self.0.entry(option).or_insert(0) += 1;
        OptionChange::Insert
    }

    /// `releasePresentationOption`: removes the option when the last
    /// reference is released.
    pub fn release(&mut self, option: usize) -> OptionChange {
        let Some(&value) = self.0.get(&option) else {
            return OptionChange::Nothing;
        };
        if value == 0 {
            return OptionChange::Nothing;
        }
        if value == 1 {
            self.0.remove(&option);
            OptionChange::Remove
        } else {
            self.0.insert(option, value - 1);
            OptionChange::Nothing
        }
    }
}

pub trait NSApplicationExt {
    /// Add a presentation option to the application and maintain a reference
    /// count so that an equal number of releases is required to disable it.
    /// This is useful so that multiple classes can affect global app state
    /// without overriding others.
    fn acquire_presentation_option(&self, option: NSApplicationPresentationOptions);

    /// See `acquire_presentation_option`.
    fn release_presentation_option(&self, option: NSApplicationPresentationOptions);

    /// True if the application is frontmost. This isn't exactly the same as
    /// isActive because an app can be active but not be frontmost if the
    /// window with activity is an NSPanel.
    fn is_frontmost(&self) -> bool;
}

impl NSApplicationExt for NSApplication {
    fn acquire_presentation_option(&self, option: NSApplicationPresentationOptions) {
        PRESENTATION_OPTION_COUNTS.with(|c| {
            let mut counts = OptionCounts(std::mem::take(&mut *c.borrow_mut()));
            let change = counts.acquire(option.0);
            *c.borrow_mut() = counts.0;
            debug_assert_eq!(change, OptionChange::Insert);
        });
        self.setPresentationOptions(self.presentationOptions() | option);
    }

    fn release_presentation_option(&self, option: NSApplicationPresentationOptions) {
        let change = PRESENTATION_OPTION_COUNTS.with(|c| {
            let mut counts = OptionCounts(std::mem::take(&mut *c.borrow_mut()));
            let change = counts.release(option.0);
            *c.borrow_mut() = counts.0;
            change
        });
        if change == OptionChange::Remove {
            self.setPresentationOptions(self.presentationOptions() - option);
        }
    }

    fn is_frontmost(&self) -> bool {
        let front: Option<_> = NSWorkspace::sharedWorkspace().frontmostApplication();
        let front_id =
            front.and_then(|a: objc2::rc::Retained<NSRunningApplication>| a.bundleIdentifier());
        let main_id = NSBundle::mainBundle().bundleIdentifier();
        front_id.map(|s| s.to_string()) == main_id.map(|s| s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_counts() {
        let mut c = OptionCounts::default();
        assert_eq!(c.release(1), OptionChange::Nothing);
        assert_eq!(c.acquire(1), OptionChange::Insert);
        assert_eq!(c.acquire(1), OptionChange::Insert);
        assert_eq!(c.release(1), OptionChange::Nothing);
        assert_eq!(c.release(1), OptionChange::Remove);
        assert_eq!(c.release(1), OptionChange::Nothing);
    }
}
