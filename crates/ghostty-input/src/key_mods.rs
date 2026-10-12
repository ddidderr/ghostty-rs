//! Key modifiers (ZIG/src/input/key_mods.zig).

use crate::config::OptionAsAlt;

/// Aliases for modifier names.
pub const ALIAS: &[(&str, Mod)] = &[
    ("cmd", Mod::Super),
    ("command", Mod::Super),
    ("opt", Mod::Alt),
    ("option", Mod::Alt),
    ("control", Mod::Ctrl),
];

/// Single modifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mod {
    Shift,
    Ctrl,
    Alt,
    Super,
}

impl Mod {
    fn from_name(s: &str) -> Option<Mod> {
        match s {
            "shift" => Some(Mod::Shift),
            "ctrl" => Some(Mod::Ctrl),
            "alt" => Some(Mod::Alt),
            "super" => Some(Mod::Super),
            _ => None,
        }
    }
}

/// Which side of the keyboard a modifier is on (`Mod.Side`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum ModSide {
    #[default]
    Left = 0,
    Right = 1,
}

impl ModSide {
    fn from_name(s: &str) -> Option<ModSide> {
        match s {
            "left" => Some(ModSide::Left),
            "right" => Some(ModSide::Right),
            _ => None,
        }
    }
}

/// The standard modifier keys only. Does not include the lock keys,
/// only standard bindable keys. (`Mods.Keys`, a packed u4)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Keys {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_: bool,
}

impl Keys {
    pub fn int(self) -> u8 {
        u8::from(self.shift)
            | u8::from(self.ctrl) << 1
            | u8::from(self.alt) << 2
            | u8::from(self.super_) << 3
    }

    pub fn from_int(v: u8) -> Keys {
        Keys {
            shift: v & 1 != 0,
            ctrl: v & 2 != 0,
            alt: v & 4 != 0,
            super_: v & 8 != 0,
        }
    }
}

/// Tracks the side that is active for any given modifier. Note
/// that this doesn't confirm a modifier is pressed; you must check
/// the bool for that in addition to this. (`Mods.Side`, a packed u4)
///
/// Not all platforms support this, check apprt for more info.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Side {
    pub shift: ModSide,
    pub ctrl: ModSide,
    pub alt: ModSide,
    pub super_: ModSide,
}

impl Side {
    pub fn int(self) -> u8 {
        self.shift as u8 | (self.ctrl as u8) << 1 | (self.alt as u8) << 2 | (self.super_ as u8) << 3
    }

    pub fn from_int(v: u8) -> Side {
        let f = |b: u8| {
            if v & b != 0 {
                ModSide::Right
            } else {
                ModSide::Left
            }
        };
        Side {
            shift: f(1),
            ctrl: f(2),
            alt: f(4),
            super_: f(8),
        }
    }
}

/// A bitmask for all key modifiers.
///
/// NOTE: Zig's field `super` is named `super_` here (`super` is a Rust
/// keyword). The packed integer layout (u16) is available via `int()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
    pub sides: Side,
}

