//! `Helpers/Fullscreen.swift`: the fullscreen styles (native and the
//! non-native variants). The platform independent parts (the mode enum and
//! the frame/menu decisions) are in `fullscreen_logic`.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code, clippy::borrow_as_ptr, clippy::ref_as_ptr)]

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use objc2::{Message, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{
    NSApplication,
    NSApplicationPresentationOptions,
    NSResponder,
    NSScreen,
    NSTitlebarAccessoryViewController,
    NSTitlebarSeparatorStyle,
    NSToolbar,
    NSWindow,
    NSWindowDidChangeScreenNotification,
    NSWindowDidEnterFullScreenNotification,
    NSWindowDidExitFullScreenNotification,
    NSWindowOrderingMode,
    NSWindowStyleMask,
    NSWindowTabGroup,
    NSWindowToolbarStyle,
};
use objc2_foundation::{NSRect, NSSize};

pub use super::fullscreen_logic::FullscreenMode;
use super::{
    dispatch::main_async,
    extensions::{
        ns_application::NSApplicationExt,
        ns_notification::{self, NsObserver},
        ns_screen::NSScreenExt,
        ns_window::NSWindowExt,
    },
    fullscreen_logic::{self, Properties, TabReinsert},
    geometry::Rect,
    notification_center::{Name, NotificationCenter, Sender, UserInfo},
    private::cgs::{CGSSpace, CGSSpaceMask, CGSSpaceType},
};

/// `Notification.Name.fullscreenDidEnter`. The sender is the style's window
/// (`Retained<NSWindow>`).
pub const FULLSCREEN_DID_ENTER: Name = Name("com.mitchellh.fullscreenDidEnter");
/// `Notification.Name.fullscreenDidExit`.
pub const FULLSCREEN_DID_EXIT: Name = Name("com.mitchellh.fullscreenDidExit");

/// Delegate that can be implemented for fullscreen implementations.
pub trait FullscreenDelegate {
    /// Called whenever the fullscreen state changed. You can call
    /// `is_fullscreen` to see the current state.
    fn fullscreen_did_change(&self);
}

/// Protocol that must be implemented by all fullscreen styles.
pub trait FullscreenStyle {
    fn set_delegate(&self, delegate: Option<Weak<dyn FullscreenDelegate>>);
    fn fullscreen_mode(&self) -> FullscreenMode;
    fn is_fullscreen(&self) -> bool;
    fn supports_tabs(&self) -> bool;
    fn enter(&self);
    fn exit(&self);
}

/// `FullscreenMode.style(for:)`: initializes the fullscreen style
/// implementation for the mode. This will not toggle any fullscreen
/// properties. This may fail if the window isn't configured properly for a
/// given mode.
#[must_use]
pub fn style_for(mode: FullscreenMode, window: &NSWindow) -> Option<Rc<dyn FullscreenStyle>> {
    match mode {
        FullscreenMode::Native => Some(NativeFullscreen::new(window)),
        _ => Some(NonNativeFullscreen::new(window, mode)),
    }
}

/// `TerminalWindow.isTabBar(_:)`.
pub type IsTabBarFn = fn(&NSWindow, &NSTitlebarAccessoryViewController) -> bool;

thread_local! {
    /// TODO(MA3): `TerminalWindow.isTabBar(_:)`, registered by the terminal
    /// window feature. Non-native fullscreen exit uses it to skip restoring
    /// the (private) tab bar accessory.
    static IS_TAB_BAR: Cell<Option<IsTabBarFn>> = const { Cell::new(None) };
}

/// Register `TerminalWindow.isTabBar(_:)`.
pub fn set_is_tab_bar_hook(f: IsTabBarFn) {
    IS_TAB_BAR.with(|h| h.set(Some(f)));
}

fn is_tab_bar(window: &NSWindow, c: &NSTitlebarAccessoryViewController) -> bool {
    IS_TAB_BAR.with(Cell::get).is_some_and(|f| f(window, c))
}

/// The base class for fullscreen implementations, cannot be used as a
/// FullscreenStyle on its own.
struct FullscreenBase {
    window: Retained<NSWindow>,
    delegate: RefCell<Option<Weak<dyn FullscreenDelegate>>>,
    observers: RefCell<Vec<NsObserver>>,
}

impl FullscreenBase {
    fn new(window: &NSWindow) -> Rc<Self> {
        let base = Rc::new(Self {
            window: window.retain(),
            delegate: RefCell::new(None),
            observers: RefCell::new(Vec::new()),
        });

        // We want to trigger delegate methods on window native fullscreen
        // changes (didEnterFullScreenNotification, etc.) no matter what our
        // fullscreen style is.
        let window_obj: &AnyObject = window.as_ref();
        // SAFETY: reading AppKit's immutable notification name constants.
        #[allow(unsafe_code)]
        let (enter, exit) = unsafe {
            (
                NSWindowDidEnterFullScreenNotification,
                NSWindowDidExitFullScreenNotification,
            )
        };
        let weak = Rc::downgrade(&base);
        let o1 = ns_notification::observe(enter, Some(window_obj), move |_| {
            if let Some(b) = weak.upgrade() {
                b.notify(FULLSCREEN_DID_ENTER);
            }
        });
        let weak = Rc::downgrade(&base);
        let o2 = ns_notification::observe(exit, Some(window_obj), move |_| {
            if let Some(b) = weak.upgrade() {
                b.notify(FULLSCREEN_DID_EXIT);
            }
        });
        base.observers.borrow_mut().extend([o1, o2]);
        base
    }

    /// Post the Ghostty notification and call the delegate.
    fn notify(&self, name: Name) {
        let id = Retained::as_ptr(&self.window) as usize;
        NotificationCenter::default_center().post(
            name,
            Some(Sender::new(id, self.window.clone())),
            UserInfo::new(),
        );
        self.fullscreen_did_change();
    }

    fn fullscreen_did_change(&self) {
        let delegate = self.delegate.borrow().as_ref().and_then(Weak::upgrade);
        if let Some(d) = delegate {
            d.fullscreen_did_change();
        }
    }
}

/// macOS native fullscreen. This is the typical behavior you get by pressing
/// the green fullscreen button on regular titlebars.
pub struct NativeFullscreen {
    base: Rc<FullscreenBase>,
}

impl NativeFullscreen {
    #[must_use]
    pub fn new(window: &NSWindow) -> Rc<Self> {
        // TODO: There are many requirements for native fullscreen we should
        // check here such as the stylemask.
        Rc::new(Self {
            base: FullscreenBase::new(window),
        })
    }
}

impl FullscreenStyle for NativeFullscreen {
    fn set_delegate(&self, delegate: Option<Weak<dyn FullscreenDelegate>>) {
        *self.base.delegate.borrow_mut() = delegate;
    }

    fn fullscreen_mode(&self) -> FullscreenMode {
        FullscreenMode::Native
    }

    fn is_fullscreen(&self) -> bool {
        self.base
            .window
            .styleMask()
            .contains(NSWindowStyleMask::FullScreen)
    }

    fn supports_tabs(&self) -> bool {
        true
    }

    fn enter(&self) {
        if self.is_fullscreen() {
            return;
        }

        // The titlebar separator shows up erroneously in fullscreen if the tab
        // bar is made to appear and then disappear by opening and then closing
        // a tab. We get rid of the separator while in fullscreen to prevent this.
        self.base
            .window
            .setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);

        // Enter fullscreen
        self.base
            .window
            .toggleFullScreen(Some(self.base.window.as_ref()));

        // Note: we don't call our delegate here because the base class
        // will always trigger the delegate on native fullscreen notifications
        // and we don't want to double notify.
    }

    fn exit(&self) {
        if !self.is_fullscreen() {
            return;
        }

        // Restore titlebar separator style. See enter for explanation.
        self.base
            .window
            .setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::Automatic);

        self.base.window.toggleFullScreen(None);

        // Note: we don't call our delegate here because the base class
        // will always trigger the delegate on native fullscreen notifications
        // and we don't want to double notify.
    }
}

