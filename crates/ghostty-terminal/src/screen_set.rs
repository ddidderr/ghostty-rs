//! A ScreenSet holds multiple terminal screens. This is initially created
//! to handle simple primary vs alternate screens, but could be extended
//! in the future to handle N screens.
//!
//! One of the goals of this is to allow lazy initialization of screens
//! as needed. The primary screen is always initialized, but the alternate
//! screen may not be until first used.
use crate::screen::{Error, Options, Screen};

/// The possible keys for screens in the screen set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Primary,
    Alternate,
}

impl Key {
    fn index(self) -> usize {
        match self {
            Key::Primary => 0,
            Key::Alternate => 1,
        }
    }
}

#[derive(Debug)]
pub struct ScreenSet {
    /// The key value of the currently active screen. Useful for simple
    /// comparisons, e.g. "is this screen the primary screen".
    pub active_key: Key,

    /// All screens that are initialized. The active screen is always
    /// initialized.
    all: [Option<Box<Screen>>; 2],
}

impl ScreenSet {
    pub fn new(opts: Options) -> Result<ScreenSet, Error> {
        // We need to initialize our initial primary screen
        let screen = Box::new(Screen::new(opts)?);
        Ok(ScreenSet {
            active_key: Key::Primary,
            all: [Some(screen), None],
        })
    }

    /// The active screen.
    pub fn active(&self) -> &Screen {
        self.all[self.active_key.index()]
            .as_deref()
            .expect("active screen is initialized")
    }

    /// The active screen.
    pub fn active_mut(&mut self) -> &mut Screen {
        self.all[self.active_key.index()]
            .as_deref_mut()
            .expect("active screen is initialized")
    }

    /// Get the screen for the given key, if it is initialized.
    pub fn get(&self, key: Key) -> Option<&Screen> {
        self.all[key.index()].as_deref()
    }

    /// Get the screen for the given key, if it is initialized.
    pub fn get_mut(&mut self, key: Key) -> Option<&mut Screen> {
        self.all[key.index()].as_deref_mut()
    }

    /// Get the screen for the given key, initializing it if necessary.
    pub fn get_init(&mut self, key: Key, opts: Options) -> Result<&mut Screen, Error> {
        let slot = &mut self.all[key.index()];
        if slot.is_none() {
            *slot = Some(Box::new(Screen::new(opts)?));
        }
        Ok(slot.as_deref_mut().expect("just initialized"))
    }

    /// Remove a key from the set. The primary screen cannot be removed (asserted).
    pub fn remove(&mut self, key: Key) {
        assert_ne!(key, Key::Primary);
        self.all[key.index()] = None;
    }

    /// Switch the active screen to the given key. Requires that the
    /// screen is initialized.
    pub fn switch_to(&mut self, key: Key) {
        assert!(self.all[key.index()].is_some());
        self.active_key = key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_set() {
        let mut set = ScreenSet::new(Options::DEFAULT).unwrap();
        assert_eq!(Key::Primary, set.active_key);

        // Initialize a secondary screen
        set.get_init(Key::Alternate, Options::DEFAULT).unwrap();
        set.switch_to(Key::Alternate);
        assert_eq!(Key::Alternate, set.active_key);
    }
}
