//! A binding maps some input trigger to an action. When the trigger
//! occurs, the action is performed. (ZIG/src/input/Binding.zig)

pub mod action;
pub mod set;

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};

pub use action::{
    Action, ActionKey, AdjustSelection, CloseTabMode, CopyToClipboard, CrashThread, CursorKey,
    GotoWindow, InspectorMode, NavigateSearch, Scope, SplitDirection, SplitFocusDirection,
    SplitResizeDirection, SplitResizeParameter, WriteScreen, WriteScreenAction, WriteScreenFormat,
};
pub use set::Set;

use crate::key;
use crate::key_mods::{self, Mods};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid format")]
    InvalidFormat,
    #[error("invalid action")]
    InvalidAction,
}

/// A binding: trigger, action and per-binding flags.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    /// The trigger that needs to be performed to execute the action.
    pub trigger: Trigger,

    /// The action to take if this binding matches
    pub action: Action,

    /// Boolean flags that can be set per binding.
    pub flags: Flags,
}

/// Flags the full binding-scoped flags that can be set per binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Flags {
    /// True if this binding should consume the input when the
    /// action is triggered.
    pub consumed: bool,

    /// True if this binding should be forwarded to all active surfaces
    /// in the application.
    pub all: bool,

    /// True if this binding is global. Global bindings should work system-wide
    /// and not just while Ghostty is focused. This may not work on all platforms.
    /// See the keybind config documentation for more information.
    pub global: bool,

    /// True if this binding should only be triggered if the action can be
    /// performed. If the action can't be performed then the binding acts as
    /// if it doesn't exist.
    pub performable: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Flags {
            consumed: true,
            all: false,
            global: false,
            performable: false,
        }
    }
}

impl Flags {
    /// Converts this to a C-compatible value (packed bits, `consumed` = bit 0).
    pub fn cval(self) -> u8 {
        u8::from(self.consumed)
            | u8::from(self.all) << 1
            | u8::from(self.global) << 2
            | u8::from(self.performable) << 3
    }
}

/// Elements yielded by the [`Parser`].
#[derive(Debug, Clone, PartialEq)]
pub enum Elem {
    /// A leader trigger in a sequence.
    Leader(Trigger),

    /// The final trigger and action in a sequence.
    Binding(Binding),

    /// A chained action `chain=<action>` that should be appended
    /// to the previous binding. Note that any action is parsed, including
    /// invalid actions for chains such as `unbind`. We expect downstream
    /// consumers to validate that the action is valid for chaining.
    Chain(Action),
}

/// Full binding parser. The binding parser is implemented as an iterator
/// which yields elements to support multi-key sequences.
#[derive(Debug, Clone)]
pub struct Parser<'a> {
    trigger_it: SequenceIterator<'a>,
    action: Action,
    flags: Flags,
    chain: bool,
}

impl<'a> Parser<'a> {
    pub fn new(raw_input: &'a str) -> Result<Parser<'a>, Error> {
        let (flags, start_idx) = parse_flags(raw_input)?;
        let input = &raw_input[start_idx..];

        // Find the equal sign. This is more complicated than it seems on
        // the surface because we need to ignore equal signs that are
        // part of the trigger.
        let eql_idx: usize = 'eql: {
            // TODO: We should change this parser into a real state machine
            // based parser that parses the trigger fully, then yields the
            // action after. The loop below is a total mess.
            let mut offset = 0usize;
            let bytes = input.as_bytes();
            while let Some(offset_idx) = input[offset..].find('=') {
                // Find: '=+ctrl' or '==action'
                let idx = offset + offset_idx;
                if idx + 1 < bytes.len() && (bytes[idx + 1] == b'+' || bytes[idx + 1] == b'=') {
                    offset += offset_idx + 1;
                    continue;
                }

                // Looks like the real equal sign.
                break 'eql idx;
            }

            return Err(Error::InvalidFormat);
        };

        // Detect chains. Chains must not have flags.
        let chain = &input[..eql_idx] == "chain";
        if chain && start_idx > 0 {
            return Err(Error::InvalidFormat);
        }

        // Sequence iterator goes up to the equal, action is after. We can
        // parse the action now.
        Ok(Parser {
            trigger_it: SequenceIterator {
                // This is kind of hacky but we put a dummy trigger
                // for chained inputs. The `next` will never yield this
                // because we have chain set.
                input: if chain { "a" } else { &input[..eql_idx] },
                i: 0,
            },
            action: Action::parse(&input[eql_idx + 1..])?,
            flags,
            chain,
        })
    }