/// The state that must be saved for non-native fullscreen to exit fullscreen.
struct SavedState {
    screen_id: Option<u32>,
    tab_group: Option<Retained<NSWindowTabGroup>>,
    tab_group_index: Option<usize>,
    content_frame: NSRect,
    style_mask: NSWindowStyleMask,
    toolbar: Option<Retained<NSToolbar>>,
    toolbar_style: NSWindowToolbarStyle,
    titlebar_accessory_view_controllers: Vec<Retained<NSTitlebarAccessoryViewController>>,
    dock: bool,
    menu: bool,
}

impl SavedState {
    fn new(window: &NSWindow) -> Option<Self> {
        let content_view = window.contentView()?;

        let tab_group = window.tabGroup();
        let tab_group_index = tab_group
            .as_ref()
            .and_then(|g| g.windows().iter().position(|w| std::ptr::eq(&*w, window)));

        // Accessing titlebarAccessoryViewControllers without a titlebar
        // triggers a crash.
        // SAFETY: `hasTitleBar` is a plain BOOL getter on NSWindow.
        #[allow(unsafe_code)]
        let has_title_bar: bool = unsafe { msg_send![window, hasTitleBar] };
        let titlebar_accessory_view_controllers = if has_title_bar {
            window.titlebarAccessoryViewControllers().to_vec()
        } else {
            Vec::new()
        };

        let menu = if let Some(cg_window_id) = window.cg_window_id() {
            // We hide the menu only if this window is not on any fullscreen
            // spaces. We do this because fullscreen spaces already hide the
            // menu and if we insert/remove this presentation option we get
            // issues (see #7075)
            let active_space = CGSSpace::active();
            let spaces = CGSSpace::list(cg_window_id, CGSSpaceMask::ALL_SPACES);
            let flags: Vec<(bool, bool)> = spaces
                .iter()
                .map(|s| {
                    (
                        *s == active_space,
                        s.space_type() == Some(CGSSpaceType::Fullscreen),
                    )
                })
                .collect();
            fullscreen_logic::can_hide_menu(
                &flags,
                active_space.space_type() == Some(CGSSpaceType::Fullscreen),
            )
        } else {
            // Window doesn't have a window device, its not visible or
            // something. In this case, we assume we can hide the menu. We may
            // want to do something more sophisticated but this works for now.
            true
        };

        Some(Self {
            screen_id: window.screen().and_then(|s| s.display_id()),
            tab_group,
            tab_group_index,
            content_frame: window.convertRectToScreen(content_view.frame()),
            style_mask: window.styleMask(),
            toolbar: window.toolbar(),
            toolbar_style: window.toolbarStyle(),
            titlebar_accessory_view_controllers,
            dock: window.screen().is_some_and(|s| s.has_dock()),
            menu,
        })
    }
}

