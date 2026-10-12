//! Keybinds (port of `Config.Keybinds` from `config/Config.zig`).

use std::fmt;

use ghostty_input::{
    Key,
    Mods,
    binding::{
        Action,
        AdjustSelection,
        CloseTabMode,
        CopyToClipboard,
        Flags,
        InspectorMode,
        NavigateSearch,
        Set,
        SplitDirection,
        SplitFocusDirection,
        SplitResizeDirection,
        SplitResizeParameter,
        Trigger,
        TriggerKey,
        WriteScreen,
        set::{EntryFormatter as BindingEntryFormatter, Value},
    },
};
use ghostty_os::osbytes::EncodedOs;

use crate::{
    cli_args::{ArgError, ConfigField},
    formatter::EntryFormatter,
};

/// The most recent binding target for `chain=` additions.
///
/// This is intentionally tracked at the Keybinds level so that chains can
/// apply across table boundaries according to parse order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ChainTarget {
    #[default]
    Root,
    Table(EncodedOs),
}

#[derive(Debug, Clone, Default)]
pub struct Keybinds {
    pub set: Set,

    /// Defined key tables. The default key table is always the root "set",
    /// which allows all table names to be available without reservation.
    /// Insertion ordered.
    pub tables: Vec<(EncodedOs, Set)>,

    /// The most recent binding target for `chain=` additions.
    pub chain_target: ChainTarget,
}

fn mods(ctrl: bool, shift: bool, alt: bool, super_: bool) -> Mods {
    Mods {
        ctrl,
        shift,
        alt,
        super_,
        ..Mods::default()
    }
}

fn uni(c: char, mods: Mods) -> Trigger {
    Trigger {
        key: TriggerKey::Unicode(c as u32),
        mods,
    }
}

fn phys(key: Key, mods: Mods) -> Trigger {
    Trigger {
        key: TriggerKey::Physical(key),
        mods,
    }
}

fn performable() -> Flags {
    Flags {
        performable: true,
        ..Flags::default()
    }
}

fn to_arg_error(e: ghostty_input::binding::Error) -> ArgError {
    match e {
        ghostty_input::binding::Error::InvalidFormat => ArgError::other("InvalidFormat"),
        ghostty_input::binding::Error::InvalidAction => ArgError::other("InvalidAction"),
    }
}

impl Keybinds {
    /// Reset to the default keybinds (Zig: `init`) for the build target.
    pub fn init(&mut self) {
        self.init_for(ghostty_input::platform::is_darwin());
    }

