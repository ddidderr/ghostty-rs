//! `MainMenu.xib` and the menu bookkeeping of `AppDelegate.swift` as data.
//!
//! The xib is not translatable at build time on Linux (no `ibtool`), so
//! `scripts/gen-macos-menu-spec.py` reads it and emits [`generated`]: every
//! menu, item, key equivalent, modifier mask, action selector, target and
//! AppDelegate outlet. `app::main_menu` builds the real `NSMenu` tree from
//! these tables, and everything that is pure logic lives here so it is tested
//! on every target:
//!
//! * [`Outlet`]: the `@IBOutlet private var menuXxx: NSMenuItem?` properties;
//! * [`SYNCED_SHORTCUTS`]: the `syncMenuShortcut(config, action:, menuItem:)`
//!   list of `syncMenuShortcuts`;
//! * [`MENU_IMAGES`]: the SF Symbols of `setupMenuImages`;
//! * [`MenuShortcutKey`]: the normalized shortcut lookup key.
//!
//! Upstream quirk preserved: the xib connects the "Use Selection for Find" item
//! to an outlet named `menuSelectionForSearch`, which does not exist in
//! `AppDelegate.swift` (AppKit only logs "Failed to connect ... outlet"), and
//! "Jump to Selection" is not connected at all. As a result the Swift
//! `menuSelectionForFind` / `menuScrollToSelection` are always `nil` and their
//! `syncMenuShortcut` calls are no-ops; [`CONNECTED_OUTLETS`] therefore does not
//! contain them and `AppDelegate` leaves them unset.

pub mod generated;

pub use generated::{CONNECTED_OUTLETS, MAIN_MENU};

/// Where an item's action is sent (`target` of the xib connection).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// `-1`, the First Responder proxy: a nil target, resolved through the
    /// responder chain (controllers, window, then the app delegate).
    FirstResponder,
    /// The `AppDelegate` object of the xib.
    AppDelegate,
}

/// An item's action: an Objective-C selector name and its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action {
    pub selector: &'static str,
    pub target: Target,
}

/// `NSEvent.ModifierFlags` device independent mask bits for a key equivalent
/// (the raw values AppKit uses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mods(pub usize);

impl Mods {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1 << 17);
    pub const CONTROL: Self = Self(1 << 18);
    pub const OPTION: Self = Self(1 << 19);
    pub const COMMAND: Self = Self(1 << 20);

    /// The four modifiers that take part in a shortcut
    /// (`MenuShortcutKey.shortcutModifiers`).
    pub const SHORTCUT: Self =
        Self(Self::SHIFT.0 | Self::CONTROL.0 | Self::OPTION.0 | Self::COMMAND.0);
}

/// Which special menu AppKit is told about (`systemMenu` in the xib).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemMenu {
    /// The application main menu (`NSApp.mainMenu`).
    Main,
    /// The application menu ("Ghostty"), `NSApp.setAppleMenu:`.
    Apple,
    /// `NSApp.windowsMenu`.
    Window,
    /// `NSApp.helpMenu`.
    Help,
}

/// One entry of a menu.
#[derive(Clone, Copy, Debug)]
pub enum ItemSpec {
    Separator,
    Item(Item),
}

/// A menu item.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub title: &'static str,
    pub key_equivalent: &'static str,
    /// `None` is Interface Builder's default mask (command); `Some(Mods::NONE)`
    /// is an explicitly empty `<modifierMask key="keyEquivalentModifierMask"/>`.
    pub modifiers: Option<Mods>,
    pub action: Option<Action>,
    pub outlet: Option<Outlet>,
    pub submenu: Option<&'static MenuSpec>,
}

/// A menu.
#[derive(Clone, Copy, Debug)]
pub struct MenuSpec {
    pub title: &'static str,
    pub system_menu: Option<SystemMenu>,
    /// An outlet connected to the menu itself (`menuServices`).
    pub outlet: Option<Outlet>,
    pub items: &'static [ItemSpec],
}

impl MenuSpec {
    /// Visit every item of this menu and its submenus, depth first.
    pub fn walk(&self, f: &mut impl FnMut(&Item)) {
        for item in self.items {
            if let ItemSpec::Item(item) = item {
                f(item);
                if let Some(sub) = item.submenu {
                    sub.walk(f);
                }
            }
        }
    }
}