struct NonNativeInner {
    base: Rc<FullscreenBase>,
    mode: FullscreenMode,
    saved_state: RefCell<Option<SavedState>>,
    screen_observer: RefCell<Option<NsObserver>>,
    close_observer: RefCell<Option<NsObserver>>,
}

/// Non-native fullscreen, with the variants selected by [`FullscreenMode`]
/// (`NonNativeFullscreen`, `...VisibleMenu`, `...PaddedNotch`).
pub struct NonNativeFullscreen {
    inner: Rc<NonNativeInner>,
}

fn to_rect(r: NSRect) -> Rect {
    Rect::new(r.origin.x, r.origin.y, r.size.width, r.size.height)
}

impl NonNativeFullscreen {
    /// `mode` must be one of the non-native modes (anything else behaves like
    /// plain `NonNative`).
    #[must_use]
    pub fn new(window: &NSWindow, mode: FullscreenMode) -> Rc<Self> {
        let base = FullscreenBase::new(window);
        let inner = Rc::new(NonNativeInner {
            base,
            mode: if mode == FullscreenMode::Native {
                FullscreenMode::NonNative
            } else {
                mode
            },
            saved_state: RefCell::new(None),
            screen_observer: RefCell::new(None),
            close_observer: RefCell::new(None),
        });

        let weak = Rc::downgrade(&inner);
        let window_obj: &AnyObject = window.as_ref();
        // SAFETY: reading AppKit's immutable notification name constant.
        #[allow(unsafe_code)]
        let will_close = unsafe { objc2_app_kit::NSWindowWillCloseNotification };
        let observer = ns_notification::observe(will_close, Some(window_obj), move |_| {
            // When the window closes we need to explicitly exit non-native
            // fullscreen otherwise some state like the menu bar can remain hidden.
            if let Some(i) = weak.upgrade() {
                i.exit();
            }
        });
        *inner.close_observer.borrow_mut() = Some(observer);
        Rc::new(Self { inner })
    }
}