impl Mods {
    /// The mask that has all the side bits set.
    pub const SIDE_MASK: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: false,
        super_: false,
        caps_lock: false,
        num_lock: false,
        sides: Side {
            shift: ModSide::Right,
            ctrl: ModSide::Right,
            alt: ModSide::Right,
            super_: ModSide::Right,
        },
    };

    /// Integer value of this struct.
    pub fn int(self) -> u16 {
        u16::from(self.keys().int())
            | u16::from(self.caps_lock) << 4
            | u16::from(self.num_lock) << 5
            | u16::from(self.sides.int()) << 6
    }

    pub fn from_int(v: u16) -> Mods {
        Mods {
            shift: v & 1 != 0,
            ctrl: v & 2 != 0,
            alt: v & 4 != 0,
            super_: v & 8 != 0,
            caps_lock: v & 16 != 0,
            num_lock: v & 32 != 0,
            sides: Side::from_int(((v >> 6) & 0xF) as u8),
        }
    }

    /// Returns true if no modifiers are set.
    pub fn empty(self) -> bool {
        self.int() == 0
    }

    /// Returns true if two mods are equal.
    pub fn equal(self, other: Mods) -> bool {
        self.int() == other.int()
    }

    /// Returns only the keys.
    pub fn keys(self) -> Keys {
        Keys {
            shift: self.shift,
            ctrl: self.ctrl,
            alt: self.alt,
            super_: self.super_,
        }
    }

    /// Return mods that are only relevant for bindings.
    pub fn binding(self) -> Mods {
        Mods {
            shift: self.shift,
            ctrl: self.ctrl,
            alt: self.alt,
            super_: self.super_,
            ..Mods::default()
        }
    }

    /// Perform `self &~ other` to remove the other mods from self.
    pub fn unset(self, other: Mods) -> Mods {
        Mods::from_int(self.int() & !other.int())
    }

    /// Returns the mods without locks set.
    pub fn without_locks(self) -> Mods {
        let mut copy = self;
        copy.caps_lock = false;
        copy.num_lock = false;
        copy
    }

    /// Return the mods to use for key translation. This handles settings
    /// like macos-option-as-alt. The translation mods should be used for
    /// translation but never sent back in for the key callback.
    pub fn translation(self, option_as_alt: OptionAsAlt) -> Mods {
        let mut result = self;

        // macos-option-as-alt for darwin
        if crate::platform::is_darwin() {
            // Alt has to be set only on the correct side
            let applies = match option_as_alt {
                OptionAsAlt::False => false,
                OptionAsAlt::True => true,
                OptionAsAlt::Left => self.sides.alt != ModSide::Right,
                OptionAsAlt::Right => self.sides.alt != ModSide::Left,
            };

            // Unset alt
            if applies {
                result.alt = false;
            }
        }

        result
    }

    /// Checks to see if super is on (MacOS) or ctrl.
    pub fn ctrl_or_super(self) -> bool {
        if crate::platform::is_darwin() {
            return self.super_;
        }
        self.ctrl
    }
}

/// Modifier remapping. See `key-remap` in Config.zig for detailed docs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemapSet {
    /// Available mappings (insertion ordered, like an array hash map).
    pub map: Vec<(Mods, Mods)>,

    /// The mask of remapped modifiers that can be used to quickly
    /// check if some input mods need remapping.
    pub mask: Mask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("missing assignment")]
    MissingAssignment,
    #[error("invalid modifier")]
    InvalidMod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseCliError {
    #[error("invalid value")]
    InvalidValue,
}

impl RemapSet {
    pub const EMPTY: RemapSet = RemapSet {
        map: Vec::new(),
        mask: Mask {
            keys: Keys {
                shift: false,
                ctrl: false,
                alt: false,
                super_: false,
            },
            left_sides: Side {
                shift: ModSide::Left,
                ctrl: ModSide::Left,
                alt: ModSide::Left,
                super_: ModSide::Left,
            },
            right_sides: Side {
                shift: ModSide::Left,
                ctrl: ModSide::Left,
                alt: ModSide::Left,
                super_: ModSide::Left,
            },
        },
    };

    fn put(&mut self, from: Mods, to: Mods) {
        if let Some(e) = self.map.iter_mut().find(|e| e.0 == from) {
            e.1 = to;
        } else {
            self.map.push((from, to));
        }
    }

    /// Parse from CLI input. Required by Config.
    pub fn parse_cli(&mut self, input: Option<&str>) -> Result<(), ParseCliError> {
        let value = input.unwrap_or("");

        // Empty value resets the set
        if value.is_empty() {
            self.map.clear();
            self.mask = Mask::default();
            return Ok(());
        }

        self.parse(value).map_err(|_| ParseCliError::InvalidValue)
    }

    /// Parse a modifier remap and add it to the set.
    pub fn parse(&mut self, input: &str) -> Result<(), ParseError> {
        // Find the assignment point ('=')
        let eql_idx = input.find('=').ok_or(ParseError::MissingAssignment)?;

        // The to side defaults to "left" if no explicit side is given.
        // This is because this is the default unsided value provided by
        // the apprts in the current Mods layout.
        let to = {
            let raw = parse_mod(&input[eql_idx + 1..])?;
            init_mods(raw.0, raw.1.unwrap_or(ModSide::Left))
        };

        // The from side, if sided, is easy and we put it directly into
        // the map.
        let from_raw = parse_mod(&input[..eql_idx])?;
        if let Some(from_side) = from_raw.1 {
            let from = init_mods(from_raw.0, from_side);
            self.put(from, to);
            self.mask.update(from);
            return Ok(());
        }

        // We need to do some combinatorial explosion here for unsided
        // from in order to assign all possible sides.
        let from_left = init_mods(from_raw.0, ModSide::Left);
        let from_right = init_mods(from_raw.0, ModSide::Right);
        self.put(from_left, to);
        self.put(from_right, to);
        self.mask.update(from_left);
        self.mask.update(from_right);
        Ok(())
    }