    pub fn next_elem(&mut self) -> Result<Option<Elem>, Error> {
        // Get our trigger. If we're out of triggers then we're done.
        let Some(trigger) = self.trigger_it.next_trigger()? else {
            return Ok(None);
        };

        // If this is our last trigger then it is our final binding.
        if !self.trigger_it.done() {
            // Global/all bindings can't be sequences
            if self.flags.global || self.flags.all {
                return Err(Error::InvalidFormat);
            }
            return Ok(Some(Elem::Leader(trigger)));
        }

        // If we're a chain then return it as-is.
        if self.chain {
            return Ok(Some(Elem::Chain(self.action.clone())));
        }

        // Out of triggers, yield the final action.
        Ok(Some(Elem::Binding(Binding {
            trigger,
            action: self.action.clone(),
            flags: self.flags,
        })))
    }

    pub fn reset(&mut self) {
        self.trigger_it.i = 0;
    }
}

fn parse_flags(raw_input: &str) -> Result<(Flags, usize), Error> {
    let mut flags = Flags::default();

    let mut start_idx = 0usize;
    let mut input = raw_input;
    // Find the next prefix
    while let Some(idx) = input.find(':') {
        let prefix = &input[..idx];

        // If the prefix is one of our flags then set it.
        match prefix {
            "all" => {
                if flags.all {
                    return Err(Error::InvalidFormat);
                }
                flags.all = true;
            }
            "global" => {
                if flags.global {
                    return Err(Error::InvalidFormat);
                }
                flags.global = true;
            }
            "unconsumed" => {
                if !flags.consumed {
                    return Err(Error::InvalidFormat);
                }
                flags.consumed = false;
            }
            "performable" => {
                if flags.performable {
                    return Err(Error::InvalidFormat);
                }
                flags.performable = true;
            }
            // If we don't recognize the prefix then we're done. We
            // let any unknown prefix fallthrough to trigger-specific
            // parsing in case there are trigger-specific prefixes
            // (none currently but historically there was `physical:`
            // at one point). Breaking here lets us always implement new
            // prefixes.
            _ => break,
        }

        // Move past the prefix
        start_idx += idx + 1;
        input = &input[idx + 1..];
    }

    Ok((flags, start_idx))
}

/// An iterator that yields each trigger in a sequence of triggers. For
/// example, the sequence "ctrl+a>ctrl+b" would yield "ctrl+a" and then
/// "ctrl+b".
#[derive(Debug, Clone)]
pub struct SequenceIterator<'a> {
    /// The input of triggers. This is expected to be ONLY triggers. Things
    /// like the "unconsumed:" prefix or action must be stripped before
    /// passing to this iterator.
    pub input: &'a str,
    pub i: usize,
}

impl<'a> SequenceIterator<'a> {
    pub fn new(input: &'a str) -> Self {
        SequenceIterator { input, i: 0 }
    }

    /// Returns the next trigger in the sequence if there is no parsing error.
    pub fn next_trigger(&mut self) -> Result<Option<Trigger>, Error> {
        if self.done() {
            return Ok(None);
        }
        let rem = &self.input[self.i..];
        let idx = rem.find('>').unwrap_or(rem.len());
        self.i += idx + 1;
        Ok(Some(Trigger::parse(&rem[..idx])?))
    }

    /// Returns true if there are no more triggers to parse.
    pub fn done(&self) -> bool {
        self.i > self.input.len()
    }
}

/// Parse a single, non-sequenced binding. To support sequences you must
/// use parse. This is a convenience function for single bindings aimed
/// primarily at tests.
///
/// This doesn't support `chain` either, since chaining requires some
/// stateful concept of a prior binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseSingleError {
    Error(Error),
    UnexpectedChain,
    UnexpectedSequence,
}

impl From<Error> for ParseSingleError {
    fn from(e: Error) -> Self {
        ParseSingleError::Error(e)
    }
}