impl NonNativeInner {
    /// The default properties for this style.
    fn properties(&self) -> Properties {
        Properties::for_mode(self.mode)
    }

    fn window(&self) -> &Retained<NSWindow> {
        &self.base.window
    }

    fn is_fullscreen(&self) -> bool {
        // isFullscreen is dependent on if we have saved state currently. We
        // could one day try to do fancier stuff like inspecting the window
        // state but there isn't currently a need for it.
        self.saved_state.borrow().is_some()
    }

    fn enter(self: &Rc<Self>) {
        // If we are in fullscreen we don't do it again.
        if self.is_fullscreen() {
            return;
        }
        let window = self.window().clone();

        // If we are in native fullscreen, exit native fullscreen. This is
        // counter intuitive but if we entered native fullscreen (through the
        // green max button or an external event) and we press the fullscreen
        // keybind, we probably want to EXIT fullscreen.
        if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
            window.toggleFullScreen(None);
            return;
        }

        // This is the screen that we're going to go fullscreen on. We use the
        // screen the window is currently on.
        let Some(screen) = window.screen() else {
            return;
        };

        // Save the state that we need to exit again
        let Some(saved_state) = SavedState::new(&window) else {
            return;
        };
        let (save_dock, save_menu) = (saved_state.dock, saved_state.menu);
        *self.saved_state.borrow_mut() = Some(saved_state);

        // Get our current first responder on this window. For non-native
        // fullscreen we have to restore this because for some reason the
        // operations below lose it (see:
        // https://github.com/ghostty-org/ghostty/issues/6999). I don't know
        // the root cause here so if we can figure that out there may be a
        // nicer way than this.
        let first_responder: Option<Retained<NSResponder>> = window.firstResponder();

        // We hide the dock if the window is on a screen with the dock.
        // We must hide the dock FIRST then hide the menu:
        // If you specify autoHideMenuBar, it must be accompanied by either
        // hideDock or autoHideDock.
        if save_dock {
            self.hide_dock();
        }

        // Hide the menu if requested
        if self.properties().hide_menu && save_menu {
            self.hide_menu();
        }

        // When we change screens we need to redo everything.
        let weak = Rc::downgrade(self);
        let window_obj: &AnyObject = window.as_ref();
        // SAFETY: reading AppKit's immutable notification name constant.
        #[allow(unsafe_code)]
        let did_change_screen = unsafe { NSWindowDidChangeScreenNotification };
        *self.screen_observer.borrow_mut() = Some(ns_notification::observe(
            did_change_screen,
            Some(window_obj),
            move |_| {
                if let Some(i) = weak.upgrade() {
                    i.window_did_change_screen();
                }
            },
        ));

        // Being untitled let's our content take up the full frame.
        window.setStyleMask(window.styleMask() - NSWindowStyleMask::Titled);