    /// Must be called prior to any remappings so that the mapping
    /// is sorted properly. Otherwise, you will get invalid results.
    pub fn finalize(&mut self) {
        // Mods with any right sides prioritize. This mirrors the Zig
        // comparator (which ignores `b`) driven by an insertion sort.
        let side_mask = Mods::SIDE_MASK.int();
        let less = |a: &Mods| a.int() & side_mask != 0;
        for i in 1..self.map.len() {
            let mut j = i;
            while j > 0 && less(&self.map[j].0) {
                self.map.swap(j, j - 1);
                j -= 1;
            }
        }
    }

    /// Compare if two RemapSets are equal. Required by Config.
    pub fn equal(&self, other: &RemapSet) -> bool {
        if self.map.len() != other.map.len() {
            return false;
        }
        self.map.iter().all(|(k, v)| {
            other
                .map
                .iter()
                .find(|(ok, _)| ok == k)
                .is_some_and(|(_, ov)| v.equal(*ov))
        })
    }

    /// Used by Formatter. Required by Config. Returns the values of each
    /// `key-remap = ...` line to print. An empty vec means the formatter
    /// should emit a single empty entry (`key-remap = `).
    pub fn format_entries(&self) -> Vec<String> {
        self.map
            .iter()
            .map(|(from, to)| {
                let mut s = String::new();
                format_mod(&mut s, *from);
                s.push('=');
                format_mod(&mut s, *to);
                s
            })
            .collect()
    }

    /// Returns true if the given mods need remapping.
    pub fn is_remapped(&self, mods: Mods) -> bool {
        self.mask.matches(mods)
    }

    /// Apply a remap to the given mods.
    pub fn apply(&self, mods: Mods) -> Mods {
        if !self.is_remapped(mods) {
            return mods;
        }

        let mods_binding = mods.keys().int();
        let mods_sides = mods.sides.int();

        for (from, to) in &self.map {
            let from_binding = from.keys().int();
            if mods_binding & from_binding != from_binding {
                continue;
            }
            let from_sides = from.sides.int();
            if (mods_sides ^ from_sides) & from_binding != 0 {
                continue;
            }

            let mut mods_int = mods.int();
            mods_int &= !from.int();
            mods_int |= to.int();
            return Mods::from_int(mods_int);
        }

        unreachable!("is_remapped true but no mapping matched")
    }
}

fn format_mod(out: &mut String, mods: Mods) {
    // Check which mod is set and format it with optional side prefix
    let list = [
        ("shift", mods.shift, mods.sides.shift),
        ("ctrl", mods.ctrl, mods.sides.ctrl),
        ("alt", mods.alt, mods.sides.alt),
        ("super", mods.super_, mods.sides.super_),
    ];
    for (name, set, side) in list {
        if set {
            if side == ModSide::Right {
                out.push_str("right_");
            } else {
                // Only write left_ if we need to distinguish
                // For now, always write left_ if it's a sided mapping
                out.push_str("left_");
            }
            out.push_str(name);
            return;
        }
    }
}

/// Parses a single mode in a single remapping string. E.g.
/// `ctrl` or `left_shift`.
fn parse_mod(input: &str) -> Result<(Mod, Option<ModSide>), ParseError> {
    let (side_str, mod_str) = match input.find('_') {
        Some(idx) => (&input[..idx], &input[idx + 1..]),
        None => ("", input),
    };

    let m = match Mod::from_name(mod_str) {
        Some(m) => m,
        None => ALIAS
            .iter()
            .find(|p| p.0 == mod_str)
            .map(|p| p.1)
            .ok_or(ParseError::InvalidMod)?,
    };

    let side = if side_str.is_empty() {
        None
    } else {
        Some(ModSide::from_name(side_str).ok_or(ParseError::InvalidMod)?)
    };
    Ok((m, side))
}

fn init_mods(m: Mod, side: ModSide) -> Mods {
    let mut mods = Mods::default();
    match m {
        Mod::Shift => {
            mods.shift = true;
            mods.sides.shift = side;
        }
        Mod::Ctrl => {
            mods.ctrl = true;
            mods.sides.ctrl = side;
        }
        Mod::Alt => {
            mods.alt = true;
            mods.sides.alt = side;
        }
        Mod::Super => {
            mods.super_ = true;
            mods.sides.super_ = side;
        }
    }
    mods
}

