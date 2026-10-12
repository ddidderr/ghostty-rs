//! A structure that contains a set of bindings and focuses on fast lookup.
//! The use case is that this will be called on EVERY key input to look
//! for an associated action so it must be fast. (`Binding.Set`)

use std::fmt::Write as _;

use indexmap::IndexMap;

use super::{Action, Elem, Error, Flags, FoldedTrigger, Parser, Trigger, TriggerKey};
use crate::key::KeyEvent;

/// Sink for `config/formatter.zig` `EntryFormatter`. The config crate
/// implements this (`ghostty_config::keybinds`) to write `keybind = <value>`
/// lines.
pub trait EntryFormatter {
    /// Format a single `key = value` entry where `value` is a string.
    fn format_entry(&mut self, value: &str);
}

/// Errors from [`Set::append_chain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AppendChainError {
    #[error("no chain parent")]
    NoChainParent,
}

/// Leaf node of a set is an action to trigger. This is a "leaf" compared
/// to the inner nodes which are "leaders" for sequences.
#[derive(Debug, Clone, PartialEq)]
pub struct Leaf {
    pub action: Action,
    pub flags: Flags,
}

impl Leaf {
    pub fn hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.action.hash_into(&mut h);
        self.flags.hash(&mut h);
        h.finish()
    }

    pub fn generic(&self) -> GenericLeaf<'_> {
        GenericLeaf {
            flags: self.flags,
            actions: std::slice::from_ref(&self.action),
        }
    }
}

/// Leaf node of a set that triggers multiple actions in sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct LeafChained {
    pub actions: Vec<Action>,
    pub flags: Flags,
}

impl LeafChained {
    pub fn generic(&self) -> GenericLeaf<'_> {
        GenericLeaf {
            flags: self.flags,
            actions: &self.actions,
        }
    }
}

/// A generic leaf node that can be used to unify the handling of
/// leaf and leaf_chained.
#[derive(Debug, Clone, Copy)]
pub struct GenericLeaf<'a> {
    pub flags: Flags,
    pub actions: &'a [Action],
}

impl GenericLeaf<'_> {
    pub fn actions_slice(&self) -> &[Action] {
        self.actions
    }
}

/// The entry type for the forward mapping of trigger to action.
#[derive(Debug, Clone)]
pub enum Value {
    /// This key is a leader key in a sequence. You must follow the given
    /// set to find the next key in the sequence.
    Leader(Box<Set>),

    /// This trigger completes a sequence and the value is the action
    /// to take along with the flags that may define binding behavior.
    Leaf(Leaf),

    /// A set of actions to take in response to a trigger.
    LeafChained(LeafChained),
}

