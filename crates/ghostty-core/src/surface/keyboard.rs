//! Keyboard input for a surface (Zig `Surface.zig`: `keyEventIsBinding`,
//! `keyCallback`, `maybeHandleBinding`, key sequences/tables, `encodeKey`,
//! the key-table binding actions and the key release on focus loss).
//!
//! # Deviations from Zig
//!
//! Zig keeps raw pointers into the config (`*const Binding.Set`) for the
//! current sequence and the key table stack. Here the sequence is stored as
//! a *path* (where it started plus the triggers pressed so far) and tables
//! are stored by name; both are resolved against the current config each
//! time. A config change drops them anyway (`update_config` calls
//! [`Surface::end_key_sequence`] and [`Surface::deactivate_all_key_tables`]).

use ghostty_apprt::{
    action::{Action as RtAction, KeySequence, KeyTable},
    runtime_api::{CoreError, InputEffect},
};
use ghostty_input::{
    Action as KeyAction,
    Key,
    KeyEvent,
    Mods,
    binding::{Action as BindingAction, Flags, Scope, Set, Trigger, TriggerKey, set::Value},
    key_encode,
};
use ghostty_inspector as inspector;
use ghostty_os::osbytes::EncodedOs;
use ghostty_terminal::terminal::{MouseEvents, ScrollViewport, Terminal};
use ghostty_termio::message::Message as IoMessage;
use ghostty_vt::{modes::Mode, mouse_shape::MouseShape};

use super::{Surface, config::DerivedConfig};

/// The maximum number of key tables that can be active at any given time.
/// `activate_key_table` calls after this are ignored.
pub const MAX_ACTIVE_KEY_TABLES: usize = 8;

/// Where a sequence (or a lookup) is rooted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetRoot {
    /// The root keybinding set.
    Root,
    /// A named key table.
    Table(EncodedOs),
}

/// The currently active key sequence: the set it started in plus the
/// triggers (leader keys) pressed so far.
#[derive(Debug, Clone, PartialEq)]
pub struct SequencePath {
    pub root: SetRoot,
    pub path: Vec<Trigger>,
}

/// An entry of the key table stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableEntry {
    pub name: EncodedOs,
    pub once: bool,
}

/// Keyboard state for the surface (Zig: `Surface.Keyboard`).
#[derive(Default)]
pub struct Keyboard {
    /// The currently active key sequence for the surface. If this is `None`
    /// then we're not currently in a key sequence.
    pub sequence: Option<SequencePath>,

    /// The queued keys when we're in the middle of a sequenced binding.
    /// These are flushed when the sequence is completed and unconsumed or
    /// invalid. Naturally bounded by the configuration maximum sequence
    /// length.
    pub sequence_queued: Vec<IoMessage>,

    /// The stack of tables that is currently active. The first value in this
    /// is the first activated table (NOT the default keybinding set).
    /// Bounded by [`MAX_ACTIVE_KEY_TABLES`].
    pub table_stack: Vec<TableEntry>,

    /// The hash value of the last keybinding trigger that we performed. Only
    /// set if the last key input matched a keybinding, consumed it, and
    /// performed it. Used to prevent sending release/repeat events for
    /// handled bindings.
    pub last_trigger: Option<u64>,
}

/// An owned copy of a [`KeyEvent`] (the `utf8` text is owned). Used for the
/// "currently pressed key" so a release can be sent on focus loss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedKeyEvent {
    pub action: KeyAction,
    pub key: Key,
    pub mods: Mods,
    pub consumed_mods: Mods,
    pub composing: bool,
    pub utf8: Vec<u8>,
    pub unshifted_codepoint: u32,
}

impl Default for OwnedKeyEvent {
    fn default() -> Self {
        Self::from_event(&KeyEvent::default())
    }
}

impl OwnedKeyEvent {
    /// Copy an event.
    #[must_use]
    pub fn from_event(e: &KeyEvent<'_>) -> Self {
        OwnedKeyEvent {
            action: e.action,
            key: e.key,
            mods: e.mods,
            consumed_mods: e.consumed_mods,
            composing: e.composing,
            utf8: e.utf8.to_vec(),
            unshifted_codepoint: e.unshifted_codepoint,
        }
    }

    /// Borrow as a [`KeyEvent`].
    #[must_use]
    pub fn as_event(&self) -> KeyEvent<'_> {
        KeyEvent {
            action: self.action,
            key: self.key,
            mods: self.mods,
            consumed_mods: self.consumed_mods,
            composing: self.composing,
            utf8: &self.utf8,
            unshifted_codepoint: self.unshifted_codepoint,
        }
    }
}

/// What to do with queued sequence inputs (Zig: `KeySequenceQueued`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceAction {
    Drop,
    Flush,
}

/// Memory handling of the queue afterwards (Zig: `KeySequenceMemory`). Both
/// release the queued entries in Rust; kept for parity with the Zig API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceFree {
    Free,
    Retain,
}