    /// Reset to the default keybinds for Darwin (`true`) or non-Darwin
    /// (`false`). Zig selects with `builtin.target.os.tag.isDarwin()` at
    /// compile time; the parameter lets both variants be tested anywhere.
    #[allow(clippy::too_many_lines)]
    pub fn init_for(&mut self, darwin: bool) {
        self.set = Set::default();
        self.tables = Vec::new();
        self.chain_target = ChainTarget::Root;

        let none = Mods::default();
        let cs = mods(true, true, false, false);
        // `inputpkg.ctrlOrSuper` for the requested platform.
        let cos = |m: Mods| ghostty_input::key::ctrl_or_super_for(m, darwin);
        let set = &mut self.set;

        // keybinds for opening and reloading config
        set.put(
            uni(',', cos(mods(false, true, false, false))),
            Action::ReloadConfig,
        );
        set.put(uni(',', cos(none)), Action::OpenConfig);

        {
            set.put(
                phys(Key::Copy, none),
                Action::CopyToClipboard(CopyToClipboard::Mixed),
            );
            set.put(phys(Key::Paste, none), Action::PasteFromClipboard);

            // On non-MacOS desktop envs (Windows, KDE, Gnome, Xfce), ctrl+insert is an
            // alt keybinding for Copy and shift+ins is an alt keybinding for Paste
            //
            // The order of these blocks is important. The *last* added keybind for a given action is
            // what will display in the menu. We want the more typical keybinds after this block to be
            // the standard
            if !darwin {
                set.put(
                    phys(Key::Insert, mods(true, false, false, false)),
                    Action::CopyToClipboard(CopyToClipboard::Mixed),
                );
                set.put(
                    phys(Key::Insert, mods(false, true, false, false)),
                    Action::PasteFromClipboard,
                );
            }

            // On macOS we default to super but Linux ctrl+shift since
            // ctrl+c is to kill the process.
            let m = if darwin {
                mods(false, false, false, true)
            } else {
                cs
            };
            set.put_flags(
                uni('c', m),
                Action::CopyToClipboard(CopyToClipboard::Mixed),
                performable(),
            );
            set.put_flags(uni('v', m), Action::PasteFromClipboard, performable());
        }

        // Increase font size mapping for keyboards with dedicated plus keys (like german)
        // Note: this order matters below because the C API will only return
        // the last keybinding for a given action. The macOS app uses this to
        // set the expected keybind for the menu.
        set.put(uni('=', cos(none)), Action::IncreaseFontSize(1.0));
        set.put(uni('+', cos(none)), Action::IncreaseFontSize(1.0));
        set.put(uni('-', cos(none)), Action::DecreaseFontSize(1.0));
        set.put(uni('0', cos(none)), Action::ResetFontSize);

        set.put(
            uni('j', mods(true, true, false, true)),
            Action::WriteScreenFile(WriteScreen::COPY),
        );
        set.put(
            uni('j', cos(mods(false, true, false, false))),
            Action::WriteScreenFile(WriteScreen::PASTE),
        );
        set.put(
            uni('j', cos(mods(false, true, true, false))),
            Action::WriteScreenFile(WriteScreen::OPEN),
        );

        // Expand Selection
        let shift = mods(false, true, false, false);
        for (k, a) in [
            (Key::ArrowLeft, AdjustSelection::Left),
            (Key::ArrowRight, AdjustSelection::Right),
            (Key::ArrowUp, AdjustSelection::Up),
            (Key::ArrowDown, AdjustSelection::Down),
            (Key::PageUp, AdjustSelection::PageUp),
            (Key::PageDown, AdjustSelection::PageDown),
            (Key::Home, AdjustSelection::Home),
            (Key::End, AdjustSelection::End),
        ] {
            set.put_flags(phys(k, shift), Action::AdjustSelection(a), performable());
        }

        // Tabs common to all platforms
        let ctrl = mods(true, false, false, false);
        set.put(phys(Key::Tab, cs), Action::PreviousTab);
        set.put(phys(Key::Tab, ctrl), Action::NextTab);

        // Windowing
        if !darwin {
            set.put(uni('n', cs), Action::NewWindow);
            set.put(uni('w', cs), Action::CloseSurface);
            set.put(uni('q', cs), Action::Quit);
            set.put(
                phys(Key::F4, mods(false, false, true, false)),
                Action::CloseWindow,
            );
            set.put(uni('t', cs), Action::NewTab);
            set.put(uni('w', cs), Action::CloseTab(CloseTabMode::This));
            set.put_flags(phys(Key::ArrowLeft, cs), Action::PreviousTab, performable());
            set.put_flags(phys(Key::ArrowRight, cs), Action::NextTab, performable());
            set.put_flags(phys(Key::PageUp, ctrl), Action::PreviousTab, performable());
            set.put_flags(phys(Key::PageDown, ctrl), Action::NextTab, performable());
            set.put(uni('o', cs), Action::NewSplit(SplitDirection::Right));
            set.put(uni('e', cs), Action::NewSplit(SplitDirection::Down));
            let ctrl_super = mods(true, false, false, true);
            set.put_flags(
                uni('[', ctrl_super),
                Action::GotoSplit(SplitFocusDirection::Previous),
                performable(),
            );
            set.put_flags(
                uni(']', ctrl_super),
                Action::GotoSplit(SplitFocusDirection::Next),
                performable(),
            );
            let ctrl_alt = mods(true, false, true, false);
            for (k, d) in [
                (Key::ArrowUp, SplitFocusDirection::Up),
                (Key::ArrowDown, SplitFocusDirection::Down),
                (Key::ArrowLeft, SplitFocusDirection::Left),
                (Key::ArrowRight, SplitFocusDirection::Right),
            ] {
                set.put_flags(phys(k, ctrl_alt), Action::GotoSplit(d), performable());
            }

            // Resizing splits
            let super_ctrl_shift = mods(true, true, false, true);
            for (k, d) in [
                (Key::ArrowUp, SplitResizeDirection::Up),
                (Key::ArrowDown, SplitResizeDirection::Down),
                (Key::ArrowLeft, SplitResizeDirection::Left),
                (Key::ArrowRight, SplitResizeDirection::Right),
            ] {
                set.put_flags(
                    phys(k, super_ctrl_shift),
                    Action::ResizeSplit(SplitResizeParameter(d, 10)),
                    performable(),
                );
            }

            // Viewport scrolling
            set.put(phys(Key::Home, shift), Action::ScrollToTop);
            set.put(phys(Key::End, shift), Action::ScrollToBottom);
            set.put(phys(Key::PageUp, shift), Action::ScrollPageUp);
            set.put(phys(Key::PageDown, shift), Action::ScrollPageDown);

            // Semantic prompts
            set.put(phys(Key::PageUp, cs), Action::JumpToPrompt(-1));
            set.put(phys(Key::PageDown, cs), Action::JumpToPrompt(1));

            // Search
            set.put_flags(uni('f', cs), Action::StartSearch, performable());
            set.put_flags(phys(Key::Escape, none), Action::EndSearch, performable());

            // Inspector, matching Chromium
            set.put(uni('i', cs), Action::Inspector(InspectorMode::Toggle));

            // Terminal
            set.put(uni('a', cs), Action::SelectAll);

            // Selection clipboard paste
            set.put(phys(Key::Insert, shift), Action::PasteFromSelection);
        }
        {
            // On macOS we default to super but everywhere else
            // is alt.
            let m = if darwin {
                mods(false, false, false, true)
            } else {
                mods(false, false, true, false)
            };

            // Cmd/Alt+N for goto tab N
            let digits = [
                Key::Digit1,
                Key::Digit2,
                Key::Digit3,
                Key::Digit4,
                Key::Digit5,
                Key::Digit6,
                Key::Digit7,
                Key::Digit8,
            ];
            // On macOS we keep these not performable so that the keyboard
            // shortcuts in tabs work. In the future the correct fix is to
            // fix the reverse mapping lookup to allow us to lookup
            // performable keybinds conditionally.
            let tab_flags = Flags {
                performable: !darwin,
                ..Flags::default()
            };
            for (i, key) in digits.into_iter().enumerate() {
                let n = i + 1;
                let c = char::from(b'0' + n as u8);
                // We register BOTH the physical `digit_N` key and the unicode
                // `N` key. This allows most keyboard layouts to work with
                // this shortcut. Namely, AZERTY doesn't produce unicode `N`
                // for their digit keys (they're on shifted keys on the same
                // physical keys).
                set.put_flags(phys(key, m), Action::GotoTab(n), tab_flags);
                // Important: this must be the LAST binding set so that the
                // libghostty trigger API returns this one for the action,
                // so that things like the macOS tab bar key equivalent label
                // work properly.
                set.put_flags(uni(c, m), Action::GotoTab(n), tab_flags);
            }
            // See comment above with the numeric goto_tab
            set.put_flags(uni('9', m), Action::LastTab, tab_flags);
        }

        // Toggle fullscreen
        set.put(phys(Key::Enter, cos(none)), Action::ToggleFullscreen);

        // Toggle zoom a split
        set.put(phys(Key::Enter, cos(shift)), Action::ToggleSplitZoom);

        // Toggle command palette, matches VSCode
        set.put(uni('p', cos(shift)), Action::ToggleCommandPalette);

        // Mac-specific keyboard bindings.
        if darwin {
            Self::init_mac_bindings(set);
        }
    }