pub fn parse_single(raw_input: &str) -> Result<Binding, ParseSingleError> {
    let mut p = Parser::new(raw_input)?;
    let elem = p.next_elem()?.ok_or(Error::InvalidFormat)?;
    match elem {
        Elem::Leader(_) => Err(ParseSingleError::UnexpectedSequence),
        Elem::Binding(b) => Ok(b),
        Elem::Chain(_) => Err(ParseSingleError::UnexpectedChain),
    }
}

/// Returns true if lhs should be sorted before rhs
pub fn less_than(lhs: &Binding, rhs: &Binding) -> bool {
    let count = |m: Mods| {
        usize::from(m.super_) + usize::from(m.ctrl) + usize::from(m.shift) + usize::from(m.alt)
    };
    let lhs_count = count(lhs.trigger.mods);
    let rhs_count = count(rhs.trigger.mods);

    if lhs_count != rhs_count {
        return lhs_count > rhs_count;
    }

    if lhs.trigger.mods.int() != rhs.trigger.mods.int() {
        return lhs.trigger.mods.int() > rhs.trigger.mods.int();
    }

    let k = |t: &Trigger| -> i64 {
        match t.key {
            TriggerKey::Physical(k) => k as i64,
            TriggerKey::Unicode(cp) => i64::from(cp),
            // Zig reads the C-union tag's key here (undefined); treat as 0.
            TriggerKey::CatchAll => 0,
        }
    };
    k(&lhs.trigger) < k(&rhs.trigger)
}

/// The key part of a [`Trigger`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriggerKey {
    /// key is the "physical" version. This is the same as mapped for
    /// standard US keyboard layouts. For non-US keyboard layouts, this
    /// is used to bind to a physical key location rather than a translated
    /// key.
    Physical(key::Key),

    /// This is used for binding to keys that produce a certain unicode
    /// codepoint. This is useful for binding to keys that don't have a
    /// registered keycode with Ghostty.
    Unicode(u32),

    /// A catch-all key that matches any key press that is otherwise
    /// unbound.
    CatchAll,
}

/// Trigger is the associated key state that can trigger an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigger {
    /// The key that has to be pressed for a binding to take action.
    pub key: TriggerKey,

    /// The key modifiers that must be active for this to match.
    pub mods: Mods,
}

impl Default for Trigger {
    fn default() -> Self {
        Trigger {
            key: TriggerKey::Physical(key::Key::Unidentified),
            mods: Mods::default(),
        }
    }
}