/// `key_encode.Options.fromTerminal`. The macOS option-as-alt value can't be
/// known from the terminal state.
#[must_use]
pub fn encode_options_from_terminal(t: &Terminal) -> key_encode::Options {
    key_encode::Options {
        alt_esc_prefix: t.modes.get(Mode::AltEscPrefix),
        cursor_key_application: t.modes.get(Mode::CursorKeys),
        keypad_key_application: t.modes.get(Mode::KeypadKeys),
        ignore_keypad_with_numlock: t.modes.get(Mode::IgnoreKeypadWithNumlock),
        modify_other_keys_state_2: t.flags.modify_other_keys_2,
        kitty_flags: t.screens.active().kitty_keyboard.current(),
        macos_option_as_alt: ghostty_input::OptionAsAlt::False,
    }
}

/// Resolve a set by root (Zig pointer) against a config.
fn root_set<'a>(cfg: &'a DerivedConfig, root: &SetRoot) -> Option<&'a Set> {
    match root {
        SetRoot::Root => Some(&cfg.keybind.set),
        SetRoot::Table(name) => cfg
            .keybind
            .tables
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, s)| s),
    }
}

/// Resolve the set a sequence path points at.
fn sequence_set<'a>(cfg: &'a DerivedConfig, seq: &SequencePath) -> Option<&'a Set> {
    let mut set = root_set(cfg, &seq.root)?;
    for trigger in &seq.path {
        match set.get_entry(*trigger)?.1 {
            Value::Leader(next) => set = next,
            _ => return None,
        }
    }
    Some(set)
}

/// What a lookup found, with everything borrowed copied out.
enum Found {
    /// A leader key: the new sequence path.
    Leader(Trigger, SequencePath),
    /// A leaf (possibly chained).
    Leaf {
        actions: Vec<BindingAction>,
        flags: Flags,
    },
}

fn found_from(trigger: Trigger, value: &Value, origin: SetRoot, parent_path: &[Trigger]) -> Found {
    match value {
        Value::Leader(_) => {
            let mut path = parent_path.to_vec();
            path.push(trigger);
            Found::Leader(trigger, SequencePath { root: origin, path })
        }
        Value::Leaf(l) => Found::Leaf {
            actions: vec![l.action.clone()],
            flags: l.flags,
        },
        Value::LeafChained(l) => Found::Leaf {
            actions: l.actions.clone(),
            flags: l.flags,
        },
    }
}

/// `SurfaceMouse.keyToMouseShape` (surface_mouse.zig), needed for the
/// cursor shape update when modifier keys change.
fn key_to_mouse_shape(
    physical_key: Key,
    mouse_event: MouseEvents,
    mods: Mods,
    over_link: bool,
    hidden: bool,
) -> Option<MouseShape> {
    let eligible = physical_key.ctrl_or_super()
        || physical_key.left_or_right_shift()
        || physical_key.left_or_right_alt();
    if !eligible {
        return None;
    }

    // Link hover or hidden state overrides any other shape processing.
    if over_link || hidden {
        return None;
    }

    let rect_select = mods.ctrl_or_super() && mods.alt;
    if mouse_event == MouseEvents::None {
        // Default terminal mode: text
        Some(if rect_select {
            MouseShape::Crosshair
        } else {
            MouseShape::Text
        })
    } else if mods.shift && rect_select {
        // Mouse tracking mode, overriding with shift + rectangle select.
        Some(MouseShape::Crosshair)
    } else if mods.shift {
        Some(MouseShape::Text)
    } else {
        Some(MouseShape::Default)
    }
}

fn closing_action(action: &BindingAction) -> bool {
    matches!(
        action,
        BindingAction::CloseSurface | BindingAction::CloseWindow | BindingAction::CloseTab(_)
    )
}