    /// The Mac-specific default keybinds (tail of Zig `Keybinds.init`).
    #[allow(clippy::too_many_lines)]
    fn init_mac_bindings(set: &mut Set) {
        let sup = mods(false, false, false, true);
        let sup_shift = mods(false, true, false, true);
        let sup_alt = mods(false, false, true, true);
        let sup_ctrl = mods(true, false, false, true);

        set.put(uni('q', sup), Action::Quit);
        set.put_flags(uni('k', sup), Action::ClearScreen, performable());
        set.put(uni('a', sup), Action::SelectAll);

        // Undo/redo
        set.put_flags(uni('t', sup_shift), Action::Undo, performable());
        set.put_flags(uni('z', sup), Action::Undo, performable());
        set.put_flags(uni('z', sup_shift), Action::Redo, performable());

        // Viewport scrolling
        set.put(phys(Key::Home, sup), Action::ScrollToTop);
        set.put(phys(Key::End, sup), Action::ScrollToBottom);
        set.put(phys(Key::PageUp, sup), Action::ScrollPageUp);
        set.put(phys(Key::PageDown, sup), Action::ScrollPageDown);
        set.put_flags(uni('j', sup), Action::ScrollToSelection, performable());

        // Semantic prompts
        set.put(phys(Key::ArrowUp, sup_shift), Action::JumpToPrompt(-1));
        set.put(phys(Key::ArrowDown, sup_shift), Action::JumpToPrompt(1));

        // Mac windowing
        set.put(uni('n', sup), Action::NewWindow);
        set.put(uni('w', sup), Action::CloseSurface);
        set.put(uni('w', sup_alt), Action::CloseTab(CloseTabMode::This));
        set.put(uni('w', sup_shift), Action::CloseWindow);
        set.put(
            uni('w', mods(false, true, true, true)),
            Action::CloseAllWindows,
        );
        set.put(uni('t', sup), Action::NewTab);
        set.put(uni('[', sup_shift), Action::PreviousTab);
        set.put(uni(']', sup_shift), Action::NextTab);
        set.put(uni('d', sup), Action::NewSplit(SplitDirection::Right));
        set.put(uni('d', sup_shift), Action::NewSplit(SplitDirection::Down));
        set.put(
            uni('[', sup),
            Action::GotoSplit(SplitFocusDirection::Previous),
        );
        set.put(uni(']', sup), Action::GotoSplit(SplitFocusDirection::Next));
        for (k, d) in [
            (Key::ArrowUp, SplitFocusDirection::Up),
            (Key::ArrowDown, SplitFocusDirection::Down),
            (Key::ArrowLeft, SplitFocusDirection::Left),
            (Key::ArrowRight, SplitFocusDirection::Right),
        ] {
            set.put(phys(k, sup_alt), Action::GotoSplit(d));
        }
        for (k, d) in [
            (Key::ArrowUp, SplitResizeDirection::Up),
            (Key::ArrowDown, SplitResizeDirection::Down),
            (Key::ArrowLeft, SplitResizeDirection::Left),
            (Key::ArrowRight, SplitResizeDirection::Right),
        ] {
            set.put(
                phys(k, sup_ctrl),
                Action::ResizeSplit(SplitResizeParameter(d, 10)),
            );
        }
        set.put(uni('=', sup_ctrl), Action::EqualizeSplits);

        // Jump to prompt, matches Terminal.app
        set.put(phys(Key::ArrowUp, sup), Action::JumpToPrompt(-1));
        set.put(phys(Key::ArrowDown, sup), Action::JumpToPrompt(1));

        // Search
        set.put_flags(uni('f', sup), Action::StartSearch, performable());
        set.put_flags(uni('e', sup), Action::SearchSelection, performable());
        set.put_flags(uni('f', sup_shift), Action::EndSearch, performable());
        set.put_flags(
            phys(Key::Escape, Mods::default()),
            Action::EndSearch,
            performable(),
        );
        set.put_flags(
            uni('g', sup),
            Action::NavigateSearch(NavigateSearch::Next),
            performable(),
        );
        set.put_flags(
            uni('g', sup_shift),
            Action::NavigateSearch(NavigateSearch::Previous),
            performable(),
        );

        // Inspector, matching Chromium
        set.put(uni('i', sup_alt), Action::Inspector(InspectorMode::Toggle));

        // Alternate keybind, common to Mac programs
        set.put(uni('f', sup_ctrl), Action::ToggleFullscreen);

        // Selection clipboard paste, matches Terminal.app
        set.put(uni('v', sup_shift), Action::PasteFromSelection);

        // "Natural text editing" keybinds. This forces these keys to go back
        // to legacy encoding (not fixterms). It seems macOS users more than
        // others are used to these keys so we set them as defaults. If
        // people want to get back to the fixterm encoding they can set
        // the keybinds to `unbind`.
        set.put(phys(Key::ArrowRight, sup), Action::Text("\\x05".into()));
        set.put(phys(Key::ArrowLeft, sup), Action::Text("\\x01".into()));
        set.put(phys(Key::Backspace, sup), Action::Text("\\x15".into()));
        set.put(
            phys(Key::ArrowLeft, mods(false, false, true, false)),
            Action::Esc("b".into()),
        );
        set.put(
            phys(Key::ArrowRight, mods(false, false, true, false)),
            Action::Esc("f".into()),
        );
    }