/// Tracks which modifier keys and sides have remappings registered.
/// Used as a fast pre-check before doing expensive map lookups.
///
/// The mask uses separate tracking for left and right sides because
/// remappings can be side-specific (e.g., only remap left_ctrl).
///
/// Note: `left_sides` uses inverted logic where 1 means "left is remapped"
/// even though `ModSide::Left = 0`. This allows efficient bitwise matching
/// since we can AND directly with the side bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mask {
    /// Which modifier keys (shift/ctrl/alt/super) have any remapping.
    pub keys: Keys,
    /// Which modifiers have left-side remappings (inverted: 1 = left remapped).
    pub left_sides: Side,
    /// Which modifiers have right-side remappings (1 = right remapped).
    pub right_sides: Side,
}

impl Mask {
    /// Adds a modifier to the mask, marking it as having a remapping.
    pub fn update(&mut self, mods: Mods) {
        let keys_int = mods.keys().int();

        // OR the new keys into our existing keys mask.
        self.keys = Keys::from_int(self.keys.int() | keys_int);

        // Both Keys and Side are u4 with matching bit positions.
        // This lets us use keys_int to select which side bits to update.
        let sides = mods.sides.int();
        let left_int = self.left_sides.int();
        let right_int = self.right_sides.int();

        // Update left_sides: set bit if this key is active AND side is left.
        // Since Left=0, we invert sides so left becomes 1.
        self.left_sides = Side::from_int(left_int | (keys_int & !sides));

        // Update right_sides: set bit if this key is active AND side is right.
        self.right_sides = Side::from_int(right_int | (keys_int & sides));
    }