impl Surface {
    /// Apply key remappings (Zig: at the top of `keyEventIsBinding` and
    /// `keyCallback`). Returns the event with remapped mods.
    fn remap_event<'a>(cfg: &DerivedConfig, event: &KeyEvent<'a>) -> KeyEvent<'a> {
        let mut event = event.clone();
        if cfg.key_remap.is_remapped(event.mods) {
            event.mods = cfg.key_remap.apply(event.mods);
        }
        event
    }

    /// Returns the flags of the binding the given key event would trigger,
    /// if any. This is useful for determining if a key event should be sent
    /// to the terminal or not.
    ///
    /// Note that this function does not check if the binding itself is
    /// performable, only if the key event would trigger a binding.
    pub fn key_event_is_binding(&self, event: &KeyEvent<'_>) -> Option<Flags> {
        let cfg = self.cfg();

        // Apply key remappings for consistency with key_callback
        let event = Self::remap_event(&cfg, event);

        match event.action {
            KeyAction::Release => return None,
            KeyAction::Press | KeyAction::Repeat => {}
        }

        let (seq, tables) = {
            let kb = self.keyboard.borrow();
            (kb.sequence.clone(), kb.table_stack.clone())
        };

        let value = 'entry: {
            // If we're in a sequence, check the sequence set
            if let Some(seq) = &seq {
                let set = sequence_set(&cfg, seq)?;
                break 'entry set.get_event(&event)?.1;
            }

            // Check active key tables (inner-most to outer-most)
            for t in tables.iter().rev() {
                if let Some(set) = root_set(&cfg, &SetRoot::Table(t.name.clone()))
                    && let Some((_, v)) = set.get_event(&event)
                {
                    break 'entry v;
                }
            }

            // Check the root set
            cfg.keybind.set.get_event(&event)?.1
        };

        Some(match value {
            Value::Leader(_) => Flags {
                // Zig: `.{}` which are the struct defaults.
                ..Flags::default()
            },
            Value::Leaf(l) => l.flags,
            Value::LeafChained(l) => l.flags,
        })
    }

    /// Called for any key events. This handles keybindings, encoding and
    /// sending to the terminal, etc.
    #[allow(clippy::needless_pass_by_value)]
    pub fn key_callback(&self, event_orig: KeyEvent<'_>) -> Result<InputEffect, CoreError> {
        let cfg = self.cfg();

        // Apply key remappings to transform modifiers before any
        // processing. This allows users to remap modifier keys at the app
        // level.
        let event = Self::remap_event(&cfg, &event_orig);

        // Setup our inspector event if we have an inspector.
        let mut insp_ev = self
            .inspector
            .borrow()
            .as_ref()
            .map(|_| inspector::KeyEvent::new(&event));

        let result = self.key_callback_remapped(&cfg, &event, insp_ev.as_mut());

        // When we're done processing, we always want to add the event to
        // the inspector. We have to check for the inspector again because
        // our keybinding might have closed it.
        if let Some(ev) = insp_ev
            && let Some(insp) = self.inspector()
        {
            insp.lock().record_key_event(ev);
            self.queue_render();
        }

        result
    }

    /// The remainder of [`Surface::key_callback`] after the key remapping and
    /// the inspector event setup.
    fn key_callback_remapped(
        &self,
        cfg: &DerivedConfig,
        event: &KeyEvent<'_>,
        mut insp_ev: Option<&mut inspector::KeyEvent>,
    ) -> Result<InputEffect, CoreError> {
        // Handle keybindings first. We need to handle this on all events
        // (press, repeat, release) because a press may perform a binding but
        // a release should not encode if we consumed the press.
        if let Some(v) = self.maybe_handle_binding(cfg, event, insp_ev.as_deref_mut()) {
            return Ok(v);
        }

        // If we allow KAM and KAM is enabled then we do nothing.
        if cfg.vt_kam_allowed && self.lock_terminal().modes.get(Mode::DisableKeyboard) {
            return Ok(InputEffect::Consumed);
        }

        // If this input event has text, then we hide the mouse if
        // configured. We only do this on pressed events to avoid hiding the
        // mouse when we change focus due to a keybinding (i.e. switching
        // tabs).
        if cfg.mouse_hide_while_typing
            && event.action == KeyAction::Press
            && !self.mouse.borrow().hidden
            && !event.utf8.is_empty()
        {
            self.hide_mouse();
        }

        // If our mouse modifiers change we may need to change our link
        // highlight state.
        let mouse_mods = self.mouse.borrow().mods;
        if !mouse_mods.equal(event.mods) {
            self.refresh_mouse_for_mods_change(event);
        }

        // Process the cursor state logic. This will update the cursor shape
        // if needed, depending on the key state.
        let shape = {
            let mouse_event = self.lock_terminal().flags.mouse_event;
            let m = self.mouse.borrow();
            key_to_mouse_shape(event.key, mouse_event, m.mods, m.over_link, m.hidden)
        };
        if let Some(shape) = shape {
            self.perform(RtAction::MouseShape(shape));
        }

        // We've processed a key event that produced some data so we want to
        // track the last pressed key.
        {
            let mut pressed = self.pressed_key.borrow_mut();
            let prev = pressed.take();
            *pressed = 'event: {
                // We need to unset the allocated fields that will become
                // invalid
                let mut copy = OwnedKeyEvent::from_event(event);
                copy.utf8.clear();

                // If we have a previous pressed key and we're releasing it
                // then we set it to invalid to prevent repeating the
                // release event.
                if event.action == KeyAction::Release {
                    // if we didn't have a previous event and this is a
                    // release event then we just want to set it to null.
                    let Some(prev) = prev else {
                        break 'event None;
                    };
                    if prev.key == copy.key {
                        copy.key = Key::Unidentified;
                    }
                }

                // If our key is invalid and we have no mods, then we're
                // done! This helps catch the state that we naturally
                // released all keys.
                if copy.key == Key::Unidentified && copy.mods.empty() {
                    break 'event None;
                }

                Some(copy)
            };
        }

        // Encode and send our key. If we didn't encode anything, then we
        // return the effect as ignored.
        let Some(write_req) = self.encode_key(event, insp_ev) else {
            // No valid request means that we didn't encode anything.
            return Ok(InputEffect::Ignored);
        };

        // If our process is exited and we press a key that results in an
        // encoded value, we close the surface. We want to eventually move
        // this behavior to the apprt probably.
        if self.child_exited.get() {
            self.close();
            return Ok(InputEffect::Closed);
        }

        self.queue_io(write_req);

        // If our event is any keypress that isn't a modifier and we
        // generated some data to send to the pty, then we move the viewport
        // down to the bottom. We also clear the selection for any key other
        // then modifiers.
        if !event.key.modifier() {
            {
                let mut t = self.lock_terminal();
                if cfg.selection_clear_on_typing || event.key == Key::Escape {
                    let _ = self.set_selection(&mut t, None);
                }

                if cfg.scroll_to_bottom.keystroke {
                    t.scroll_viewport(ScrollViewport::Bottom);
                }
            }
            self.queue_render();
        }

        Ok(InputEffect::Consumed)
    }

    /// The modifier-change block of `keyCallback` (`mouse_mods` label).
    fn refresh_mouse_for_mods_change(&self, event: &KeyEvent<'_>) {
        // Update our modifiers, this will update mouse mods too
        self.mods_changed(event.mods);

        let (mouse_event, mouse_shape) = {
            let t = self.lock_terminal();
            (t.flags.mouse_event, t.mouse_shape)
        };
        let shift = self.mouse.borrow().mods.shift;

        // We only refresh links if
        // 1. mouse reporting is off
        // OR
        // 2. mouse reporting is on and we are not reporting shift to the
        //    terminal
        if mouse_event == MouseEvents::None
            || (shift && !self.mouse_shift_capture(&self.lock_terminal()))
        {
            // Refresh our link state
            let pos = self.rt_surface.get_cursor_pos();
            let over_link = self.mouse.borrow().over_link;
            let vp = self.pos_to_viewport(f64::from(pos.x), f64::from(pos.y));
            if let Err(err) = self.mouse_refresh_links(pos, vp, over_link) {
                log::warn!("failed to refresh links err={err}");
            }
        } else if mouse_event != MouseEvents::None && !shift {
            // If we have mouse reports on and we don't have shift pressed,
            // we reset state
            self.perform(RtAction::MouseShape(mouse_shape));
            self.perform(RtAction::MouseOverLink(
                ghostty_apprt::action::MouseOverLink { url: String::new() },
            ));
            self.queue_render();
        }
    }

    /// Maybe handles a binding for a given event and if so returns the
    /// effect. Returns `None` if the event is not handled in any way and
    /// processing should continue.
    fn maybe_handle_binding(
        &self,
        cfg: &DerivedConfig,
        event: &KeyEvent<'_>,
        mut insp_ev: Option<&mut inspector::KeyEvent>,
    ) -> Option<InputEffect> {
        match event.action {
            // Release events never trigger a binding but we need to check
            // if we consumed the press event so we don't encode the
            // release.
            KeyAction::Release => {
                if let Some(last) = self.keyboard.borrow().last_trigger
                    && last == event.binding_hash()
                {
                    // We don't reset the last trigger on release because an
                    // apprt may send multiple release events for a single
                    // press event.
                    return Some(InputEffect::Consumed);
                }

                return None;
            }

            // Carry on processing.
            KeyAction::Press | KeyAction::Repeat => {}
        }

        let (seq, tables) = {
            let kb = self.keyboard.borrow();
            (kb.sequence.clone(), kb.table_stack.clone())
        };

        // Find an entry in the keybind set that matches our event.
        let found = 'entry: {
            // Handle key sequences first.
            if let Some(seq) = &seq {
                // Get our entry from the set for the given event.
                if let Some(set) = sequence_set(cfg, seq)
                    && let Some((t, v)) = set.get_event(event)
                {
                    break 'entry found_from(*t, v, seq.root.clone(), &seq.path);
                }

                // No entry found. We need to encode everything up to this
                // point and send to the pty since we're in a sequence.

                // We ignore modifiers so that nested sequences such as
                // ctrl+a>ctrl+b>c work.
                if event.key.modifier() {
                    return None;
                }

                // If we have a catch-all of ignore, then we special case
                // our invalid sequence handling to ignore it.
                if self.catch_all_is_ignore(cfg) {
                    self.end_key_sequence(SequenceAction::Drop, SequenceFree::Retain);
                    return Some(InputEffect::Ignored);
                }

                // Encode everything up to this point
                self.end_key_sequence(SequenceAction::Flush, SequenceFree::Retain);

                return None;
            }

            // No currently active sequence, move on to tables. For tables,
            // we search inner-most table to outer-most. The table stack
            // does NOT include the root set.
            for (i, table) in tables.iter().rev().enumerate() {
                let root = SetRoot::Table(table.name.clone());
                if let Some(set) = root_set(cfg, &root)
                    && let Some((t, v)) = set.get_event(event)
                {
                    // If this is a one-shot activation AND its the
                    // currently active table, then we deactivate it after
                    // this. Note: we may want to change the semantics here
                    // to remove this table no matter where it is in the
                    // stack, maybe.
                    let f = found_from(*t, v, root, &[]);
                    if table.once && i == 0 {
                        self.deactivate_key_table();
                    }
                    break 'entry f;
                }
            }

            // No table, use our default set
            let (t, v) = cfg.keybind.set.get_event(event)?;
            found_from(*t, v, SetRoot::Root, &[])
        };

        // Determine if this entry has an action or if its a leader key.
        let (actions, flags) = match found {
            Found::Leader(trigger, path) => {
                // Setup the next set we'll look at.
                self.keyboard.borrow_mut().sequence = Some(path);

                // Store this event so that we can drain and encode on
                // invalid. We don't need to cap this because it is
                // naturally capped by the config validation.
                if let Some(req) = self.encode_key(event, insp_ev.as_deref_mut()) {
                    self.keyboard.borrow_mut().sequence_queued.push(req);
                }

                // Start or continue our key sequence
                self.perform(RtAction::KeySequence(KeySequence::Trigger(trigger)));

                return Some(InputEffect::Consumed);
            }
            Found::Leaf { actions, flags } => (actions, flags),
        };

        // consumed determines if the input is consumed or if we continue
        // encoding the key (if we have a key to encode).
        //
        // If the consumed flag is explicitly set, then we are consumed. If
        // the global or all flag is set, we always consume.
        let consumed = flags.consumed || flags.global || flags.all;

        // We have an action, so at this point we're handling SOMETHING so we
        // reset the last trigger to null. We only set this if we actually
        // perform an action (below). An action also always resets the
        // sequence set.
        {
            let mut kb = self.keyboard.borrow_mut();
            kb.last_trigger = None;
            kb.sequence = None;
        }

        // Attempt to perform the action
        log::debug!("key event binding flags={flags:?} action={actions:?}");
        let performed = 'performed: {
            // If this is a global or all action, then we perform it on the
            // app and it applies to every surface.
            if flags.global || flags.all {
                self.perform_all_chained_action(&actions);

                // "All" actions are always performed since they are global.
                break 'performed true;
            }

            // Perform each action. We are performed if ANY of the chained
            // actions perform.
            let mut performed = false;
            for action in &actions {
                match self.perform_binding_action(action) {
                    Ok(v) => performed = performed || v,
                    Err(err) => log::info!(
                        "key binding action failed action={} err={err}",
                        action.key().name()
                    ),
                }
            }

            performed
        };

        if performed {
            // If we performed an action and it was a closing action, our
            // "self" is not safe to use anymore so we need to just exit
            // immediately.
            if actions.iter().any(closing_action) {
                log::debug!("key binding is a closing binding, halting key event processing");
                return Some(InputEffect::Closed);
            }

            // If our action was "ignore" then we return the special input
            // effect of "ignored".
            if actions.iter().any(|a| matches!(a, BindingAction::Ignore)) {
                // If we're in a sequence, clear it.
                self.end_key_sequence(SequenceAction::Drop, SequenceFree::Retain);

                return Some(InputEffect::Ignored);
            }
        }

        // If we have the performable flag and the action was not performed,
        // then we act as though a binding didn't exist.
        if flags.performable && !performed {
            // If we're in a sequence, we treat this as if we pressed a key
            // that doesn't exist in the sequence. Reset our sequence and
            // flush any queued events.
            self.end_key_sequence(SequenceAction::Flush, SequenceFree::Retain);

            return None;
        }

        // If we consume this event, then we are done. If we don't consume
        // it, we processed the action but we still want to process our
        // encodings, too.
        if consumed {
            // If we had queued events, we deinit them since we consumed
            self.end_key_sequence(SequenceAction::Drop, SequenceFree::Retain);

            // Store our last trigger so we don't encode the release event
            self.keyboard.borrow_mut().last_trigger = Some(event.binding_hash());

            if let Some(ev) = insp_ev {
                ev.binding = actions;
            }
            return Some(InputEffect::Consumed);
        }

        // If we didn't perform OR we didn't consume, then we want to encode
        // any queued events for a sequence.
        self.end_key_sequence(SequenceAction::Flush, SequenceFree::Retain);

        None
    }

    /// Perform actions on the app and every surface (Zig:
    /// `App.performAllChainedAction`; reimplemented here because the app
    /// handle isn't reachable from a surface).
    fn perform_all_chained_action(&self, actions: &[BindingAction]) {
        for action in actions {
            match action.scope() {
                Scope::App => self.perform_app_action(action),
                Scope::Surface => {
                    for surface in self.app.surfaces() {
                        if let Err(err) = surface.perform_binding_action(action) {
                            log::warn!(
                                "error performing chained action action={} err={err}",
                                action.key().name()
                            );
                        }
                    }
                }
            }
        }
    }

    /// `App.performAction` for app-scoped binding actions.
    fn perform_app_action(&self, action: &BindingAction) {
        use ghostty_apprt::action::{ReloadConfig, Target};
        let rt = match action {
            BindingAction::Quit => RtAction::Quit,
            BindingAction::NewWindow => RtAction::NewWindow,
            BindingAction::OpenConfig => RtAction::OpenConfig,
            BindingAction::ReloadConfig => RtAction::ReloadConfig(ReloadConfig::default()),
            BindingAction::CloseAllWindows => RtAction::CloseAllWindows,
            BindingAction::ToggleQuickTerminal => RtAction::ToggleQuickTerminal,
            BindingAction::ToggleVisibility => RtAction::ToggleVisibility,
            BindingAction::CheckForUpdates => RtAction::CheckForUpdates,
            BindingAction::ShowGtkInspector => RtAction::ShowGtkInspector,
            BindingAction::Undo => RtAction::Undo,
            BindingAction::Redo => RtAction::Redo,
            _ => return,
        };
        // Zig `App.performAction`: new_window is created with parent = null,
        // so it never inherits from the invoking surface.
        self.rt_app.perform_action(Target::App, rt);
    }

    /// Deactivate every key table. Returns whether any was active.
    pub(crate) fn deactivate_all_key_tables(&self) -> bool {
        {
            let mut kb = self.keyboard.borrow_mut();
            // No key table active. This does nothing.
            if kb.table_stack.is_empty() {
                return false;
            }

            // Clear the entire table stack.
            kb.table_stack.clear();
        }

        // Notify the UI.
        self.perform(RtAction::KeyTable(KeyTable::DeactivateAll));
        true
    }

    /// Activate a key table by name (Zig: `activate_key_table[_once]`).
    /// Returns whether the table was activated.
    #[allow(dead_code, clippy::unnecessary_wraps)] // used by the binding-actions module
    pub(crate) fn activate_key_table(
        &self,
        name: &EncodedOs,
        once: bool,
    ) -> Result<bool, CoreError> {
        let cfg = self.cfg();

        // Look up the table in our config
        if root_set(&cfg, &SetRoot::Table(name.to_owned())).is_none() {
            log::debug!("key table not found: {name:?}");
            return Ok(false);
        }

        {
            let mut kb = self.keyboard.borrow_mut();

            // If this is the same table as is currently active, then do
            // nothing.
            if kb.table_stack.last().is_some_and(|t| &t.name == name) {
                log::debug!("ignoring duplicate activate table: {name:?}");
                return Ok(false);
            }

            // If we're already at the max, ignore it.
            if kb.table_stack.len() >= MAX_ACTIVE_KEY_TABLES {
                log::info!("ignoring activate table, max depth reached: {name:?}");
                return Ok(false);
            }

            // Add the table to the stack.
            kb.table_stack.push(TableEntry {
                name: name.to_owned(),
                once,
            });
        }

        // Notify the UI.
        self.perform(RtAction::KeyTable(KeyTable::Activate(name.to_owned())));

        log::debug!("key table activated: {name:?}");
        Ok(true)
    }

    /// Deactivate the inner-most key table (Zig: `deactivate_key_table`).
    /// Returns whether a table was active.
    pub(crate) fn deactivate_key_table(&self) -> bool {
        {
            let mut kb = self.keyboard.borrow_mut();
            // No key table active. This does nothing.
            if kb.table_stack.pop().is_none() {
                return false;
            }
        }

        // Notify the UI.
        self.perform(RtAction::KeyTable(KeyTable::Deactivate));
        true
    }

    /// This checks if the current keybinding sets have a catch_all binding
    /// with `ignore`. This is used to determine some special input cases.
    fn catch_all_is_ignore(&self, cfg: &DerivedConfig) -> bool {
        let trigger = Trigger {
            key: TriggerKey::CatchAll,
            mods: Mods::default(),
        };

        let tables = self.keyboard.borrow().table_stack.clone();
        let mut value = None;
        for t in tables.iter().rev() {
            if let Some(set) = root_set(cfg, &SetRoot::Table(t.name.clone()))
                && let Some(v) = set.get(trigger)
            {
                value = Some(v);
                break;
            }
        }
        let Some(value) = value.or_else(|| cfg.keybind.set.get(trigger)) else {
            return false;
        };

        // We have a catch-all entry, see if its an ignore
        match value {
            Value::Leader(_) => false,
            Value::Leaf(l) => matches!(l.action, BindingAction::Ignore),
            Value::LeafChained(l) => l.actions.iter().any(|a| matches!(a, BindingAction::Ignore)),
        }
    }

    /// End a key sequence. Safe to call if no key sequence is active.
    ///
    /// `action` and `free` determine the behavior of the queued inputs up
    /// to this point.
    pub(crate) fn end_key_sequence(&self, action: SequenceAction, _free: SequenceFree) {
        // Notify apprt key sequence ended
        self.perform(RtAction::KeySequence(KeySequence::End));

        // No matter what we clear our current sequence set. This restores
        // the set we look at to the root set.
        let queued = {
            let mut kb = self.keyboard.borrow_mut();
            kb.sequence = None;
            std::mem::take(&mut kb.sequence_queued)
        };

        // If we have no queued data, there is nothing else to do. Run the
        // proper action otherwise.
        if action == SequenceAction::Flush {
            for req in queued {
                self.queue_io(req);
            }
        }
    }

    /// Build the key encoding options (Zig: `encodeKeyOpts`). On macOS the
    /// `macos-option-as-alt` value comes from the config, or is detected from
    /// the keyboard layout when unset.
    fn encode_key_opts(&self) -> key_encode::Options {
        let mut opts = {
            let t = self.lock_terminal();
            encode_options_from_terminal(&t)
        };
        if !cfg!(target_os = "macos") {
            return opts;
        }

        opts.macos_option_as_alt = self.cfg().macos_option_as_alt.unwrap_or_else(|| {
            // If we don't have alt pressed, it doesn't matter what this
            // config is so we can just say "false" and avoid more expensive
            // checks below.
            if !self.mouse.borrow().mods.alt {
                return ghostty_input::OptionAsAlt::False;
            }

            // Alt is pressed, we're on macOS. We break some encapsulation
            // here and assume libghostty for ease...
            self.rt_app.keyboard_layout().detect_option_as_alt()
        });
        opts
    }

    /// Encodes the key event into a write request. Returns `None` if
    /// nothing was encoded.
    fn encode_key(
        &self,
        event: &KeyEvent<'_>,
        insp_ev: Option<&mut inspector::KeyEvent>,
    ) -> Option<IoMessage> {
        // Build our encoding options, which requires the lock.
        let opts = self.encode_key_opts();

        let mut data: Vec<u8> = Vec::with_capacity(64);
        if let Err(err) = key_encode::encode(&mut data, event, &opts) {
            log::warn!("error encoding key event err={err}");
            return None;
        }

        // Special-case: we did nothing.
        if data.is_empty() {
            return None;
        }

        // Copy the encoded data into the inspector event if we have one.
        if let Some(ev) = insp_ev {
            ev.pty.clone_from(&data);
        }

        Some(IoMessage::write_req(&data))
    }

    /// Called from `focus_callback` when focus is lost: if we have a
    /// pressed key, send a key release event for it (and for all
    /// modifiers). Depending on the apprt, this CAN result in duplicate key
    /// release events, but that is better than not sending a key release
    /// event at all.
    pub(crate) fn release_pressed_key_on_unfocus(&self) {
        let Some(mut pressed_key) = self.pressed_key.borrow_mut().take() else {
            return;
        };

        // All our actions will be releases
        pressed_key.action = KeyAction::Release;

        // Release the full key first
        if pressed_key.key != Key::Unidentified {
            let ev = pressed_key.as_event();
            match self.key_callback(ev) {
                Ok(InputEffect::Closed) => return,
                Ok(_) => {}
                Err(err) => log::warn!("error releasing key on focus loss err={err}"),
            }
        }

        // Release any modifiers if set
        if pressed_key.mods.empty() {
            return;
        }

        // Go through all the modifiers and if they're set, release both the
        // left and right sides of the modifier. This may not match the exact
        // input event but it ensures a full reset.
        let original_key = pressed_key.key;
        let release =
            |pk: &mut OwnedKeyEvent, which: fn(&mut Mods) -> &mut bool, keys: [Key; 2]| {
                if !*which(&mut pk.mods) {
                    return true;
                }
                *which(&mut pk.mods) = false;
                for key in keys {
                    pk.key = key;
                    if pk.key != original_key {
                        let ev = pk.as_event();
                        match self.key_callback(ev) {
                            Ok(InputEffect::Closed) => return false,
                            Ok(_) => {}
                            Err(err) => log::warn!("error releasing key on focus loss err={err}"),
                        }
                    }
                }
                true
            };
        if !release(
            &mut pressed_key,
            |m| &mut m.shift,
            [Key::ShiftRight, Key::ShiftLeft],
        ) {
            return;
        }
        if !release(
            &mut pressed_key,
            |m| &mut m.ctrl,
            [Key::ControlRight, Key::ControlLeft],
        ) {
            return;
        }
        if !release(
            &mut pressed_key,
            |m| &mut m.alt,
            [Key::AltRight, Key::AltLeft],
        ) {
            return;
        }
        release(
            &mut pressed_key,
            |m| &mut m.super_,
            [Key::MetaRight, Key::MetaLeft],
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use ghostty_config::{cli_args::ConfigField, config::Config};
    use ghostty_terminal::terminal::Options;

    use super::*;

    fn cfg_with(binds: &[&str]) -> DerivedConfig {
        let mut c = Config::default();
        for b in binds {
            c.keybind.parse_cli(Some(b)).unwrap();
        }
        DerivedConfig::new(std::sync::Arc::new(c)).unwrap()
    }

    #[test]
    fn owned_key_event_round_trip() {
        let e = KeyEvent {
            action: KeyAction::Press,
            key: Key::KeyA,
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
            utf8: "a".as_bytes(),
            unshifted_codepoint: 'a' as u32,
            ..KeyEvent::default()
        };
        let o = OwnedKeyEvent::from_event(&e);
        assert_eq!(o.as_event(), e);
    }

    #[test]
    fn closing_actions() {
        assert!(closing_action(&BindingAction::CloseSurface));
        assert!(closing_action(&BindingAction::CloseWindow));
        assert!(!closing_action(&BindingAction::Ignore));
    }

    #[test]
    fn sequence_path_resolution() {
        let cfg = cfg_with(&["ctrl+a>b=new_window", "ctrl+a>c>d=quit"]);
        let a = Trigger {
            key: TriggerKey::Unicode('a' as u32),
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
        };
        let seq = SequencePath {
            root: SetRoot::Root,
            path: vec![a],
        };
        let set = sequence_set(&cfg, &seq).expect("leader set");
        let b = Trigger {
            key: TriggerKey::Unicode('b' as u32),
            mods: Mods::default(),
        };
        assert!(matches!(set.get(b), Some(Value::Leaf(_))));
        let c = Trigger {
            key: TriggerKey::Unicode('c' as u32),
            mods: Mods::default(),
        };
        let seq2 = SequencePath {
            root: SetRoot::Root,
            path: vec![a, c],
        };
        assert!(sequence_set(&cfg, &seq2).is_some());
        // A path through a leaf is invalid
        let seq3 = SequencePath {
            root: SetRoot::Root,
            path: vec![a, b],
        };
        assert!(sequence_set(&cfg, &seq3).is_none());
    }

    #[test]
    fn root_set_tables() {
        let cfg = cfg_with(&["foo/x=new_window"]);
        assert!(root_set(&cfg, &SetRoot::Table("foo".into())).is_some());
        assert!(root_set(&cfg, &SetRoot::Table("bar".into())).is_none());
    }

    #[test]
    fn found_from_leader_extends_path() {
        let cfg = cfg_with(&["ctrl+a>b=new_window"]);
        let (t, v) = cfg
            .keybind
            .set
            .bindings
            .iter()
            .find(|(_, v)| matches!(v, Value::Leader(_)))
            .unwrap();
        match found_from(t.0, v, SetRoot::Root, &[]) {
            Found::Leader(_, p) => assert_eq!(p.path.len(), 1),
            Found::Leaf { .. } => panic!("expected leader"),
        }
    }

    #[test]
    fn encode_options_follow_terminal_modes() {
        let mut t = Terminal::new(Options::new(10, 5)).unwrap();
        let o = encode_options_from_terminal(&t);
        assert!(!o.cursor_key_application);
        t.modes.set(Mode::CursorKeys, true);
        t.modes.set(Mode::AltEscPrefix, false);
        t.flags.modify_other_keys_2 = true;
        let o = encode_options_from_terminal(&t);
        assert!(o.cursor_key_application);
        assert!(!o.alt_esc_prefix);
        assert!(o.modify_other_keys_state_2);
        assert_eq!(o.kitty_flags.int(), 0);
    }

    #[test]
    fn mouse_shape_for_keys() {
        let none = Mods::default();
        // No specific key pressed
        assert_eq!(
            key_to_mouse_shape(Key::Unidentified, MouseEvents::None, none, false, false),
            None
        );
        // Over a link
        assert_eq!(
            key_to_mouse_shape(Key::ShiftLeft, MouseEvents::None, none, true, false),
            None
        );
        // Hidden
        assert_eq!(
            key_to_mouse_shape(Key::ShiftLeft, MouseEvents::None, none, false, true),
            None
        );
        // Shift in mouse tracking mode -> text
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        assert_eq!(
            key_to_mouse_shape(Key::ShiftLeft, MouseEvents::Normal, shift, false, false),
            Some(MouseShape::Text)
        );
        // Released in tracking mode -> default
        assert_eq!(
            key_to_mouse_shape(Key::ShiftLeft, MouseEvents::Normal, none, false, false),
            Some(MouseShape::Default)
        );
        // ctrl+alt in text mode -> crosshair
        let ca = Mods {
            ctrl: true,
            alt: true,
            ..Mods::default()
        };
        assert_eq!(
            key_to_mouse_shape(Key::AltLeft, MouseEvents::None, ca, false, false),
            Some(MouseShape::Crosshair)
        );
        assert_eq!(
            key_to_mouse_shape(Key::AltLeft, MouseEvents::None, none, false, false),
            Some(MouseShape::Text)
        );
    }
}