    fn table_index(&self, name: &EncodedOs) -> Option<usize> {
        self.tables.iter().position(|(n, _)| n == name)
    }

    /// Get a table by name.
    #[must_use]
    pub fn table(&self, name: &str) -> Option<&Set> {
        self.tables
            .iter()
            .find(|(n, _)| n == &EncodedOs::from_plain_text(name))
            .map(|(_, s)| s)
    }

    /// Like format_entry but has an option to include docs.
    pub fn format_entry_docs(&self, f: &mut EntryFormatter<'_>, docs: bool) -> fmt::Result {
        if self.set.bindings.is_empty() && self.tables.is_empty() {
            return writeln!(f.writer, "{} = ", f.name);
        }

        // Format root set bindings
        for (k, v) in &self.set.bindings {
            if docs {
                // Zig looks up help by `@tagName(v)` of the set value
                // (leader/leaf/leaf_chained), which never matches a keybind
                // action name, so only the blank line is ever written.
                f.writer.write_str("\n")?;
            }

            let mut buf = ghostty_os::osbytes::valid_str_to_config_string(&k.0.to_string());
            emit(v, &mut buf, f)?;
        }

        // Format table bindings
        for (table_name, table_set) in &self.tables {
            for (k, v) in &table_set.bindings {
                let mut buf = format!(
                    "{}/{}",
                    table_name.as_encoded(),
                    ghostty_os::osbytes::valid_str_to_config_string(&k.0.to_string())
                );
                emit(v, &mut buf, f)?;
            }
        }
        Ok(())
    }
}

struct Adapter<'a, 'b> {
    f: &'a mut EntryFormatter<'b>,
    result: fmt::Result,
}

impl BindingEntryFormatter for Adapter<'_, '_> {
    fn format_entry(&mut self, value: &str) {
        if self.result.is_ok() {
            self.result = self
                .f
                .format_entry(&ghostty_os::osbytes::EncodedOs::from_encoded(value));
        }
    }
}