    /// Returns true if the given mods match any remapping in this mask.
    /// This is a fast check to avoid expensive map lookups when no
    /// remapping could possibly apply.
    ///
    /// Checks both that the modifier key is remapped AND that the
    /// specific side (left/right) being pressed has a remapping.
    pub fn matches(&self, mods: Mods) -> bool {
        // Find which pressed keys have remappings registered.
        let active = mods.keys().int() & self.keys.int();
        if active == 0 {
            return false;
        }

        // Check if the pressed side matches a remapped side.
        let sides = mods.sides.int();
        let left_int = self.left_sides.int();
        let right_int = self.right_sides.int();
        let side_match = (!sides & left_int) | (sides & right_int);

        // Final check: is any active (pressed + remapped) key also side-matched?
        (active & side_match) != 0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn m(f: impl FnOnce(&mut Mods)) -> Mods {
        let mut x = Mods::default();
        f(&mut x);
        x
    }

    // For our own understanding
    #[test]
    fn mods_layout() {
        assert_eq!(Mods::default().int(), 0);
        assert_eq!(
            Mods {
                shift: true,
                ..Default::default()
            }
            .int(),
            1
        );
        assert_eq!(Mods::from_int(Mods::SIDE_MASK.int()), Mods::SIDE_MASK);
    }

    #[test]
    fn translation_macos_option_as_alt() {
        crate::platform::with_darwin(true, || {
            // Unset
            {
                let mods = Mods::default();
                assert_eq!(mods.translation(OptionAsAlt::True), mods);
            }

            // Set
            {
                let mods = m(|x| x.alt = true);
                assert_eq!(Mods::default(), mods.translation(OptionAsAlt::True));
            }

            // Set but disabled
            {
                let mods = m(|x| x.alt = true);
                assert_eq!(mods.translation(OptionAsAlt::False), mods);
            }

            // Set wrong side
            {
                let mods = m(|x| {
                    x.alt = true;
                    x.sides.alt = ModSide::Right;
                });
                assert_eq!(mods.translation(OptionAsAlt::Left), mods);
            }
            {
                let mods = m(|x| {
                    x.alt = true;
                    x.sides.alt = ModSide::Left;
                });
                assert_eq!(mods.translation(OptionAsAlt::Right), mods);
            }

            // Set with other mods
            {
                let mods = m(|x| {
                    x.alt = true;
                    x.shift = true;
                });
                assert_eq!(m(|x| x.shift = true), mods.translation(OptionAsAlt::True));
            }
        });

        // Not darwin: identity.
        crate::platform::with_darwin(false, || {
            let mods = m(|x| x.alt = true);
            assert_eq!(mods.translation(OptionAsAlt::True), mods);
        });
    }

    #[test]
    fn ctrl_or_super_follows_platform() {
        let ctrl = m(|x| x.ctrl = true);
        let sup = m(|x| x.super_ = true);
        crate::platform::with_darwin(true, || {
            assert!(sup.ctrl_or_super());
            assert!(!ctrl.ctrl_or_super());
        });
        crate::platform::with_darwin(false, || {
            assert!(ctrl.ctrl_or_super());
            assert!(!sup.ctrl_or_super());
        });
    }

    fn left(k: &str) -> Mods {
        match k {
            "ctrl" => m(|x| x.ctrl = true),
            "alt" => m(|x| x.alt = true),
            "shift" => m(|x| x.shift = true),
            _ => m(|x| x.super_ = true),
        }
    }

    #[test]
    fn remap_set_unsided_remap_creates_both_left_and_right_mappings() {
        let mut set = RemapSet::default();
        set.parse("ctrl=super").unwrap();
        set.finalize();
        let want = Mods {
            super_: true,
            sides: Side {
                super_: ModSide::Left,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            want,
            set.apply(Mods {
                ctrl: true,
                sides: Side {
                    ctrl: ModSide::Left,
                    ..Default::default()
                },
                ..Default::default()
            })
        );
        assert_eq!(
            want,
            set.apply(Mods {
                ctrl: true,
                sides: Side {
                    ctrl: ModSide::Right,
                    ..Default::default()
                },
                ..Default::default()
            })
        );
    }

    fn sided(name: &str, side: ModSide) -> Mods {
        init_mods(
            match name {
                "shift" => Mod::Shift,
                "ctrl" => Mod::Ctrl,
                "alt" => Mod::Alt,
                _ => Mod::Super,
            },
            side,
        )
    }

    #[test]
    fn remap_set_sided_from_only_maps_that_side() {
        let mut set = RemapSet::default();
        set.parse("left_alt=ctrl").unwrap();
        set.finalize();
        assert_eq!(
            sided("ctrl", ModSide::Left),
            set.apply(sided("alt", ModSide::Left))
        );
        let right_alt = sided("alt", ModSide::Right);
        assert_eq!(right_alt, set.apply(right_alt));
    }

    #[test]
    fn remap_set_sided_to() {
        let mut set = RemapSet::default();
        set.parse("ctrl=right_super").unwrap();
        set.finalize();
        assert_eq!(
            sided("super", ModSide::Right),
            set.apply(sided("ctrl", ModSide::Left))
        );
    }

    #[test]
    fn remap_set_both_sides_specified() {
        let mut set = RemapSet::default();
        set.parse("left_shift=right_ctrl").unwrap();
        set.finalize();
        assert_eq!(
            sided("ctrl", ModSide::Right),
            set.apply(sided("shift", ModSide::Left))
        );
    }

    #[test]
    fn remap_set_multiple_parses_accumulate() {
        let mut set = RemapSet::default();
        set.parse("left_ctrl=super").unwrap();
        set.parse("left_alt=ctrl").unwrap();
        set.finalize();
        assert_eq!(
            sided("super", ModSide::Left),
            set.apply(sided("ctrl", ModSide::Left))
        );
        assert_eq!(
            sided("ctrl", ModSide::Left),
            set.apply(sided("alt", ModSide::Left))
        );
    }

    #[test]
    fn remap_set_error_on_missing_assignment() {
        let mut set = RemapSet::default();
        assert_eq!(set.parse("ctrl"), Err(ParseError::MissingAssignment));
        assert_eq!(set.parse(""), Err(ParseError::MissingAssignment));
    }

    #[test]
    fn remap_set_error_on_invalid_modifier() {
        let mut set = RemapSet::default();
        assert_eq!(set.parse("invalid=ctrl"), Err(ParseError::InvalidMod));
        assert_eq!(set.parse("ctrl=invalid"), Err(ParseError::InvalidMod));
        assert_eq!(set.parse("middle_ctrl=super"), Err(ParseError::InvalidMod));
    }

    #[test]
    fn remap_set_is_remapped_checks_mask() {
        let mut set = RemapSet::default();
        set.parse("ctrl=super").unwrap();
        set.finalize();
        assert!(set.is_remapped(left("ctrl")));
        assert!(!set.is_remapped(left("alt")));
        assert!(!set.is_remapped(left("shift")));
    }

    #[test]
    fn remap_set_clone_creates_independent_copy() {
        let mut set = RemapSet::default();
        set.parse("ctrl=super").unwrap();
        set.finalize();
        let cloned = set.clone();
        assert!(set.equal(&cloned));
        assert!(cloned.is_remapped(left("ctrl")));
    }

    #[test]
    fn remap_set_equal_compares_correctly() {
        let mut set1 = RemapSet::default();
        let mut set2 = RemapSet::default();
        assert!(set1.equal(&set2));
        set1.parse("ctrl=super").unwrap();
        assert!(!set1.equal(&set2));
        set2.parse("ctrl=super").unwrap();
        assert!(set1.equal(&set2));
        set1.parse("alt=shift").unwrap();
        assert!(!set1.equal(&set2));
    }

    #[test]
    fn remap_set_parse_cli_basic() {
        let mut set = RemapSet::default();
        set.parse_cli(Some("ctrl=super")).unwrap();
        assert_eq!(set.map.len(), 2);
    }

    #[test]
    fn remap_set_parse_cli_empty_clears() {
        let mut set = RemapSet::default();
        set.parse_cli(Some("ctrl=super")).unwrap();
        assert_eq!(set.map.len(), 2);
        set.parse_cli(Some("")).unwrap();
        assert_eq!(set.map.len(), 0);
    }

    #[test]
    fn remap_set_parse_cli_invalid() {
        let mut set = RemapSet::default();
        assert_eq!(
            set.parse_cli(Some("foo=bar")),
            Err(ParseCliError::InvalidValue)
        );
        assert_eq!(
            set.parse_cli(Some("ctrl")),
            Err(ParseCliError::InvalidValue)
        );
    }

    #[test]
    fn remap_set_parse_aliased_modifiers() {
        let mut set = RemapSet::default();
        set.parse("cmd=ctrl").unwrap();
        set.finalize();
        assert_eq!(
            sided("ctrl", ModSide::Left),
            set.apply(sided("super", ModSide::Left))
        );
    }

    #[test]
    fn remap_set_parse_aliased_modifiers_command() {
        let mut set = RemapSet::default();
        set.parse("command=alt").unwrap();
        set.finalize();
        assert_eq!(
            sided("alt", ModSide::Left),
            set.apply(sided("super", ModSide::Left))
        );
    }

    #[test]
    fn remap_set_parse_aliased_modifiers_opt_and_option() {
        let mut set = RemapSet::default();
        set.parse("opt=super").unwrap();
        set.finalize();
        assert_eq!(
            sided("super", ModSide::Left),
            set.apply(sided("alt", ModSide::Left))
        );

        let mut set = RemapSet::default();
        set.parse("option=shift").unwrap();
        set.finalize();
        assert_eq!(
            sided("shift", ModSide::Left),
            set.apply(sided("alt", ModSide::Left))
        );
    }

    #[test]
    fn remap_set_parse_aliased_modifiers_control() {
        let mut set = RemapSet::default();
        set.parse("control=super").unwrap();
        set.finalize();
        assert_eq!(
            sided("super", ModSide::Left),
            set.apply(sided("ctrl", ModSide::Left))
        );
    }

    #[test]
    fn remap_set_parse_aliased_modifiers_on_target_side() {
        let mut set = RemapSet::default();
        set.parse("alt=cmd").unwrap();
        set.finalize();
        assert_eq!(
            sided("super", ModSide::Left),
            set.apply(sided("alt", ModSide::Left))
        );
    }

    #[test]
    fn remap_set_format_entry_empty() {
        // The `key-remap = \n` rendering is done by the config formatter;
        // here an empty set yields no entries.
        assert_eq!(RemapSet::default().format_entries(), Vec::<String>::new());
    }

    #[test]
    fn remap_set_format_entry_single_sided() {
        let mut set = RemapSet::default();
        set.parse("left_ctrl=super").unwrap();
        set.finalize();
        assert_eq!(set.format_entries(), vec!["left_ctrl=left_super"]);
    }

    #[test]
    fn remap_set_format_entry_unsided_creates_two_entries() {
        let mut set = RemapSet::default();
        set.parse("ctrl=super").unwrap();
        set.finalize();
        let w = set.format_entries();
        assert!(w.contains(&"left_ctrl=left_super".to_string()));
        assert!(w.contains(&"right_ctrl=left_super".to_string()));
    }

    #[test]
    fn remap_set_format_entry_right_sided() {
        let mut set = RemapSet::default();
        set.parse("left_alt=right_ctrl").unwrap();
        set.finalize();
        assert_eq!(set.format_entries(), vec!["left_alt=right_ctrl"]);
    }
}