/// The `@IBOutlet`s of the Swift `AppDelegate` (plus `MenuSelectionForSearch`, the
/// outlet name the xib actually connects, see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Outlet {
    MenuAbout,
    MenuServices,
    MenuCheckForUpdates,
    MenuOpenConfig,
    MenuReloadConfig,
    MenuSecureInput,
    MenuQuit,
    MenuNewWindow,
    MenuNewTab,
    MenuSplitRight,
    MenuSplitLeft,
    MenuSplitDown,
    MenuSplitUp,
    MenuClose,
    MenuCloseTab,
    MenuCloseWindow,
    MenuCloseAllWindows,
    MenuUndo,
    MenuRedo,
    MenuCopy,
    MenuPaste,
    MenuPasteSelection,
    MenuSelectAll,
    MenuFindParent,
    MenuFind,
    MenuSelectionForFind,
    MenuScrollToSelection,
    MenuFindNext,
    MenuFindPrevious,
    MenuHideFindBar,
    MenuToggleVisibility,
    MenuToggleFullScreen,
    MenuBringAllToFront,
    MenuZoomSplit,
    MenuPreviousSplit,
    MenuNextSplit,
    MenuSelectSplitAbove,
    MenuSelectSplitBelow,
    MenuSelectSplitLeft,
    MenuSelectSplitRight,
    MenuReturnToDefaultSize,
    MenuFloatOnTop,
    MenuUseAsDefault,
    MenuSetAsDefaultTerminal,
    MenuIncreaseFontSize,
    MenuDecreaseFontSize,
    MenuResetFontSize,
    MenuChangeTitle,
    MenuChangeTabTitle,
    MenuReadonly,
    MenuQuickTerminal,
    MenuTerminalInspector,
    MenuCommandPalette,
    MenuEqualizeSplits,
    MenuMoveSplitDividerUp,
    MenuMoveSplitDividerDown,
    MenuMoveSplitDividerLeft,
    MenuMoveSplitDividerRight,
    MenuSelectionForSearch,
}

/// `syncMenuShortcuts`: (Ghostty keybind action, menu item), in the Swift order.
pub const SYNCED_SHORTCUTS: &[(&str, Outlet)] = &[
    ("check_for_updates", Outlet::MenuCheckForUpdates),
    ("open_config", Outlet::MenuOpenConfig),
    ("reload_config", Outlet::MenuReloadConfig),
    ("quit", Outlet::MenuQuit),
    ("new_window", Outlet::MenuNewWindow),
    ("new_tab", Outlet::MenuNewTab),
    ("close_surface", Outlet::MenuClose),
    ("close_tab", Outlet::MenuCloseTab),
    ("close_window", Outlet::MenuCloseWindow),
    ("close_all_windows", Outlet::MenuCloseAllWindows),
    ("new_split:right", Outlet::MenuSplitRight),
    ("new_split:left", Outlet::MenuSplitLeft),
    ("new_split:down", Outlet::MenuSplitDown),
    ("new_split:up", Outlet::MenuSplitUp),
    ("undo", Outlet::MenuUndo),
    ("redo", Outlet::MenuRedo),
    ("copy_to_clipboard", Outlet::MenuCopy),
    ("paste_from_clipboard", Outlet::MenuPaste),
    ("paste_from_selection", Outlet::MenuPasteSelection),
    ("select_all", Outlet::MenuSelectAll),
    ("start_search", Outlet::MenuFind),
    ("search_selection", Outlet::MenuSelectionForFind),
    ("scroll_to_selection", Outlet::MenuScrollToSelection),
    ("search:next", Outlet::MenuFindNext),
    ("search:previous", Outlet::MenuFindPrevious),
    ("toggle_split_zoom", Outlet::MenuZoomSplit),
    ("goto_split:previous", Outlet::MenuPreviousSplit),
    ("goto_split:next", Outlet::MenuNextSplit),
    ("goto_split:up", Outlet::MenuSelectSplitAbove),
    ("goto_split:down", Outlet::MenuSelectSplitBelow),
    ("goto_split:left", Outlet::MenuSelectSplitLeft),
    ("goto_split:right", Outlet::MenuSelectSplitRight),
    ("resize_split:up,10", Outlet::MenuMoveSplitDividerUp),
    ("resize_split:down,10", Outlet::MenuMoveSplitDividerDown),
    ("resize_split:right,10", Outlet::MenuMoveSplitDividerRight),
    ("resize_split:left,10", Outlet::MenuMoveSplitDividerLeft),
    ("equalize_splits", Outlet::MenuEqualizeSplits),
    ("reset_window_size", Outlet::MenuReturnToDefaultSize),
    ("increase_font_size:1", Outlet::MenuIncreaseFontSize),
    ("decrease_font_size:1", Outlet::MenuDecreaseFontSize),
    ("reset_font_size", Outlet::MenuResetFontSize),
    ("prompt_surface_title", Outlet::MenuChangeTitle),
    ("prompt_tab_title", Outlet::MenuChangeTabTitle),
    ("toggle_quick_terminal", Outlet::MenuQuickTerminal),
    ("toggle_visibility", Outlet::MenuToggleVisibility),
    ("toggle_window_float_on_top", Outlet::MenuFloatOnTop),
    ("inspector:toggle", Outlet::MenuTerminalInspector),
    ("toggle_command_palette", Outlet::MenuCommandPalette),
    ("toggle_secure_input", Outlet::MenuSecureInput),
];

