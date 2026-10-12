//! `MainMenu.xib`, built programmatically from [`menu_spec`].
//!
//! `NSApplicationMain` loads the xib named by `NSMainNibFile` (`MainMenu`),
//! which instantiates the `AppDelegate`, the menus, the `NSFontManager` object
//! and wires the `@IBOutlet`s. The Rust app has no `ibtool`/nib, so
//! [`install`] creates the same objects in code:
//!
//! * menus/items with the same titles, key equivalents, modifier masks and
//!   actions (selector names are registered with the runtime; items whose xib
//!   target is the First Responder proxy get a nil target, so they travel the
//!   responder chain exactly like in the nib; `AppDelegate` targets are set);
//! * the system menus (`systemMenu` = apple/window/help) are registered with
//!   `NSApplication` (`setAppleMenu:`, `windowsMenu`, `helpMenu`);
//! * the outlet items are returned for `AppDelegate` ([`Outlets`]).
//!
//! Edit-menu extras AppKit injects (Start Dictation, Emoji & Symbols) and the
//! Window-menu tab items are added by AppKit on its own at runtime, as in the
//! nib case.

#![allow(unsafe_code)]

use std::{collections::HashMap, ffi::CString};

use objc2::{
    MainThreadMarker,
    MainThreadOnly,
    msg_send,
    rc::Retained,
    runtime::{AnyObject, Sel},
    sel,
};
use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSFontManager, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

use super::{
    app_delegate::AppDelegate,
    menu_spec::{ItemSpec, MenuSpec, Outlet, SystemMenu, Target},
};

/// The items/menus the xib connects to `AppDelegate` outlets.
#[derive(Default)]
pub struct Outlets {
    pub items: HashMap<Outlet, Retained<NSMenuItem>>,
    /// `menuServices`, the submenu of the Services item.
    pub services_menu: Option<Retained<NSMenu>>,
}

/// Build the main menu from the spec and register it (and the system menus)
/// with `app`. Also instantiates the `NSFontManager` the xib contains.
pub fn install(mtm: MainThreadMarker, app: &NSApplication, delegate: &AppDelegate) -> Outlets {
    // `<customObject customClass="NSFontManager"/>`: make sure the shared
    // font manager exists as the nib's instantiation would.
    let _ = NSFontManager::sharedFontManager(mtm);

    let mut outlets = Outlets::default();
    let main = build_menu(
        mtm,
        app,
        delegate,
        &super::menu_spec::MAIN_MENU,
        &mut outlets,
    );
    app.setMainMenu(Some(&main));
    outlets
}

fn build_menu(
    mtm: MainThreadMarker,
    app: &NSApplication,
    delegate: &AppDelegate,
    spec: &MenuSpec,
    outlets: &mut Outlets,
) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(spec.title));
    register_system_menu(app, spec.system_menu, &menu);
    if spec.outlet == Some(Outlet::MenuServices) {
        outlets.services_menu = Some(menu.clone());
    }

    for entry in spec.items {
        match entry {
            ItemSpec::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            ItemSpec::Item(item) => {
                let action: Option<Sel> = item.action.map(|a| selector(a.selector));
                // SAFETY: a valid title, selector (or none) and key equivalent.
                let ns_item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(item.title),
                        action,
                        &NSString::from_str(item.key_equivalent),
                    )
                };
                if let Some(mods) = item.modifiers {
                    ns_item.setKeyEquivalentModifierMask(NSEventModifierFlags(mods.0));
                }
                if let Some(a) = item.action {
                    match a.target {
                        // SAFETY: a nil target / the delegate (which outlives the menu).
                        Target::FirstResponder => unsafe { ns_item.setTarget(None) },
                        Target::AppDelegate => {
                            let target: &AnyObject = delegate;
                            // SAFETY: the delegate outlives the menu.
                            unsafe { ns_item.setTarget(Some(target)) };
                        }
                    }
                }
                if let Some(sub) = item.submenu {
                    let submenu = build_menu(mtm, app, delegate, sub, outlets);
                    ns_item.setSubmenu(Some(&submenu));
                }
                if let Some(o) = item.outlet {
                    outlets.items.insert(o, ns_item.clone());
                }
                menu.addItem(&ns_item);
            }
        }
    }
    menu
}

fn register_system_menu(app: &NSApplication, kind: Option<SystemMenu>, menu: &NSMenu) {
    match kind {
        Some(SystemMenu::Window) => app.setWindowsMenu(Some(menu)),
        Some(SystemMenu::Help) => app.setHelpMenu(Some(menu)),
        Some(SystemMenu::Apple) => {
            // `systemMenu="apple"`: Interface Builder calls the private
            // `-[NSApplication setAppleMenu:]`.
            let sel = sel!(setAppleMenu:);
            let responds: bool = unsafe { msg_send![app, respondsToSelector: sel] };
            if responds {
                // SAFETY: `setAppleMenu:` takes an NSMenu and returns void.
                let _: () = unsafe { msg_send![app, setAppleMenu: menu] };
            }
        }
        Some(SystemMenu::Main) | None => {}
    }
}

/// Register a selector by name (`#selector(name)`).
#[must_use]
pub fn selector(name: &str) -> Sel {
    let c = CString::new(name).expect("selector names contain no NUL");
    Sel::register(&c)
}