/// Backwards compatible key names from Ghostty 1.1.x. Ghostty 1.2+ doesn't
/// support these anymore since we moved to W3C key codes.
pub const BACKWARDS_COMPATIBLE_KEYS: &[(&str, TriggerKey)] = {
    use TriggerKey::{Physical as P, Unicode as U};
    use key::Key as K;
    &[
        ("zero", U('0' as u32)),
        ("one", U('1' as u32)),
        ("two", U('2' as u32)),
        ("three", U('3' as u32)),
        ("four", U('4' as u32)),
        ("five", U('5' as u32)),
        ("six", U('6' as u32)),
        ("seven", U('7' as u32)),
        ("eight", U('8' as u32)),
        ("nine", U('9' as u32)),
        ("plus", U('+' as u32)),
        ("apostrophe", U('\'' as u32)),
        ("grave_accent", P(K::Backquote)),
        ("left_bracket", P(K::BracketLeft)),
        ("right_bracket", P(K::BracketRight)),
        ("up", P(K::ArrowUp)),
        ("down", P(K::ArrowDown)),
        ("left", P(K::ArrowLeft)),
        ("right", P(K::ArrowRight)),
        ("kp_0", P(K::Numpad0)),
        ("kp_1", P(K::Numpad1)),
        ("kp_2", P(K::Numpad2)),
        ("kp_3", P(K::Numpad3)),
        ("kp_4", P(K::Numpad4)),
        ("kp_5", P(K::Numpad5)),
        ("kp_6", P(K::Numpad6)),
        ("kp_7", P(K::Numpad7)),
        ("kp_8", P(K::Numpad8)),
        ("kp_9", P(K::Numpad9)),
        ("kp_add", P(K::NumpadAdd)),
        ("kp_subtract", P(K::NumpadSubtract)),
        ("kp_multiply", P(K::NumpadMultiply)),
        ("kp_divide", P(K::NumpadDivide)),
        ("kp_decimal", P(K::NumpadDecimal)),
        ("kp_enter", P(K::NumpadEnter)),
        ("kp_equal", P(K::NumpadEqual)),
        ("kp_separator", P(K::NumpadSeparator)),
        ("kp_left", P(K::NumpadLeft)),
        ("kp_right", P(K::NumpadRight)),
        ("kp_up", P(K::NumpadUp)),
        ("kp_down", P(K::NumpadDown)),
        ("kp_page_up", P(K::NumpadPageUp)),
        ("kp_page_down", P(K::NumpadPageDown)),
        ("kp_home", P(K::NumpadHome)),
        ("kp_end", P(K::NumpadEnd)),
        ("kp_insert", P(K::NumpadInsert)),
        ("kp_delete", P(K::NumpadDelete)),
        ("kp_begin", P(K::NumpadBegin)),
        ("left_shift", P(K::ShiftLeft)),
        ("right_shift", P(K::ShiftRight)),
        ("left_control", P(K::ControlLeft)),
        ("right_control", P(K::ControlRight)),
        ("left_alt", P(K::AltLeft)),
        ("right_alt", P(K::AltRight)),
        ("left_super", P(K::MetaLeft)),
        ("right_super", P(K::MetaRight)),
        // Physical variants. This is a blunt approach to this but its
        // glue for backwards compatibility so I'm not too worried about
        // making this super nice.
        ("physical:zero", P(K::Digit0)),
        ("physical:one", P(K::Digit1)),
        ("physical:two", P(K::Digit2)),
        ("physical:three", P(K::Digit3)),
        ("physical:four", P(K::Digit4)),
        ("physical:five", P(K::Digit5)),
        ("physical:six", P(K::Digit6)),
        ("physical:seven", P(K::Digit7)),
        ("physical:eight", P(K::Digit8)),
        ("physical:nine", P(K::Digit9)),
        ("physical:apostrophe", P(K::Quote)),
        ("physical:grave_accent", P(K::Backquote)),
        ("physical:left_bracket", P(K::BracketLeft)),
        ("physical:right_bracket", P(K::BracketRight)),
        ("physical:up", P(K::ArrowUp)),
        ("physical:down", P(K::ArrowDown)),
        ("physical:left", P(K::ArrowLeft)),
        ("physical:right", P(K::ArrowRight)),
        ("physical:kp_0", P(K::Numpad0)),
        ("physical:kp_1", P(K::Numpad1)),
        ("physical:kp_2", P(K::Numpad2)),
        ("physical:kp_3", P(K::Numpad3)),
        ("physical:kp_4", P(K::Numpad4)),
        ("physical:kp_5", P(K::Numpad5)),
        ("physical:kp_6", P(K::Numpad6)),
        ("physical:kp_7", P(K::Numpad7)),
        ("physical:kp_8", P(K::Numpad8)),
        ("physical:kp_9", P(K::Numpad9)),
        ("physical:kp_add", P(K::NumpadAdd)),
        ("physical:kp_subtract", P(K::NumpadSubtract)),
        ("physical:kp_multiply", P(K::NumpadMultiply)),
        ("physical:kp_divide", P(K::NumpadDivide)),
        ("physical:kp_decimal", P(K::NumpadDecimal)),
        ("physical:kp_enter", P(K::NumpadEnter)),
        ("physical:kp_equal", P(K::NumpadEqual)),
        ("physical:kp_separator", P(K::NumpadSeparator)),
        ("physical:kp_left", P(K::NumpadLeft)),
        ("physical:kp_right", P(K::NumpadRight)),
        ("physical:kp_up", P(K::NumpadUp)),
        ("physical:kp_down", P(K::NumpadDown)),
        ("physical:kp_page_up", P(K::NumpadPageUp)),
        ("physical:kp_page_down", P(K::NumpadPageDown)),
        ("physical:kp_home", P(K::NumpadHome)),
        ("physical:kp_end", P(K::NumpadEnd)),
        ("physical:kp_insert", P(K::NumpadInsert)),
        ("physical:kp_delete", P(K::NumpadDelete)),
        ("physical:kp_begin", P(K::NumpadBegin)),
        ("physical:left_shift", P(K::ShiftLeft)),
        ("physical:right_shift", P(K::ShiftRight)),
        ("physical:left_control", P(K::ControlLeft)),
        ("physical:right_control", P(K::ControlRight)),
        ("physical:left_alt", P(K::AltLeft)),
        ("physical:right_alt", P(K::AltRight)),
        ("physical:left_super", P(K::MetaLeft)),
        ("physical:right_super", P(K::MetaRight)),
    ]
};