        // We don't want the non-native fullscreen window to be resizable
        // from the edges.
        window.setStyleMask(window.styleMask() - NSWindowStyleMask::Resizable);

        // Focus window
        window.makeKeyAndOrderFront(None);

        // Set frame to screen size, accounting for any elements such as the
        // menu bar. We do this async so that all the style edits above (title
        // removal, dock hide, menu hide, etc.) take effect. This fixes:
        // https://github.com/ghostty-org/ghostty/issues/1996
        let this = self.clone();
        main_async(move || {
            this.window()
                .setFrame_display(this.fullscreen_frame(&screen), true);
            if let Some(first_responder) = &first_responder {
                this.window().makeFirstResponder(Some(first_responder));
            }

            this.base.notify(FULLSCREEN_DID_ENTER);
        });
    }

    fn exit(&self) {
        if !self.is_fullscreen() {
            return;
        }
        let Some(saved_state) = self.saved_state.borrow().as_ref().map(SavedStateView::of) else {
            return;
        };
        let window = self.window().clone();

        // Remove all our notifications. We remove them one by one because we
        // don't want to remove the observers that our superclass sets.
        *self.screen_observer.borrow_mut() = None;

        // See enter where we do the same thing to understand why.
        let first_responder: Option<Retained<NSResponder>> = window.firstResponder();

        // Unhide our elements
        if saved_state.dock {
            self.unhide_dock();
        }
        if self.properties().hide_menu && saved_state.menu {
            self.unhide_menu();
        }

        // Restore our saved state
        let saved = self.saved_state.borrow_mut().take().expect("checked above");
        window.setStyleMask(saved.style_mask);
        window.setFrame_display(window.frameRectForContentRect(saved.content_frame), true);

        // Removing the "titled" style also derefs all our accessory view
        // controllers so we need to restore those.
        for c in &saved.titlebar_accessory_view_controllers {
            // Restoring the tab bar causes all sorts of problems. Its best to
            // just ignore it, even though this is kind of a hack.
            if is_tab_bar(&window, c) {
                continue;
            }

            let present = window
                .titlebarAccessoryViewControllers()
                .iter()
                .any(|x| std::ptr::eq(Retained::as_ptr(&x), Retained::as_ptr(c)));
            if !present {
                window.addTitlebarAccessoryViewController(c);
            }
        }

        // Removing "titled" also clears our toolbar
        window.setToolbar(saved.toolbar.as_deref());
        window.setToolbarStyle(saved.toolbar_style);

        // If the window was previously in a tab group that isn't empty now,
        // we re-add it. We have to do this because our process of doing
        // non-native fullscreen removes the window from the tab group.
        if let (Some(tab_group), Some(tab_index)) = (&saved.tab_group, saved.tab_group_index) {
            let windows = tab_group.windows();
            if windows.count() > 0 {
                match fullscreen_logic::tab_reinsert(tab_index, windows.count()) {
                    TabReinsert::BelowFirst => {
                        // We were previously the first tab. Add it before
                        // ("below") the first window in the tab group currently.
                        windows
                            .objectAtIndex(0)
                            .add_tabbed_window_safely(&window, NSWindowOrderingMode::Below);
                    }
                    TabReinsert::AboveIndex(i) => {
                        // We were somewhere in the middle
                        windows
                            .objectAtIndex(i)
                            .add_tabbed_window_safely(&window, NSWindowOrderingMode::Above);
                    }
                    TabReinsert::BelowLast => {
                        // We were at the end
                        windows
                            .objectAtIndex(windows.count() - 1)
                            .add_tabbed_window_safely(&window, NSWindowOrderingMode::Below);
                    }
                }
            }
        }

        if let Some(first_responder) = &first_responder {
            window.makeFirstResponder(Some(first_responder));
        }

        // Focus window
        window.makeKeyAndOrderFront(None);

        // Notify the delegate
        self.base.notify(FULLSCREEN_DID_EXIT);
    }

    fn fullscreen_frame(&self, screen: &NSScreen) -> NSRect {
        let mtm = super::main_thread();
        let app = NSApplication::sharedApplication(mtm);
        let options = app.presentationOptions();
        let menu_hidden = options.contains(NSApplicationPresentationOptions::AutoHideMenuBar)
            || options.contains(NSApplicationPresentationOptions::HideMenuBar);
        // NOTE on macOS bugs: macOS used to have a bug where menuBarHeight
        // didn't account for the notch. I reported this as a radar and it was
        // fixed at some point. I don't know when that was so I can't put an
        // #available check, but it was in a bug fix release so I think if a
        // bug is reported to Ghostty we can just advise the user to update.
        let f = fullscreen_logic::fullscreen_frame(
            to_rect(screen.frame()),
            menu_hidden,
            app.mainMenu().map_or(0.0, |m| m.menuBarHeight()),
            self.properties().padded_notch,
            screen.safeAreaInsets().top,
        );
        NSRect::new(
            objc2_foundation::NSPoint::new(f.origin.x, f.origin.y),
            NSSize::new(f.size.width, f.size.height),
        )
    }

    // MARK: Window Events

    fn window_did_change_screen(&self) {
        if !self.is_fullscreen() {
            return;
        }
        let saved_screen_id = match self.saved_state.borrow().as_ref() {
            Some(s) => s.screen_id,
            None => return,
        };

        // Our screens must have changed
        if saved_screen_id == self.window().screen().and_then(|s| s.display_id()) {
            return;
        }

        // When we change screens, we simply exit fullscreen. Changing
        // screens shouldn't naturally be possible, it can only happen
        // through external window managers. There's a lot of accounting
        // to do to get the screen change right so instead of breaking
        // we just exit out. The user can re-enter fullscreen thereafter.
        self.exit();
    }

    // MARK: Dock

    #[allow(clippy::unused_self)]
    fn hide_dock(&self) {
        NSApplication::sharedApplication(super::main_thread())
            .acquire_presentation_option(NSApplicationPresentationOptions::AutoHideDock);
    }

    #[allow(clippy::unused_self)]
    fn unhide_dock(&self) {
        NSApplication::sharedApplication(super::main_thread())
            .release_presentation_option(NSApplicationPresentationOptions::AutoHideDock);
    }

    // MARK: Menu

    #[allow(clippy::unused_self)]
    fn hide_menu(&self) {
        NSApplication::sharedApplication(super::main_thread())
            .acquire_presentation_option(NSApplicationPresentationOptions::AutoHideMenuBar);
    }

    #[allow(clippy::unused_self)]
    fn unhide_menu(&self) {
        NSApplication::sharedApplication(super::main_thread())
            .release_presentation_option(NSApplicationPresentationOptions::AutoHideMenuBar);
    }
}

/// The `Bool` flags of the saved state (what `exit` reads before it is taken).
struct SavedStateView {
    dock: bool,
    menu: bool,
}

impl SavedStateView {
    fn of(s: &SavedState) -> Self {
        Self {
            dock: s.dock,
            menu: s.menu,
        }
    }
}

impl FullscreenStyle for NonNativeFullscreen {
    fn set_delegate(&self, delegate: Option<Weak<dyn FullscreenDelegate>>) {
        *self.inner.base.delegate.borrow_mut() = delegate;
    }

    fn fullscreen_mode(&self) -> FullscreenMode {
        self.inner.mode
    }

    fn is_fullscreen(&self) -> bool {
        self.inner.is_fullscreen()
    }

    // Non-native fullscreen never supports tabs because tabs require the
    // "titled" style and we don't have it for non-native fullscreen.
    fn supports_tabs(&self) -> bool {
        false
    }

    fn enter(&self) {
        self.inner.enter();
    }

    fn exit(&self) {
        self.inner.exit();
    }
}