/// `setupMenuImages`: (menu item, SF Symbol name), in the Swift order.
pub const MENU_IMAGES: &[(Outlet, &str)] = &[
    (Outlet::MenuAbout, "info.circle"),
    (Outlet::MenuCheckForUpdates, "square.and.arrow.down"),
    (Outlet::MenuOpenConfig, "gear"),
    (
        Outlet::MenuReloadConfig,
        "arrow.trianglehead.2.clockwise.rotate.90",
    ),
    (Outlet::MenuSecureInput, "lock.display"),
    (Outlet::MenuNewWindow, "macwindow.badge.plus"),
    (Outlet::MenuNewTab, "macwindow"),
    (Outlet::MenuSplitRight, "rectangle.righthalf.inset.filled"),
    (Outlet::MenuSplitLeft, "rectangle.leadinghalf.inset.filled"),
    (Outlet::MenuSplitUp, "rectangle.tophalf.inset.filled"),
    (Outlet::MenuSplitDown, "rectangle.bottomhalf.inset.filled"),
    (Outlet::MenuClose, "xmark"),
    (Outlet::MenuPasteSelection, "doc.on.clipboard.fill"),
    (Outlet::MenuIncreaseFontSize, "textformat.size.larger"),
    (Outlet::MenuResetFontSize, "textformat.size"),
    (Outlet::MenuDecreaseFontSize, "textformat.size.smaller"),
    (Outlet::MenuCommandPalette, "filemenu.and.selection"),
    (Outlet::MenuQuickTerminal, "apple.terminal"),
    (Outlet::MenuChangeTabTitle, "pencil.line"),
    (Outlet::MenuTerminalInspector, "scope"),
    (Outlet::MenuReadonly, "eye.fill"),
    (Outlet::MenuSetAsDefaultTerminal, "star.fill"),
    (
        Outlet::MenuToggleFullScreen,
        "square.arrowtriangle.4.outward",
    ),
    (Outlet::MenuToggleVisibility, "eye"),
    (Outlet::MenuZoomSplit, "arrow.up.left.and.arrow.down.right"),
    (Outlet::MenuPreviousSplit, "chevron.backward.2"),
    (Outlet::MenuNextSplit, "chevron.forward.2"),
    (
        Outlet::MenuEqualizeSplits,
        "inset.filled.topleft.topright.bottomleft.bottomright.rectangle",
    ),
    (Outlet::MenuSelectSplitLeft, "arrow.left"),
    (Outlet::MenuSelectSplitRight, "arrow.right"),
    (Outlet::MenuSelectSplitAbove, "arrow.up"),
    (Outlet::MenuSelectSplitBelow, "arrow.down"),
    (Outlet::MenuMoveSplitDividerUp, "arrow.up.to.line"),
    (Outlet::MenuMoveSplitDividerDown, "arrow.down.to.line"),
    (Outlet::MenuMoveSplitDividerLeft, "arrow.left.to.line"),
    (Outlet::MenuMoveSplitDividerRight, "arrow.right.to.line"),
    (Outlet::MenuFloatOnTop, "square.filled.on.square"),
    (Outlet::MenuFindParent, "text.page.badge.magnifyingglass"),
];

/// `AppDelegate.MenuShortcutKey`: a menu shortcut normalized for quick lookup
/// (lower-cased key equivalent + only the shift/control/option/command bits).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MenuShortcutKey {
    key_equivalent: String,
    modifiers_raw_value: usize,
}

impl MenuShortcutKey {
    /// `init?(keyEquivalent:modifiers:)`; `modifiers_raw` is the
    /// `NSEvent.ModifierFlags.rawValue`.
    #[must_use]
    pub fn new(key_equivalent: &str, modifiers_raw: usize) -> Option<Self> {
        let normalized = key_equivalent.to_lowercase();
        if normalized.is_empty() {
            return None;
        }
        Some(Self {
            key_equivalent: normalized,
            modifiers_raw_value: modifiers_raw & Mods::SHORTCUT.0,
        })
    }
}