/// The codepoint we use for comparisons. Case folding can result
/// in more codepoints so we need to use a 3 element array.
///
/// Uses the full Unicode case folding table (uucode's `case_folding_full`,
/// generated by `ghostty-unicode`).
fn folded_codepoint(cp: u32) -> [u32; 3] {
    // ASCII fast path
    if let Some(c) = char::from_u32(cp)
        && c.is_ascii_alphabetic()
    {
        return [u32::from(c.to_ascii_lowercase()), 0, 0];
    }

    // Unicode slow path. Case folding can result in more codepoints.
    ghostty_unicode::case_folding::fold_full(cp)
}

impl Trigger {
    /// Parse a single trigger. The input is expected to be ONLY the trigger
    /// (i.e. in the sequence `a=ignore` input is only `a`). The trigger may
    /// not be part of a sequence (i.e. `a>b`). This parses exactly a single
    /// trigger.
    pub fn parse(input: &str) -> Result<Trigger, Error> {
        if input.is_empty() {
            return Err(Error::InvalidFormat);
        }
        let mut result = Trigger::default();
        let mut rem: &str = input;
        'outer: while !rem.is_empty() {
            let idx = rem.find('+').unwrap_or(rem.len());
            let part = &rem[..idx];
            rem = if idx >= rem.len() {
                ""
            } else {
                &rem[idx + 1..]
            };

            // Check if its a modifier
            {
                let slot: Option<&mut bool> = match part {
                    "shift" => Some(&mut result.mods.shift),
                    "ctrl" => Some(&mut result.mods.ctrl),
                    "alt" => Some(&mut result.mods.alt),
                    "super" => Some(&mut result.mods.super_),
                    "caps_lock" => Some(&mut result.mods.caps_lock),
                    "num_lock" => Some(&mut result.mods.num_lock),
                    _ => None,
                };
                if let Some(slot) = slot {
                    // Repeat not allowed
                    if *slot {
                        return Err(Error::InvalidFormat);
                    }
                    *slot = true;
                    continue 'outer;
                }
            }

            // Alias modifiers
            for (alias, m) in key_mods::ALIAS {
                if part == *alias {
                    // Repeat not allowed
                    let slot = match m {
                        key_mods::Mod::Shift => &mut result.mods.shift,
                        key_mods::Mod::Ctrl => &mut result.mods.ctrl,
                        key_mods::Mod::Alt => &mut result.mods.alt,
                        key_mods::Mod::Super => &mut result.mods.super_,
                    };
                    if *slot {
                        return Err(Error::InvalidFormat);
                    }
                    *slot = true;
                    continue 'outer;
                }
            }

            // Anything after this point is a key and we only support
            // single keys.
            if !result.is_key_unset() {
                return Err(Error::InvalidFormat);
            }

            // If the part is empty it means that it is actually
            // a literal `+`, which we treat as a Unicode character.
            if part.is_empty() {
                result.key = TriggerKey::Unicode('+' as u32);
                continue;
            }

            // Check if its a key
            if part != "unidentified"
                && let Some(k) = key::Key::from_name(part)
            {
                result.key = TriggerKey::Physical(k);
                continue;
            }

            // If we're still unset and we have exactly one unicode
            // character then we can use that as a key.
            {
                let mut it = part.chars();
                if let (Some(c), None) = (it.next(), it.next()) {
                    result.key = TriggerKey::Unicode(c as u32);
                    continue;
                }
            }

            // Look for a matching w3c name next.
            if let Some(w3c_key) = key::Key::from_w3c(part) {
                result.key = TriggerKey::Physical(w3c_key);
                continue;
            }

            // Check for catch_all. We do this near the end since its unlikely
            // in most cases that we're setting a catch-all key.
            if part == "catch_all" {
                result.key = TriggerKey::CatchAll;
                continue;
            }

            // If we're still unset then we look for backwards compatible
            // keys with Ghostty 1.1.x. We do this last so its least likely
            // to impact performance for modern users.
            if let Some((_, old_key)) = BACKWARDS_COMPATIBLE_KEYS.iter().find(|e| e.0 == part) {
                result.key = *old_key;
                continue;
            }

            // We didn't recognize this value
            return Err(Error::InvalidFormat);
        }

