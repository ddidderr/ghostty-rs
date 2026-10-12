//! A noop implementation of the keymap interface so that the embedded
//! library can compile on non-macOS platforms. Port of
//! `input/KeymapNoop.zig`.

use crate::Mods;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Translation {
    pub text: String,
    pub composing: bool,
    pub mods: Mods,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Keymap;

impl Keymap {
    #[allow(clippy::unnecessary_wraps)]
    pub fn new() -> Result<Keymap, std::convert::Infallible> {
        Ok(Keymap)
    }

    #[allow(clippy::unnecessary_wraps, clippy::unused_self)]
    pub fn reload(&mut self) -> Result<(), std::convert::Infallible> {
        Ok(())
    }

    #[allow(clippy::unnecessary_wraps, clippy::unused_self)]
    pub fn translate(
        &self,
        _state: &mut State,
        _code: u16,
        _mods: Mods,
    ) -> Result<Translation, std::convert::Infallible> {
        Ok(Translation::default())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn noop_translate_is_empty() {
        let km = Keymap::new().unwrap();
        let t = km.translate(&mut State, 0, Mods::default()).unwrap();
        assert_eq!(t, Translation::default());
    }
}
