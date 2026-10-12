//! The reference counting of `Helpers/Cursor.swift`'s `Cursor` class, split
//! from the NSCursor calls so it is testable everywhere.

/// Tracks how many times the cursor was hidden.
#[derive(Debug, Default)]
pub struct CursorCounter {
    counter: u64,
}

impl CursorCounter {
    #[must_use]
    pub const fn new() -> Self {
        Self { counter: 0 }
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.counter == 0
    }

    #[must_use]
    pub fn count(&self) -> u64 {
        self.counter
    }

    pub fn hide(&mut self) {
        self.counter += 1;
    }

    /// Returns true if the cursor was previously hidden.
    pub fn unhide(&mut self) -> bool {
        if self.counter > 0 {
            self.counter -= 1;
            return true;
        }
        false
    }

    /// Unhide until visible; returns how many hides were undone.
    pub fn unhide_completely(&mut self) -> u64 {
        let counter = self.counter;
        for _ in 0..counter {
            let was_hidden = self.unhide();
            debug_assert!(was_hidden);
        }
        debug_assert_eq!(self.counter, 0);
        counter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hide_unhide() {
        let mut c = CursorCounter::new();
        assert!(c.is_visible());
        assert!(!c.unhide());
        c.hide();
        c.hide();
        assert!(!c.is_visible());
        assert!(c.unhide());
        assert!(!c.is_visible());
        assert_eq!(c.unhide_completely(), 1);
        assert!(c.is_visible());
    }
}