        Ok(result)
    }

    /// Returns true if this trigger has no key set.
    pub fn is_key_unset(&self) -> bool {
        match self.key {
            TriggerKey::Physical(v) => v == key::Key::Unidentified,
            TriggerKey::Unicode(_) | TriggerKey::CatchAll => false,
        }
    }

    /// Returns a hash code that can be used to uniquely identify this trigger.
    pub fn hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash_incremental(&mut h);
        h.finish()
    }

    /// Hash the trigger into the given hasher (case-folded for unicode keys).
    pub fn hash_incremental<H: Hasher>(&self, h: &mut H) {
        std::mem::discriminant(&self.key).hash(h);
        match self.key {
            TriggerKey::Physical(v) => v.hash(h),
            TriggerKey::Unicode(cp) => folded_codepoint(cp).hash(h),
            TriggerKey::CatchAll => {}
        }
        self.mods.binding().hash(h);
    }

    /// Returns true if two triggers are equal.
    pub fn equal(&self, other: &Trigger) -> bool {
        self == other
    }

    /// Returns true if two triggers are equal using folded codepoints.
    pub fn folded_equal(&self, other: &Trigger) -> bool {
        if self.mods != other.mods {
            return false;
        }
        match (self.key, other.key) {
            (TriggerKey::Physical(a), TriggerKey::Physical(b)) => a == b,
            (TriggerKey::Unicode(a), TriggerKey::Unicode(b)) => {
                folded_codepoint(a) == folded_codepoint(b)
            }
            (TriggerKey::CatchAll, TriggerKey::CatchAll) => true,
            _ => false,
        }
    }

    /// Convert the trigger to a C API compatible trigger.
    pub fn cval(&self) -> TriggerC {
        match self.key {
            TriggerKey::Physical(v) => TriggerC {
                tag: TriggerCTag::Physical,
                key: v as u32,
                mods: self.mods,
            },
            TriggerKey::Unicode(v) => TriggerC {
                tag: TriggerCTag::Unicode,
                key: v,
                mods: self.mods,
            },
            TriggerKey::CatchAll => TriggerC {
                tag: TriggerCTag::CatchAll,
                key: 0,
                mods: self.mods,
            },
        }
    }
}

/// C API tag for triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum TriggerCTag {
    Physical,
    Unicode,
    CatchAll,
}

/// The struct used for triggers in the C API (`Trigger.C`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerC {
    pub tag: TriggerCTag,
    /// Physical key enum value or unicode codepoint (undefined for catch_all).
    pub key: u32,
    pub mods: Mods,
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Modifiers first
        if self.mods.super_ {
            f.write_str("super+")?;
        }
        if self.mods.ctrl {
            f.write_str("ctrl+")?;
        }
        if self.mods.alt {
            f.write_str("alt+")?;
        }
        if self.mods.shift {
            f.write_str("shift+")?;
        }

        // Key
        match self.key {
            TriggerKey::Physical(k) => f.write_str(k.name()),
            TriggerKey::Unicode(c) => {
                // Zig `{u}` writes the codepoint as UTF-8.
                match char::from_u32(c) {
                    Some(ch) => write!(f, "{ch}"),
                    None => f.write_str("\u{FFFD}"),
                }
            }
            TriggerKey::CatchAll => f.write_str("catch_all"),
        }
    }
}

/// Hash-map key wrapper for triggers: equality and hashing use the
/// case-folded comparison (`Trigger.bindingSetEqual`).
#[derive(Debug, Clone, Copy)]
pub struct FoldedTrigger(pub Trigger);

impl PartialEq for FoldedTrigger {
    fn eq(&self, other: &Self) -> bool {
        self.0.folded_equal(&other.0)
    }
}
impl Eq for FoldedTrigger {}
impl Hash for FoldedTrigger {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash_incremental(state);
    }
}

#[cfg(test)]
mod tests;