impl Value {
    pub fn as_leaf(&self) -> Option<&Leaf> {
        match self {
            Value::Leaf(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_leader(&self) -> Option<&Set> {
        match self {
            Value::Leader(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_leaf_chained(&self) -> Option<&LeafChained> {
        match self {
            Value::LeafChained(l) => Some(l),
            _ => None,
        }
    }

    /// Implements the formatter for the fmt package. This encodes the
    /// action back into the format used by parse.
    ///
    /// NOTE: the Zig version has no `leaf_chained` prong (it would not
    /// compile if used); here only the first action is written.
    pub fn format(&self) -> String {
        let mut out = String::new();
        match self {
            Value::Leader(set) => {
                // the leader key was already printed.
                for (k, v) in &set.bindings {
                    let _ = write!(out, ">{}{}", k.0, v.format());
                }
            }
            Value::Leaf(leaf) => {
                // action implements the format
                let _ = write!(out, "={}", leaf.action);
            }
            Value::LeafChained(leaf) => {
                if let Some(a) = leaf.actions.first() {
                    let _ = write!(out, "={a}");
                }
            }
        }
        out
    }

    /// Writes the configuration entries for the binding
    /// that this value is part of.
    ///
    /// The value may be part of multiple configuration entries
    /// if they're all part of the same prefix sequence (e.g. 'a>b', 'a>c').
    /// These will result in multiple separate entries in the configuration.
    ///
    /// `buffer` is temporary storage that is shared between calls to
    /// nested levels of the set. For example, 'a>b>c=x' and 'a>b>d=y' will
    /// reuse the 'a>b' written to the buffer before flushing it to the
    /// formatter with 'c=x' and 'd=y'.
    pub fn format_entries(&self, buffer: &mut String, formatter: &mut dyn EntryFormatter) {
        match self {
            Value::Leader(set) => {
                // We'll rewind to this position after each sub-entry,
                // sharing the prefix between siblings.
                let pos = buffer.len();
                for (k, v) in &set.bindings {
                    buffer.truncate(pos);
                    let _ = write!(
                        buffer,
                        ">{}",
                        ghostty_os::osbytes::valid_str_to_config_string(&k.0.to_string())
                    );
                    v.format_entries(buffer, formatter);
                }
                buffer.truncate(pos);
            }

            Value::Leaf(leaf) => {
                // When we get to the leaf, the buffer contains
                // the full sequence of keys needed to reach this action.
                let pos = buffer.len();
                let _ = write!(buffer, "={}", leaf.action);
                formatter.format_entry(buffer);
                buffer.truncate(pos);
            }

            Value::LeafChained(leaf) => {
                let pos = buffer.len();
                for (i, action) in leaf.actions.iter().enumerate() {
                    if i == 0 {
                        let _ = write!(buffer, "={action}");
                        formatter.format_entry(buffer);
                        buffer.truncate(pos);
                    } else {
                        // DEVIATION: Zig rewinds the shared buffer to 0 and
                        // overwrites the sequence prefix; we use a scratch
                        // string so sibling entries keep their prefix.
                        let line = format!("chain={action}");
                        formatter.format_entry(&line);
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecurseError {
    SequenceUnbind,
    NoChainParent,
    UnexpectedEndOfInput,
    InvalidChainAction,
}

/// A set of bindings with fast lookup.
#[derive(Debug, Default)]
pub struct Set {
    /// The set of bindings.
    pub bindings: IndexMap<FoldedTrigger, Value>,

    /// The reverse mapping of action to binding. Note that multiple
    /// bindings can map to the same action and this map will only have
    /// the most recently added binding for an action.
    ///
    /// Sequenced triggers are never present in the reverse map at this time.
    /// This is a conscious decision since the primary use case of the reverse
    /// map is to support GUI toolkit keyboard accelerators and no mainstream
    /// GUI toolkit supports sequences.
    ///
    /// Performable triggers are also not present in the reverse map. This
    /// is so that GUI toolkits don't register performable triggers as
    /// menu shortcuts (the primary use case of the reverse map). GUI toolkits
    /// such as GTK handle menu shortcuts too early in the event lifecycle
    /// for performable to work so this is a conscious decision to ease the
    /// integration with GUI toolkits.
    reverse: IndexMap<Action, Trigger>,

    /// The chain parent is the information necessary to attach a chained
    /// action to the proper location in our mapping.
    ///
    /// DEVIATION: Zig stores raw pointers; here it is the path of triggers
    /// (through leader sets) from this set to the parent entry. The last
    /// element is the parent's own trigger.
    chain_parent: Option<Vec<Trigger>>,
}

impl Clone for Set {
    /// Deep clone the set. The clone has no chain parent.
    fn clone(&self) -> Self {
        Set {
            bindings: self.bindings.clone(),
            reverse: self.reverse.clone(),
            chain_parent: None,
        }
    }
}

impl Set {
    pub fn new() -> Set {
        Set::default()
    }

    /// Deep clone the set (see [`Clone`]).
    pub fn clone_set(&self) -> Set {
        self.clone()
    }

    /// The trigger path of the current chain parent, if any.
    pub fn chain_parent(&self) -> Option<&[Trigger]> {
        self.chain_parent.as_deref()
    }

    /// The (last) trigger of the chain parent, if any.
    pub fn chain_parent_trigger(&self) -> Option<Trigger> {
        self.chain_parent.as_ref().and_then(|p| p.last().copied())
    }

    /// The value of the chain parent, if any.
    pub fn chain_parent_value(&self) -> Option<&Value> {
        let path = self.chain_parent.as_ref()?;
        let (last, prefix) = path.split_last()?;
        let mut cur = self;
        for t in prefix {
            cur = cur.bindings.get(&FoldedTrigger(*t))?.as_leader()?;
        }
        cur.bindings.get(&FoldedTrigger(*last))
    }

    /// Parse a user input binding and add it to the set. This will handle
    /// the "unbind" case, ensure consumed/unconsumed fields are set correctly,
    /// handle sequences, etc.
    ///
    /// Any Error returned is validated before any set modifications are made.
    pub fn parse_and_put(&mut self, input: &str) -> Result<(), Error> {
        self.parse_and_put_bytes(input.as_bytes())
    }

    pub fn parse_and_put_bytes(&mut self, input: &[u8]) -> Result<(), Error> {
        // To make cleanup easier, we ensure that the full sequence is
        // valid before making any set modifications.
        let mut it = Parser::from_bytes(input)?;
        while it.next_elem()?.is_some() {}
        it.reset();

        match Set::parse_and_put_recurse(self, &mut it, true) {
            // If we have an updated set (a binding was added) then we store
            // it for our chain parent.
            Ok(Some(path)) => self.chain_parent = Some(path),

            // If we didn't update a set then we clear our chain parent since
            // chaining is no longer valid until a valid binding is saved.
            // SequenceUnbind reaching the root means we've unbound all the
            // way up and this put was a success.
            Ok(None) | Err(RecurseError::SequenceUnbind) => self.chain_parent = None,

            // If our parser input was too short then the format
            // is invalid because we handle all valid cases. A chain without
            // a parent or an invalid chain action are format errors too.
            Err(
                RecurseError::UnexpectedEndOfInput
                | RecurseError::NoChainParent
                | RecurseError::InvalidChainAction,
            ) => return Err(Error::InvalidFormat),
        }
        Ok(())
    }

    /// Returns the path (relative to `set`) of the leaf that was
    /// ultimately updated if a binding was added.
    fn parse_and_put_recurse(
        set: &mut Set,
        it: &mut Parser<'_>,
        root: bool,
    ) -> Result<Option<Vec<Trigger>>, RecurseError> {
        let elem = it
            .next_elem()
            .ok()
            .flatten()
            .ok_or(RecurseError::UnexpectedEndOfInput)?;
        match elem {
            Elem::Leader(t) => {
                // If we have a leader, we need to upsert a set for it.
                enum Old {
                    None,
                    Leaf(Leaf),
                    Chained(LeafChained),
                }
                let key = FoldedTrigger(t);
                let mut old = Old::None;
                if let Some(entry) = set.bindings.get_mut(&key) {
                    match entry {
                        // We have an existing leader for this key already
                        // so recurse into this set.
                        Value::Leader(s) => {
                            let r = Set::parse_and_put_recurse(s, it, false);
                            return match r {
                                Ok(path) => Ok(path.map(|p| prepend(t, p))),
                                // Our child put unbound. If our set is empty we
                                // need to dealloc and continue up. If our set is
                                // not empty then we're done.
                                Err(RecurseError::SequenceUnbind) => {
                                    if s.bindings.is_empty() {
                                        set.remove(t);
                                        Err(RecurseError::SequenceUnbind)
                                    } else {
                                        Ok(None)
                                    }
                                }
                                Err(e) => Err(e),
                            };
                        }
                        Value::Leaf(l) => old = Old::Leaf(l.clone()),
                        Value::LeafChained(l) => old = Old::Chained(l.clone()),
                    }
                    // Remove the existing action. Fallthrough as if
                    // we don't have a leader.
                    set.remove(t);
                }

                // Create our new set for this leader and insert the leader entry
                set.bindings.insert(key, Value::Leader(Box::default()));

                // Recurse
                let r = {
                    let Some(Value::Leader(next)) = set.bindings.get_mut(&key) else {
                        unreachable!("leader was just inserted");
                    };
                    Set::parse_and_put_recurse(next, it, false)
                };
                match r {
                    Ok(path) => Ok(path.map(|p| prepend(t, p))),
                    // If our action was to unbind, we restore the old
                    // action if we have it.
                    Err(RecurseError::SequenceUnbind) => {
                        set.remove(t);
                        match old {
                            Old::None => {}
                            Old::Leaf(leaf) => set.put_flags(t, leaf.action, leaf.flags),
                            Old::Chained(leaf) => {
                                // Rebuild our chain
                                let mut actions = leaf.actions.into_iter();
                                if let Some(first) = actions.next() {
                                    set.put_flags(t, first, leaf.flags);
                                    for action in actions {
                                        if set.append_chain(action).is_err() {
                                            set.remove(t);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                        Ok(None)
                    }
                    Err(e) => Err(e),
                }
            }

            Elem::Binding(b) => match b.action {
                Action::Unbind => {
                    set.remove(b.trigger);
                    Err(RecurseError::SequenceUnbind)
                }
                action => {
                    set.put_flags(b.trigger, action, b.flags);
                    Ok(set.chain_parent.clone())
                }
            },

            Elem::Chain(action) => {
                // Chains can only happen on the root.
                debug_assert!(root);
                // Unbind is not valid for chains.
                if action == Action::Unbind {
                    return Err(RecurseError::InvalidChainAction);
                }
                set.append_chain(action)
                    .map_err(|_| RecurseError::NoChainParent)?;
                Ok(set.chain_parent.clone())
            }
        }
    }

    /// Add a binding to the set. If the binding already exists then
    /// this will overwrite it.
    pub fn put(&mut self, t: Trigger, action: Action) {
        self.put_flags(t, action, Flags::default());
    }

    /// Add a binding to the set with explicit flags.
    pub fn put_flags(&mut self, t: Trigger, action: Action, flags: Flags) {
        // unbind should never go into the set, it should be handled prior
        assert_ne!(action, Action::Unbind);

        // This is true if we're going to track this entry as
        // a reverse mapping. There are certain scenarios we don't.
        // See the reverse map docs for more information.
        let track_reverse = !flags.performable;

        self.chain_parent = None;

        let key = FoldedTrigger(t);
        // If we have an existing binding for this trigger, we have to
        // update the reverse mapping to remove the old action. A leader or
        // chained leaf is simply replaced (chained leaves aren't in the
        // reverse mapping).
        if track_reverse && matches!(self.bindings.get(&key), Some(Value::Leaf(_))) {
            let t_hash = t.hash();
            if let Some(i) = self.reverse.values().position(|v| v.hash() == t_hash) {
                self.reverse.swap_remove_index(i);
            }
        }

        // `insert` keeps the existing key and position (like getOrPut).
        self.bindings.insert(
            key,
            Value::Leaf(Leaf {
                action: action.clone(),
                flags,
            }),
        );

        if track_reverse {
            self.reverse.insert(action, t);
        }

        let stored = self.bindings.get_key_value(&key).map_or(t, |(k, _)| k.0);
        self.chain_parent = Some(vec![stored]);
    }

    /// Append a chained action to the prior set action.
    ///
    /// It is an error if there is no valid prior chain parent.
    pub fn append_chain(&mut self, action: Action) -> Result<(), AppendChainError> {
        // Unbind is not a valid chain action; callers must check this.
        assert_ne!(action, Action::Unbind);

        let path = self
            .chain_parent
            .clone()
            .ok_or(AppendChainError::NoChainParent)?;
        let (last, prefix) = path.split_last().ok_or(AppendChainError::NoChainParent)?;

        // Navigate to the set that owns the parent entry.
        let mut parent_set: &mut Set = self;
        for t in prefix {
            match parent_set.bindings.get_mut(&FoldedTrigger(*t)) {
                Some(Value::Leader(s)) => parent_set = s,
                _ => return Err(AppendChainError::NoChainParent),
            }
        }

        let key = FoldedTrigger(*last);
        let Some((idx, stored_key, value)) = parent_set.bindings.get_full_mut(&key) else {
            return Err(AppendChainError::NoChainParent);
        };
        let stored_key = stored_key.0;
        let _ = idx;
        match value {
            // Leader can never be a chain parent. Verified through various
            // assertions and unit tests.
            Value::Leader(_) => unreachable!("leader can never be a chain parent"),

            // If it is already a chained action, we just append the
            // action. Easy!
            Value::LeafChained(leaf) => leaf.actions.push(action),

            // If it is a leaf, we need to convert it to a leaf_chained.
            // We also need to be careful to remove any prior reverse
            // mappings for this action since chained actions are not
            // part of the reverse mapping.
            Value::Leaf(leaf) => {
                let first = leaf.action.clone();
                let flags = leaf.flags;

                // Convert to leaf_chained first, before fixing up reverse
                // mapping, so the fixup doesn't find this (now chained) entry.
                *value = Value::LeafChained(LeafChained {
                    actions: vec![first.clone(), action],
                    flags,
                });

                // Clean up our reverse mapping. Chained actions are not
                // part of the reverse mapping, so we need to fix up the
                // reverse map (possibly restoring another trigger for the
                // same action).
                parent_set.fixup_reverse_for_action(&first, stored_key);
            }
        }
        Ok(())
    }

    /// Get a binding for a given trigger.
    pub fn get(&self, t: Trigger) -> Option<&Value> {
        self.bindings.get(&FoldedTrigger(t))
    }

    /// Get the stored key and value for a given trigger.
    pub fn get_entry(&self, t: Trigger) -> Option<(&Trigger, &Value)> {
        self.bindings
            .get_key_value(&FoldedTrigger(t))
            .map(|(k, v)| (&k.0, v))
    }

    /// Get a trigger for the given action. An action can have multiple
    /// triggers so this will return the first one found.
    pub fn get_trigger(&self, a: &Action) -> Option<Trigger> {
        self.reverse.get(a).copied()
    }

    /// Get an entry for the given key event. This will attempt to find
    /// a binding using multiple parts of the event in the following order:
    ///
    ///   1. Translated key (event.key)
    ///   2. Physical key (event.physical_key)
    ///   3. Unshifted Unicode codepoint (event.unshifted_codepoint)
    pub fn get_event(&self, event: &KeyEvent<'_>) -> Option<(&Trigger, &Value)> {
        let mut trigger = Trigger {
            mods: event.mods.binding(),
            key: TriggerKey::Physical(event.key),
        };
        if let Some(v) = self.get_entry(trigger) {
            return Some(v);
        }

        // If our UTF-8 text is exactly one codepoint, we try to match that.
        if !event.utf8.is_empty() {
            // Invalid UTF-8, no codepoints or multiple codepoints drops to
            // invalid format.
            let mut it = crate::key_encode::utf8_chars(event.utf8).unwrap_or_else(|| "".chars());
            if let (Some(cp), None) = (it.next(), it.next()) {
                trigger.key = TriggerKey::Unicode(cp as u32);
                if let Some(v) = self.get_entry(trigger) {
                    return Some(v);
                }
            }
        }

        // Finally fallback to the full unshifted codepoint if we have one.
        // Question: should we be doing this if we have UTF-8 text? I
        // suspect "no" but we don't currently have any failing scenarios
        // to verify this.
        if event.unshifted_codepoint > 0 {
            trigger.key = TriggerKey::Unicode(event.unshifted_codepoint);
            if let Some(v) = self.get_entry(trigger) {
                return Some(v);
            }
        }

        // Fallback to catch_all with modifiers first, then without modifiers.
        trigger.key = TriggerKey::CatchAll;
        if let Some(v) = self.get_entry(trigger) {
            return Some(v);
        }
        if !trigger.mods.empty() {
            trigger.mods = crate::key_mods::Mods::default();
            if let Some(v) = self.get_entry(trigger) {
                return Some(v);
            }
        }

        None
    }

    /// Remove a binding for a given trigger.
    pub fn remove(&mut self, t: Trigger) {
        // Removal always resets our chain parent. We could make this
        // finer grained but the way it is documented is that chaining
        // must happen directly after sets so this works.
        self.chain_parent = None;

        let Some(entry) = self.bindings.swap_remove(&FoldedTrigger(t)) else {
            return;
        };

        match entry {
            // For a leader removal, the child set is dropped. Leaders are
            // never part of reverse maps so no other accounting is needed.
            Value::Leader(_) => {}

            // For an action we need to fix up the reverse mapping.
            Value::Leaf(leaf) => self.fixup_reverse_for_action(&leaf.action, t),

            // Chained leaves are never in our reverse mapping so no
            // cleanup is required.
            Value::LeafChained(_) => {}
        }
    }

    /// Fix up the reverse mapping after removing an action.
    ///
    /// When an action is removed from a binding (either by removal or by
    /// converting to a chained action), we need to update the reverse mapping.
    /// If another binding has the same action, we update the reverse mapping
    /// to point to that binding. Otherwise, we remove the action from the
    /// reverse mapping entirely.
    ///
    /// The `old` parameter is the trigger that was previously bound to this
    /// action. It is used to check if the reverse mapping still points to
    /// this trigger; if not, no fixup is needed since the reverse map already
    /// points to a different trigger for this action.
    fn fixup_reverse_for_action(&mut self, action: &Action, old: Trigger) {
        let Some(entry) = self.reverse.get_mut(action) else {
            return;
        };

        // If our value is not the same as the old trigger, we can
        // ignore it because our reverse mapping points somewhere else.
        if !entry.equal(&old) {
            return;
        }

        // It is the same trigger, so let's now go through our bindings
        // and try to find another trigger that maps to the same action.
        let action_hash = action.hash();
        for (k, v) in &self.bindings {
            if let Value::Leaf(leaf_search) = v
                && leaf_search.action.hash() == action_hash
            {
                *entry = k.0;
                return;
            }
        }

        // No other trigger points to this action so we remove
        // the reverse mapping completely.
        self.reverse.swap_remove(action);
    }
}

fn prepend(t: Trigger, mut p: Vec<Trigger>) -> Vec<Trigger> {
    p.insert(0, t);
    p
}