/// The `dockMenu` entries of `reloadDockMenu`: (title, selector).
pub const DOCK_MENU: &[(&str, &str)] = &[("New Window", "newWindow:"), ("New Tab", "newTab:")];

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn all_items() -> Vec<Item> {
        let mut v = Vec::new();
        MAIN_MENU.walk(&mut |i| v.push(*i));
        v
    }

    #[test]
    fn xib_menu_structure() {
        let top: Vec<_> = MAIN_MENU
            .items
            .iter()
            .filter_map(|i| {
                if let ItemSpec::Item(i) = i {
                    Some(i.title)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(top, ["Ghostty", "File", "Edit", "View", "Window", "Help"]);
        assert_eq!(MAIN_MENU.system_menu, Some(SystemMenu::Main));
        let sys: Vec<_> = all_items()
            .iter()
            .filter_map(|i| i.submenu.and_then(|m| m.system_menu))
            .collect();
        assert_eq!(
            sys,
            [SystemMenu::Apple, SystemMenu::Window, SystemMenu::Help]
        );
    }

    #[test]
    fn fixed_key_equivalents_from_the_xib() {
        let find = |title: &str| *all_items().iter().find(|i| i.title == title).unwrap();
        let prefs = find("Preferences…");
        assert_eq!(
            (prefs.key_equivalent, prefs.modifiers.is_none()),
            (",", true)
        );
        let hide_others = find("Hide Others");
        assert_eq!(hide_others.key_equivalent, "h");
        assert_eq!(
            hide_others.modifiers,
            Some(Mods(Mods::OPTION.0 | Mods::COMMAND.0))
        );
        let fs = find("Toggle Full Screen");
        assert_eq!(fs.key_equivalent, "f");
        assert_eq!(fs.modifiers, Some(Mods(Mods::CONTROL.0 | Mods::COMMAND.0)));
        assert_eq!(find("Ghostty Help").key_equivalent, "?");
        assert_eq!(find("Minimize").key_equivalent, "m");
    }

    #[test]
    fn outlets_match_the_swift_app_delegate() {
        // Every Swift outlet except the two the xib never connects has an item.
        let connected: std::collections::HashSet<_> = CONNECTED_OUTLETS.iter().copied().collect();
        for o in [Outlet::MenuSelectionForFind, Outlet::MenuScrollToSelection] {
            assert!(!connected.contains(&o), "{o:?} is not connected by the xib");
        }
        assert!(connected.contains(&Outlet::MenuSelectionForSearch));
        // Each connected outlet is attached to exactly one item/menu.
        let mut seen = std::collections::HashMap::new();
        MAIN_MENU.walk(&mut |i| {
            if let Some(o) = i.outlet {
                *seen.entry(o).or_insert(0) += 1;
            }
            if let Some(o) = i.submenu.and_then(|m| m.outlet) {
                *seen.entry(o).or_insert(0) += 1;
            }
        });
        assert_eq!(seen.len(), CONNECTED_OUTLETS.len());
        assert!(seen.values().all(|&n| n == 1));
    }

    #[test]
    fn synced_actions_are_unique_and_ordered_like_swift() {
        assert_eq!(SYNCED_SHORTCUTS.len(), 49);
        assert_eq!(
            SYNCED_SHORTCUTS[0],
            ("check_for_updates", Outlet::MenuCheckForUpdates)
        );
        assert_eq!(
            SYNCED_SHORTCUTS[48],
            ("toggle_secure_input", Outlet::MenuSecureInput)
        );
        // toggle_fullscreen is deliberately not synced.
        assert!(
            !SYNCED_SHORTCUTS
                .iter()
                .any(|(a, _)| *a == "toggle_fullscreen")
        );
    }

    #[test]
    fn menu_shortcut_key_normalizes() {
        let a = MenuShortcutKey::new("T", Mods::COMMAND.0 | 0x10000).unwrap();
        let b = MenuShortcutKey::new("t", Mods::COMMAND.0).unwrap();
        // caps lock (0x10000) is not a shortcut modifier.
        assert_eq!(a, b);
        assert_ne!(
            b,
            MenuShortcutKey::new("t", Mods::COMMAND.0 | Mods::SHIFT.0).unwrap()
        );
        assert!(MenuShortcutKey::new("", Mods::COMMAND.0).is_none());
    }
}