fn emit(v: &Value, buf: &mut String, f: &mut EntryFormatter<'_>) -> fmt::Result {
    let mut adapter = Adapter { f, result: Ok(()) };
    v.format_entries(buf, &mut adapter);
    adapter.result
}

fn equal_set(a: &Set, b: &Set) -> bool {
    // Two keybinds are considered equal if their primary bindings
    // are the same. We don't compare reverse mappings and such.
    if a.bindings.len() != b.bindings.len() {
        return false;
    }

    for (k, av) in &a.bindings {
        let Some(bv) = b.bindings.get(k) else {
            return false;
        };
        match (av, bv) {
            (Value::Leader(x), Value::Leader(y)) => {
                if !equal_set(x, y) {
                    return false;
                }
            }
            (Value::Leaf(x), Value::Leaf(y)) => {
                if x != y {
                    return false;
                }
            }
            (Value::LeafChained(x), Value::LeafChained(y)) => {
                if x.flags != y.flags || x.actions != y.actions {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

impl PartialEq for Keybinds {
    /// Compare if two of our value are equal (Zig: `equal`).
    fn eq(&self, other: &Self) -> bool {
        if !equal_set(&self.set, &other.set) {
            return false;
        }

        // Compare tables
        if self.tables.len() != other.tables.len() {
            return false;
        }
        for (name, set) in &self.tables {
            let Some(other_set) = other
                .tables
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, set)| set)
            else {
                return false;
            };
            if !equal_set(set, other_set) {
                return false;
            }
        }
        true
    }
}

impl ConfigField for Keybinds {
    fn parse_bytes(&mut self, input: Option<&[u8]>) -> Result<(), ArgError> {
        let value = input.ok_or(ArgError::ValueRequired)?;

        // Check for special values
        if value.is_empty() {
            log::info!("config has 'keybind =', using default keybinds");
            self.init();
            return Ok(());
        }

        if value == b"clear" {
            log::info!("config has 'keybind = clear', all keybinds cleared");
            self.set = Set::default();
            self.tables = Vec::new();
            self.chain_target = ChainTarget::Root;
            return Ok(());
        }

        // Check for table syntax: "name/" or "name/binding"
        // We look for '/' only before the first '=' to avoid matching
        // action arguments like "foo=text:/hello".
        let eq_idx = value.iter().position(|b| *b == b'=').unwrap_or(value.len());
        if let Some(slash_idx) = value[..eq_idx].iter().position(|b| *b == b'/') {
            let table_name = &value[..slash_idx];

            // Length zero is valid, so you can set `/=action` for the slash key.
            // Ignore '+', '>' because they can be part of sequences and
            // triggers. This lets things like `ctrl+/=action` work.
            if !table_name.is_empty() && !table_name.iter().any(|b| matches!(b, b'+' | b'>')) {
                let binding = &value[slash_idx + 1..];
                let table_name = EncodedOs::from_bytes(table_name);

                // Get or create the table
                let idx = if let Some(i) = self.table_index(&table_name) {
                    i
                } else {
                    self.tables.push((table_name.clone(), Set::default()));
                    self.tables.len() - 1
                };

                // If there's no binding after the slash, this is a table
                // definition/clear command
                if binding.is_empty() {
                    log::debug!("config has 'keybind = {table_name:?}/', table cleared");
                    self.tables[idx].1 = Set::default();
                    self.chain_target = ChainTarget::Root;
                    return Ok(());
                }

                // Chains are only allowed at the root level. Their target is
                // tracked globally by parse order in `self.chain_target`.
                if binding.starts_with(b"chain=") {
                    return Err(ArgError::other("InvalidFormat"));
                }

                // Parse and add the binding to the table
                self.tables[idx]
                    .1
                    .parse_and_put_bytes(binding)
                    .map_err(to_arg_error)?;
                self.chain_target = ChainTarget::Table(table_name.clone());
                return Ok(());
            }
        }

        if value.starts_with(b"chain=") {
            match self.chain_target.clone() {
                ChainTarget::Root => self.set.parse_and_put_bytes(value).map_err(to_arg_error)?,
                ChainTarget::Table(table_name) => {
                    let Some(idx) = self.table_index(&table_name) else {
                        self.chain_target = ChainTarget::Root;
                        return Err(ArgError::other("InvalidFormat"));
                    };
                    self.tables[idx]
                        .1
                        .parse_and_put_bytes(value)
                        .map_err(to_arg_error)?;
                }
            }
            return Ok(());
        }

        // Parse into default set
        self.set.parse_and_put_bytes(value).map_err(to_arg_error)?;
        self.chain_target = ChainTarget::Root;
        Ok(())
    }

    /// An empty value resets to the default keybinds (Zig: `init`).
    fn reset(&mut self, _default: Self) {
        self.init();
    }

    fn format_entry(&self, f: &mut EntryFormatter<'_>) -> fmt::Result {
        self.format_entry_docs(f, false)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn fmt_kb(name: &str, kb: &Keybinds) -> String {
        let mut s = String::new();
        crate::formatter::format_entry(name, kb, &mut s).unwrap();
        s
    }

    fn parsed(inputs: &[&str]) -> Keybinds {
        let mut kb = Keybinds::default();
        for i in inputs {
            kb.parse_cli(Some(i)).unwrap();
        }
        kb
    }

    #[test]
    fn parse_cli() {
        let mut set = Keybinds::default();
        set.parse_cli(Some("shift+a=copy_to_clipboard")).unwrap();
        set.parse_cli(Some("shift+a=csi:hello")).unwrap();
    }

    #[test]
    fn format_config_single() {
        let list = parsed(&["shift+a=csi:hello"]);
        assert_eq!(fmt_kb("a", &list), "a = shift+a=csi:hello\n");
    }

    // Regression test for https://github.com/ghostty-org/ghostty/issues/2734
    #[test]
    fn format_config_multiple_items() {
        let list = parsed(&["ctrl+z>1=goto_tab:1", "ctrl+z>2=goto_tab:2"]);
        // Note they turn into translated keys because they match
        // their ASCII mapping.
        assert_eq!(
            fmt_kb("keybind", &list),
            "keybind = ctrl+z>1=goto_tab:1\nkeybind = ctrl+z>2=goto_tab:2\n"
        );
    }

    #[test]
    fn format_config_multiple_items_nested() {
        let list = parsed(&[
            "ctrl+a>ctrl+b>n=new_window",
            "ctrl+a>ctrl+b>w=close_window",
            "ctrl+a>ctrl+c>t=new_tab",
            "ctrl+b>ctrl+d>a=previous_tab",
        ]);
        // NB: This does not currently retain the order of the keybinds.
        let want = "a = ctrl+a>ctrl+b>n=new_window\n\
                    a = ctrl+a>ctrl+b>w=close_window\n\
                    a = ctrl+a>ctrl+c>t=new_tab\n\
                    a = ctrl+b>ctrl+d>a=previous_tab\n";
        assert_eq!(fmt_kb("a", &list), want);
    }

    #[test]
    fn parse_cli_table_definition() {
        let kb = parsed(&["foo/shift+a=copy_to_clipboard"]);
        assert_eq!(kb.tables.len(), 1);
        assert!(kb.table("foo").is_some());
        assert_eq!(kb.table("foo").unwrap().bindings.len(), 1);
    }

    #[test]
    fn parse_cli_table_clear() {
        let mut kb = parsed(&["foo/shift+a=copy_to_clipboard"]);
        assert_eq!(kb.table("foo").unwrap().bindings.len(), 1);
        kb.parse_cli(Some("foo/")).unwrap();
        assert_eq!(kb.table("foo").unwrap().bindings.len(), 0);
    }

    #[test]
    fn parse_cli_table_multiple_bindings() {
        let kb = parsed(&[
            "foo/shift+a=copy_to_clipboard",
            "foo/shift+b=paste_from_clipboard",
            "bar/ctrl+c=close_window",
        ]);
        assert_eq!(kb.tables.len(), 2);
        assert_eq!(kb.table("foo").unwrap().bindings.len(), 2);
        assert_eq!(kb.table("bar").unwrap().bindings.len(), 1);
    }

    #[test]
    fn parse_cli_table_does_not_affect_root_set() {
        let kb = parsed(&[
            "shift+a=copy_to_clipboard",
            "foo/shift+b=paste_from_clipboard",
        ]);
        assert_eq!(kb.set.bindings.len(), 1);
        assert_eq!(kb.table("foo").unwrap().bindings.len(), 1);
    }

    #[test]
    fn parse_cli_table_empty_name_is_invalid() {
        let mut kb = Keybinds::default();
        assert_eq!(
            kb.parse_cli(Some("/shift+a=copy_to_clipboard")),
            Err(ArgError::other("InvalidFormat"))
        );
    }

    #[test]
    fn parse_cli_table_with_key_sequence() {
        let kb = parsed(&["foo/ctrl+a>ctrl+b=new_window"]);
        assert_eq!(kb.table("foo").unwrap().bindings.len(), 1);
    }

    fn assert_root_only(input: &str) {
        let kb = parsed(&[input]);
        assert_eq!(kb.set.bindings.len(), 1);
        assert_eq!(kb.tables.len(), 0);
    }

    #[test]
    fn parse_cli_slash_in_action_argument_is_not_a_table() {
        assert_root_only("ctrl+a=text:/hello");
    }

    #[test]
    fn parse_cli_slash_as_key_with_modifier_is_not_a_table() {
        assert_root_only("ctrl+/=text:foo");
    }

    #[test]
    fn parse_cli_shift_slash_as_key_is_not_a_table() {
        assert_root_only("shift+/=ignore");
    }

    #[test]
    fn parse_cli_bare_slash_as_key_is_not_a_table() {
        assert_root_only("/=text:foo");
    }

    #[test]
    fn parse_cli_slash_in_key_sequence_is_not_a_table() {
        assert_root_only("ctrl+a>ctrl+/=new_window");
    }

    #[test]
    fn parse_cli_table_with_slash_in_binding() {
        let kb = parsed(&["mytable//=text:foo"]);
        assert_eq!(kb.set.bindings.len(), 0);
        assert_eq!(kb.tables.len(), 1);
        assert_eq!(kb.table("mytable").unwrap().bindings.len(), 1);
    }

    #[test]
    fn parse_cli_table_with_sequence_containing_slash() {
        let kb = parsed(&["mytable/a>/=new_window"]);
        assert_eq!(kb.set.bindings.len(), 0);
        assert_eq!(kb.tables.len(), 1);
        assert!(kb.table("mytable").is_some());
    }

    #[test]
    fn parse_cli_chain_without_prior_parsed_binding_is_invalid() {
        let mut kb = Keybinds::default();
        assert_eq!(
            kb.parse_cli(Some("chain=new_tab")),
            Err(ArgError::other("InvalidFormat"))
        );
    }

    #[test]
    fn parse_cli_table_chain_syntax_is_invalid() {
        let mut kb = Keybinds::default();
        kb.parse_cli(Some("foo/a=text:hello")).unwrap();
        assert_eq!(
            kb.parse_cli(Some("foo/chain=deactivate_key_table")),
            Err(ArgError::other("InvalidFormat"))
        );
    }

    #[test]
    fn parse_cli_chain_applies_to_most_recent_table_binding() {
        let kb = parsed(&[
            "ctrl+n=activate_key_table:foo",
            "foo/a=text:hello",
            "chain=deactivate_key_table",
        ]);

        let root = kb
            .set
            .get(Trigger {
                mods: mods(true, false, false, false),
                key: TriggerKey::Unicode('n' as u32),
            })
            .unwrap();
        let leaf = root.as_leaf().unwrap();
        assert!(matches!(leaf.action, Action::ActivateKeyTable(_)));

        let foo = kb
            .table("foo")
            .unwrap()
            .get(Trigger {
                mods: Mods::default(),
                key: TriggerKey::Unicode('a' as u32),
            })
            .unwrap();
        let chained = foo.as_leaf_chained().unwrap();
        assert_eq!(chained.actions.len(), 2);
        assert!(matches!(chained.actions[0], Action::Text(_)));
        assert!(matches!(chained.actions[1], Action::DeactivateKeyTable));
    }

    #[test]
    fn clone_with_tables() {
        let kb = parsed(&[
            "shift+a=copy_to_clipboard",
            "foo/shift+b=paste_from_clipboard",
            "bar/ctrl+c=close_window",
        ]);
        let cloned = kb.clone();
        assert_eq!(kb.set.bindings.len(), cloned.set.bindings.len());
        assert_eq!(kb.tables.len(), cloned.tables.len());
        assert_eq!(
            kb.table("foo").unwrap().bindings.len(),
            cloned.table("foo").unwrap().bindings.len()
        );
        assert_eq!(
            kb.table("bar").unwrap().bindings.len(),
            cloned.table("bar").unwrap().bindings.len()
        );
        assert_eq!(kb, cloned);
    }

    #[test]
    fn equal_with_tables() {
        let a = parsed(&["foo/shift+a=copy_to_clipboard"]);
        let b = parsed(&["foo/shift+a=copy_to_clipboard"]);
        assert_eq!(a, b);
    }

    #[test]
    fn equal_with_tables_different_table_count() {
        let a = parsed(&["foo/shift+a=copy_to_clipboard"]);
        let b = parsed(&[
            "foo/shift+a=copy_to_clipboard",
            "bar/shift+b=paste_from_clipboard",
        ]);
        assert_ne!(a, b);
    }

    #[test]
    fn equal_with_tables_different_table_names() {
        let a = parsed(&["foo/shift+a=copy_to_clipboard"]);
        let b = parsed(&["bar/shift+a=copy_to_clipboard"]);
        assert_ne!(a, b);
    }

    #[test]
    fn equal_with_tables_different_bindings() {
        let a = parsed(&["foo/shift+a=copy_to_clipboard"]);
        let b = parsed(&["foo/shift+b=paste_from_clipboard"]);
        assert_ne!(a, b);
    }

    #[test]
    fn format_entry_with_tables() {
        let kb = parsed(&["foo/shift+a=csi:hello"]);
        assert_eq!(fmt_kb("keybind", &kb), "keybind = foo/shift+a=csi:hello\n");
    }

    #[test]
    fn format_entry_with_tables_and_root_set() {
        let kb = parsed(&["shift+b=csi:world", "foo/shift+a=csi:hello"]);
        let out = fmt_kb("keybind", &kb);
        assert!(out.contains("keybind = shift+b=csi:world\n"));
        assert!(out.contains("keybind = foo/shift+a=csi:hello\n"));
    }

    #[test]
    fn parse_cli_clear_clears_tables() {
        let mut kb = parsed(&[
            "shift+a=copy_to_clipboard",
            "foo/shift+b=paste_from_clipboard",
            "bar/ctrl+c=close_window",
        ]);
        assert_eq!(kb.set.bindings.len(), 1);
        assert_eq!(kb.tables.len(), 2);
        kb.parse_cli(Some("clear")).unwrap();
        assert_eq!(kb.set.bindings.len(), 0);
        assert_eq!(kb.tables.len(), 0);
    }

    #[test]
    fn parse_cli_reset_clears_tables() {
        let mut kb = parsed(&[
            "foo/shift+a=copy_to_clipboard",
            "bar/shift+b=paste_from_clipboard",
        ]);
        assert_eq!(kb.tables.len(), 2);
        kb.parse_cli(Some("")).unwrap();
        assert_eq!(kb.tables.len(), 0);
        assert!(!kb.set.bindings.is_empty());
    }

    #[test]
    fn reset_calls_init() {
        let mut kb = parsed(&["foo/shift+a=copy_to_clipboard"]);
        ConfigField::reset(&mut kb, Keybinds::default());
        assert_eq!(kb.tables.len(), 0);
        assert!(!kb.set.bindings.is_empty());
    }

    #[test]
    fn default_keybinds_format_roundtrip() {
        // Every default binding must format and re-parse to an equal set.
        let mut kb = Keybinds::default();
        kb.init();
        let out = fmt_kb("keybind", &kb);
        let mut again = Keybinds::default();
        for line in out.lines() {
            let v = line.strip_prefix("keybind = ").unwrap();
            again.parse_cli(Some(v)).unwrap();
        }
        assert_eq!(kb.set.bindings.len(), again.set.bindings.len());
    }

    fn lookup(kb: &Keybinds, trigger: Trigger) -> Option<&Value> {
        kb.set.get(trigger)
    }

    #[test]
    fn default_keybinds_darwin_use_super() {
        let mut kb = Keybinds::default();
        kb.init_for(true);
        let sup = mods(false, false, false, true);

        // cmd+c copies (performable), cmd+q quits, cmd+t opens a tab.
        for (c, want) in [
            ('c', Action::CopyToClipboard(CopyToClipboard::Mixed)),
            ('q', Action::Quit),
            ('t', Action::NewTab),
            ('1', Action::GotoTab(1)),
        ] {
            let Some(Value::Leaf(l)) = lookup(&kb, uni(c, sup)) else {
                panic!("missing super+{c}");
            };
            assert_eq!(l.action, want);
        }

        // tab shortcuts are NOT performable on macOS.
        let Some(Value::Leaf(l)) = lookup(&kb, uni('1', sup)) else {
            panic!()
        };
        assert!(!l.flags.performable);
        let Some(Value::Leaf(l)) = lookup(&kb, uni('c', sup)) else {
            panic!()
        };
        assert!(l.flags.performable);

        // No ctrl+shift+n window binding on macOS, no ctrl+insert copy.
        assert!(lookup(&kb, uni('n', mods(true, true, false, false))).is_none());
        assert!(lookup(&kb, phys(Key::Insert, mods(true, false, false, false))).is_none());

        // Natural text editing.
        let Some(Value::Leaf(l)) = lookup(&kb, phys(Key::ArrowLeft, sup)) else {
            panic!()
        };
        assert_eq!(l.action, Action::Text("\\x01".into()));

        // ctrlOrSuper picks super: super+comma opens the config.
        let Some(Value::Leaf(l)) = lookup(&kb, uni(',', sup)) else {
            panic!()
        };
        assert_eq!(l.action, Action::OpenConfig);

        // Formatted defaults re-parse to the same number of bindings.
        let out = fmt_kb("keybind", &kb);
        let mut again = Keybinds::default();
        for line in out.lines() {
            let v = line.strip_prefix("keybind = ").unwrap();
            again.parse_cli(Some(v)).unwrap();
        }
        assert_eq!(kb.set.bindings.len(), again.set.bindings.len());
    }

    #[test]
    fn default_keybinds_non_darwin_use_ctrl_shift() {
        let mut kb = Keybinds::default();
        kb.init_for(false);
        let cs = mods(true, true, false, false);
        let Some(Value::Leaf(l)) = lookup(&kb, uni('c', cs)) else {
            panic!()
        };
        assert_eq!(l.action, Action::CopyToClipboard(CopyToClipboard::Mixed));
        assert!(lookup(&kb, uni('q', mods(false, false, false, true))).is_none());
    }
}
