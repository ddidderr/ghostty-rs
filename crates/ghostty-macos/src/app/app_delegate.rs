//! `AppDelegate.swift`: the application delegate.
//!
//! Mapping notes (see `docs/rewrite/notes/macos-app-design.md`):
//! * the `@IBOutlet private var menuXxx` properties are the [`Outlet`] map in
//!   [`Ivars`], filled by `app::main_menu::install`;
//! * `@Published appIcon`, `ObservableObject` conformance: not needed (the only
//!   consumers are SwiftUI views, which are ported programmatically);
//! * `NotificationCenter.default.addObserver(self, selector:)` becomes
//!   block-based observers whose tokens live in [`Ivars::observers`];
//! * `[weak self]` closures capture an `objc2::rc::Weak<AppDelegate>`;
//! * `@IBAction`s are Objective-C methods with the same selectors so the
//!   responder chain / menu targets resolve them exactly like in the nib.

#![allow(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::CString,
    ptr::NonNull,
    rc::Rc,
};

use block2::{DynBlock, RcBlock};
use dispatch2::{DispatchObject, DispatchQueue, DispatchRetained, DispatchSource};
use objc2::{
    AnyThread,
    DefinedClass,
    MainThreadMarker,
    MainThreadOnly,
    Message,
    define_class,
    msg_send,
    rc::{Retained, Weak},
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::{
    NSAlert,
    NSAlertFirstButtonReturn,
    NSAlertStyle,
    NSApplication,
    NSApplicationActivationPolicy,
    NSApplicationDelegate,
    NSApplicationTerminateReply,
    NSEvent,
    NSEventMask,
    NSEventModifierFlags,
    NSEventType,
    NSMenu,
    NSMenuItem,
    NSMenuItemValidation,
    NSModalResponse,
    NSSound,
    NSWindow,
    NSWindowLevel,
    NSWindowStyleMask,
    NSWorkspace,
};
use objc2_foundation::{
    NSArray,
    NSCoder,
    NSDictionary,
    NSKeyValueChangeKey,
    NSNotification,
    NSObject,
    NSObjectProtocol,
    NSProcessInfo,
    NSSet,
    NSString,
    NSURL,
    NSUUID,
};
use objc2_user_notifications::{
    UNAuthorizationOptions,
    UNAuthorizationStatus,
    UNNotification,
    UNNotificationAction,
    UNNotificationActionOptions,
    UNNotificationCategory,
    UNNotificationCategoryOptions,
    UNNotificationPresentationOptions,
    UNNotificationResponse,
    UNNotificationSetting,
    UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

use super::{
    main_menu::{self, Outlets},
    menu_spec::{DOCK_MENU, MENU_IMAGES, MenuShortcutKey, Outlet, SYNCED_SHORTCUTS},
};
use crate::{
    features::{
        about::AboutController,
        global_keybinds::GlobalEventTap,
        quick_terminal::{
            QUICK_TERMINAL_DID_CHANGE_VISIBILITY,
            QuickTerminalPosition,
            controller::{QuickTerminalController, QuickTerminalRestorableState},
        },
        secure_input::SecureInput,
        services::ServiceProvider,
        settings::ConfigurationErrorsController,
        terminal::{BaseTerminalController, TerminalController, TerminalWindow},
        update::UpdateController,
    },
    ghostty::{
        app::{App, Readiness},
        capi::{
            consts::{GHOSTTY_ACTION_PRESS, GHOSTTY_COLOR_SCHEME_DARK, GHOSTTY_COLOR_SCHEME_LIGHT},
            ghostty_app_has_global_keybinds,
            ghostty_app_key,
            ghostty_app_key_is_binding,
            ghostty_app_set_color_scheme,
        },
        config::Config,
        config_types::{AutoUpdate, BellFeatures, MacDockDropBehavior, MacHidden},
        notification as gn,
        package::{LaunchSource, SetSecureInput, launch_source},
        shell,
        surface_configuration::SurfaceConfiguration,
        surface_view::SurfaceView,
    },
    helpers::{
        extensions::{
            ns_menu_item::NSMenuItemExt,
            ns_notification::{self, NsObserver},
            ns_workspace::NSWorkspaceExt,
            user_defaults,
        },
        notification_center::{Notification, NotificationCenter, ObserverToken},
    },
};

/// `DerivedConfig`: the parts of the config the delegate caches.
#[derive(Clone, Copy)]
struct DerivedConfig {
    initial_window: bool,
    should_quit_after_last_window_closed: bool,
    quick_terminal_position: QuickTerminalPosition,
}

impl Default for DerivedConfig {
    fn default() -> Self {
        Self {
            initial_window: true,
            should_quit_after_last_window_closed: false,
            quick_terminal_position: QuickTerminalPosition::Top,
        }
    }
}

impl DerivedConfig {
    fn new(config: &Config) -> Self {
        Self {
            initial_window: config.initial_window(),
            should_quit_after_last_window_closed: config.should_quit_after_last_window_closed(),
            quick_terminal_position: config.quick_terminal_position(),
        }
    }
}

/// `QuickTerminalState`.
enum QuickTerminalState {
    /// Controller has not been initialized and has no pending restoration state.
    Uninitialized,
    /// Restoration state is pending; controller will use this when first accessed.
    /// (Set by `application(_:didDecodeRestorableState:)`; TODO(MA4).)
    #[allow(dead_code)]
    PendingRestore(QuickTerminalRestorableState),
    /// Controller has been initialized.
    Initialized(Retained<QuickTerminalController>),
}

/// `AppDelegate.ToggleVisibilityState`.
pub struct ToggleVisibilityState {
    hidden_windows: Vec<Weak<NSWindow>>,
    key_window: Option<Weak<NSWindow>>,
}

impl ToggleVisibilityState {
    fn new(mtm: MainThreadMarker) -> Self {
        let app = NSApplication::sharedApplication(mtm);
        // We need to know the key window so that we can bring focus back to the
        // right window if it was hidden.
        let key_window = app.keyWindow().map(|w| Weak::from_retained(&w));

        // We need to keep track of the windows that were visible because we only
        // want to bring back these windows if we remove the toggle.
        //
        // We also ignore fullscreen windows because they don't hide anyways.
        let mut visible: Vec<Retained<NSWindow>> = Vec::new();
        for window in app.windows() {
            if !window.isVisible() || window.styleMask().contains(NSWindowStyleMask::FullScreen) {
                continue;
            }
            // We only keep track of selectedWindow if it's in a tabGroup,
            // so we can keep its selection state when restoring
            let to_hide = window
                .tabGroup()
                .and_then(|g| g.selectedWindow())
                .unwrap_or(window);
            if !visible
                .iter()
                .any(|w| Retained::as_ptr(w) == Retained::as_ptr(&to_hide))
            {
                visible.push(to_hide);
            }
        }
        Self {
            hidden_windows: visible.iter().map(Weak::from_retained).collect(),
            key_window,
        }
    }

    fn restore(&self) {
        for w in &self.hidden_windows {
            if let Some(w) = w.load() {
                w.orderFrontRegardless();
            }
        }
        if let Some(w) = self.key_window.as_ref().and_then(Weak::load) {
            w.makeKeyWindow();
        }
    }
}

/// Instance variables of [`AppDelegate`].
pub struct Ivars {
    /// The ghostty global state. Only one per process.
    ghostty: Rc<App>,
    /// The `@IBOutlet` menu items.
    outlets: RefCell<HashMap<Outlet, Retained<NSMenuItem>>>,
    services_menu: RefCell<Option<Retained<NSMenu>>>,
    /// The dock menu.
    dock_menu: Retained<NSMenu>,
    /// This is only true before application has become active.
    application_has_become_active: Cell<bool>,
    /// Set in applicationDidFinishLaunching with the system uptime.
    application_launch_time: Cell<f64>,
    derived_config: Cell<DerivedConfig>,
    /// The global undo manager for app-level state such as window restoration.
    undo_manager: Retained<crate::helpers::expiring_undo_manager::ExpiringUndoManager>,
    quick_terminal_state: RefCell<QuickTerminalState>,
    /// Manages updates.
    update_controller: UpdateController,
    /// Tracks the windows that we hid for toggleVisibility.
    hidden_state: RefCell<Option<ToggleVisibilityState>>,
    /// Signals.
    signals: RefCell<Vec<DispatchRetained<DispatchSource>>>,
    /// Ghostty menu items indexed by their normalized shortcut. This avoids
    /// traversing the entire menu tree on every key equivalent event. Weak so the
    /// cache can never own menu items; the most recent item wins.
    menu_items_by_shortcut: RefCell<HashMap<MenuShortcutKey, Weak<NSMenuItem>>>,
    /// Observation tokens (NotificationCenter observers).
    observers: RefCell<Vec<NsObserver>>,
    bus_observers: RefCell<Vec<ObserverToken>>,
    local_monitor: RefCell<Option<Retained<AnyObject>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; AppDelegate does not
    // implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AppDelegate"]
    #[ivars = Ivars]
    pub struct AppDelegate;

    // SAFETY: NSObjectProtocol has no requirements.
    unsafe impl NSObjectProtocol for AppDelegate {}
    // SAFETY: the protocol methods are implemented in the inherent block below
    // with the protocol's selectors/signatures.
    unsafe impl NSApplicationDelegate for AppDelegate {}
    // SAFETY: as above.
    unsafe impl UNUserNotificationCenterDelegate for AppDelegate {}
    // SAFETY: as above.
    unsafe impl NSMenuItemValidation for AppDelegate {}

    impl AppDelegate {
        // MARK: - NSApplicationDelegate

        #[unsafe(method(applicationWillFinishLaunching:))]
        fn application_will_finish_launching(&self, _notification: &NSNotification) {
            if cfg!(debug_assertions) {
                if user_defaults::ghostty_suite().is_some() {
                    let clear = std::env::var("GHOSTTY_CLEAR_USER_DEFAULTS").ok();
                    if clear.as_deref().is_some_and(objc_bool_value) {
                        super::support::remove_persistent_domain_for_ghostty_suite();
                    }
                }
            }
            super::support::register_defaults(&[
                // Disable the automatic full screen menu item because we handle
                // it manually.
                ("NSFullScreenMenuItemEverywhere", false),
                // On macOS 26 RC1, the autofill heuristic controller causes unusable levels
                // of slowdowns and CPU usage in the terminal window under certain [unknown]
                // conditions. We don't know exactly why/how. This disables the full heuristic
                // controller.
                //
                // Practically, this means things like SMS autofill don't work, but that is
                // a desirable behavior to NOT have happen for a terminal, so this is a win.
                // Manual autofill via the `Edit => AutoFill` menu item still work as expected.
                ("NSAutoFillHeuristicControllerEnabled", false),
            ]);
        }

        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            self.did_finish_launching();
        }

        #[unsafe(method(applicationDidHide:))]
        fn application_did_hide(&self, _notification: &NSNotification) {
            // Keep track of our hidden state to restore properly
            *self.ivars().hidden_state.borrow_mut() = Some(ToggleVisibilityState::new(self.mtm()));
        }

        #[unsafe(method(applicationDidBecomeActive:))]
        fn application_did_become_active(&self, _notification: &NSNotification) {
            self.did_become_active();
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn application_should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            self.ivars().derived_config.get().should_quit_after_last_window_closed
        }

        #[unsafe(method(applicationShouldTerminate:))]
        fn application_should_terminate(&self, sender: &NSApplication) -> NSApplicationTerminateReply {
            self.should_terminate(sender)
        }

        #[unsafe(method(applicationWillTerminate:))]
        fn application_will_terminate(&self, _notification: &NSNotification) {
            // We have no notifications we want to persist after death,
            // so remove them all now. In the future we may want to be
            // more selective and only remove surface-targeted notifications.
            UNUserNotificationCenter::currentNotificationCenter().removeAllDeliveredNotifications();
        }

        /// This is called when the application is already open and someone
        /// double-clicks the icon or clicks the dock icon.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn application_should_handle_reopen(&self, _sender: &NSApplication, flag: bool) -> bool {
            self.should_handle_reopen(flag)
        }

        #[unsafe(method(application:openFile:))]
        fn application_open_file(&self, _sender: &NSApplication, filename: &NSString) -> bool {
            self.open_file(&filename.to_string())
        }

        // MARK: Restorable state

        /// We support NSSecureCoding for restorable state. Required as of macOS
        /// Sonoma (14) but a good idea anyways.
        #[unsafe(method(applicationSupportsSecureRestorableState:))]
        fn application_supports_secure_restorable_state(&self, _app: &NSApplication) -> bool {
            true
        }

        #[unsafe(method(application:willEncodeRestorableState:))]
        fn application_will_encode_restorable_state(&self, _app: &NSApplication, _coder: &NSCoder) {
            log::debug!("application will save window state");
            if self.ivars().ghostty.config().window_save_state() == "never" {
                return;
            }
            // TODO(MA4): encode the quick terminal state
            // (QuickTerminalRestorableState.encode(with:)).
            match &*self.ivars().quick_terminal_state.borrow() {
                QuickTerminalState::Initialized(c) if c.restorable() => {
                    log::warn!("TODO(MA4): encode quick terminal restorable state");
                }
                _ => {}
            }
        }

        #[unsafe(method(application:didDecodeRestorableState:))]
        fn application_did_decode_restorable_state(&self, _app: &NSApplication, _coder: &NSCoder) {
            log::debug!("application will restore window state");
            // TODO(MA4): decode QuickTerminalRestorableState(coder:) and, if present
            // and window-save-state != never, set `.pendingRestore(state)`.
        }

        #[unsafe(method_id(applicationDockMenu:))]
        fn application_dock_menu(&self, _sender: &NSApplication) -> Option<Retained<NSMenu>> {
            Some(self.ivars().dock_menu.clone())
        }

        // MARK: - UNUserNotificationCenterDelegate

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn user_notification_did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            self.ivars().ghostty.handle_user_notification(response);
            completion.call(());
        }

        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn user_notification_will_present(
            &self,
            _center: &UNUserNotificationCenter,
            notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            let should_present = self.ivars().ghostty.should_present_notification(notification);
            let options = if should_present {
                UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::Sound
            } else {
                UNNotificationPresentationOptions::empty()
            };
            completion.call((options,));
        }

        // MARK: - KVO (NSApplication.effectiveAppearance)

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>,
            _context: *mut std::ffi::c_void,
        ) {
            if key_path.is_some_and(|k| k.to_string() == "effectiveAppearance") {
                self.report_color_scheme();
            }
        }

        // MARK: - IB Actions

        #[unsafe(method(openConfig:))]
        fn open_config_action(&self, _sender: Option<&AnyObject>) {
            App::open_config();
        }

        #[unsafe(method(reloadConfig:))]
        fn reload_config_action(&self, _sender: Option<&AnyObject>) {
            self.ivars().ghostty.reload_config(false);
        }

        #[unsafe(method(checkForUpdates:))]
        fn check_for_updates_action(&self, _sender: Option<&AnyObject>) {
            self.ivars().update_controller.check_for_updates();
            // UpdateSimulator.happyPath.simulate(with: updateViewModel)
        }

        #[unsafe(method(newWindow:))]
        fn new_window_action(&self, _sender: Option<&AnyObject>) {
            let _ = TerminalController::new_window(&self.ivars().ghostty, None);
        }

        #[unsafe(method(newTab:))]
        fn new_tab_action(&self, _sender: Option<&AnyObject>) {
            let parent = TerminalController::preferred_parent().and_then(|c| c.window());
            let _ = TerminalController::new_tab(&self.ivars().ghostty, parent.as_deref(), None);
        }

        #[unsafe(method(closeAllWindows:))]
        fn close_all_windows_action(&self, _sender: Option<&AnyObject>) {
            TerminalController::close_all_windows();
            AboutController::shared().hide();
        }

        #[unsafe(method(showAbout:))]
        fn show_about_action(&self, _sender: Option<&AnyObject>) {
            AboutController::shared().show();
        }

        #[unsafe(method(showHelp:))]
        fn show_help_action(&self, _sender: Option<&AnyObject>) {
            if let Some(url) = NSURL::URLWithString(&NSString::from_str("https://ghostty.org/docs")) {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }

        #[unsafe(method(toggleSecureInput:))]
        fn toggle_secure_input_action(&self, _sender: Option<&AnyObject>) {
            self.set_secure_input(SetSecureInput::Toggle);
        }

        #[unsafe(method(toggleQuickTerminal:))]
        fn toggle_quick_terminal_action(&self, _sender: Option<&AnyObject>) {
            self.quick_controller().toggle();
        }

        /// Toggles visibility of all Ghosty Terminal windows. When hidden,
        /// activates Ghostty as the frontmost application.
        #[unsafe(method(toggleVisibility:))]
        fn toggle_visibility_action(&self, _sender: Option<&AnyObject>) {
            self.toggle_visibility();
        }

        #[unsafe(method(bringAllToFront:))]
        fn bring_all_to_front_action(&self, sender: Option<&AnyObject>) {
            let app = NSApplication::sharedApplication(self.mtm());
            if !app.isActive() {
                #[allow(deprecated)]
                app.activateIgnoringOtherApps(true);
            }
            app.arrangeInFront(sender);
        }

        #[unsafe(method(undo:))]
        fn undo_action(&self, _sender: Option<&AnyObject>) {
            self.ivars().undo_manager.undo();
        }

        #[unsafe(method(redo:))]
        fn redo_action(&self, _sender: Option<&AnyObject>) {
            self.ivars().undo_manager.redo();
        }

        #[unsafe(method(floatOnTop:))]
        fn float_on_top_action(&self, menu_item: &NSMenuItem) {
            use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn};
            let on = menu_item.state() == NSControlStateValueOn;
            menu_item.setState(if on { NSControlStateValueOff } else { NSControlStateValueOn });
            let Some(window) = NSApplication::sharedApplication(self.mtm()).keyWindow() else { return };
            window.setLevel(if menu_item.state() == NSControlStateValueOn {
                FLOATING_WINDOW_LEVEL
            } else {
                NORMAL_WINDOW_LEVEL
            });
        }

        #[unsafe(method(useAsDefault:))]
        fn use_as_default_action(&self, _sender: &NSMenuItem) {
            use objc2_app_kit::NSControlStateValueOn;
            let ud = user_defaults::ghostty();
            let key = NSString::from_str(TerminalWindow::DEFAULT_LEVEL_KEY);
            let float_on = self
                .menu_item(Outlet::MenuFloatOnTop)
                .is_some_and(|i| i.state() == NSControlStateValueOn);
            if float_on {
                ud.setInteger_forKey(FLOATING_WINDOW_LEVEL, &key);
            } else {
                ud.removeObjectForKey(&key);
            }
        }

        #[unsafe(method(setAsDefaultTerminal:))]
        fn set_as_default_terminal_action(&self, _sender: &NSMenuItem) {
            self.set_as_default_terminal();
        }

        // MARK: NSMenuItemValidation

        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            self.validate(item)
        }

        // MARK: Services / dock helpers are in the inherent impl below.
    }
);

const NORMAL_WINDOW_LEVEL: NSWindowLevel = 0;
const FLOATING_WINDOW_LEVEL: NSWindowLevel = 3;

/// `(clear as NSString).boolValue`.
fn objc_bool_value(s: &str) -> bool {
    NSString::from_str(s).boolValue()
}

impl AppDelegate {
    /// `override init()`: creates the Ghostty app and the delegate.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let config_path = if cfg!(debug_assertions) {
            std::env::var("GHOSTTY_CONFIG_PATH").ok()
        } else {
            None
        };
        let ghostty = App::new(config_path.as_deref());
        let ivars = Ivars {
            ghostty,
            outlets: RefCell::default(),
            services_menu: RefCell::default(),
            dock_menu: NSMenu::new(mtm),
            application_has_become_active: Cell::new(false),
            application_launch_time: Cell::new(0.0),
            derived_config: Cell::new(DerivedConfig::default()),
            undo_manager: crate::helpers::expiring_undo_manager::ExpiringUndoManager::new(mtm),
            quick_terminal_state: RefCell::new(QuickTerminalState::Uninitialized),
            update_controller: UpdateController::new(),
            hidden_state: RefCell::default(),
            signals: RefCell::default(),
            menu_items_by_shortcut: RefCell::default(),
            observers: RefCell::default(),
            bus_observers: RefCell::default(),
            local_monitor: RefCell::default(),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `init` is NSObject's designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this.ivars()
            .ghostty
            .set_delegate(Weak::from_retained(&this));
        this
    }

    /// `NSApp.delegate as? AppDelegate`.
    #[must_use]
    pub fn shared(mtm: MainThreadMarker) -> Option<Retained<Self>> {
        let delegate = NSApplication::sharedApplication(mtm).delegate()?;
        // SAFETY: a protocol object is an Objective-C object; we only use the
        // result to attempt a checked downcast.
        let obj: Retained<AnyObject> = unsafe { Retained::cast_unchecked(delegate) };
        obj.downcast::<Self>().ok()
    }

    /// Connect the menu built from the xib spec (`@IBOutlet`s) -- done by
    /// `entry::run` before the application starts, like nib loading.
    pub fn connect_outlets(&self, outlets: Outlets) {
        *self.ivars().outlets.borrow_mut() = outlets.items;
        *self.ivars().services_menu.borrow_mut() = outlets.services_menu;
    }

    /// `applicationShouldHandleReopen(_:hasVisibleWindows:)`: called when the
    /// application is already open and someone double-clicks the icon or
    /// clicks the dock icon.
    fn should_handle_reopen(&self, flag: bool) -> bool {
        // If we have visible windows then we allow macOS to do its default behavior
        // of focusing one of them.
        if flag {
            return true;
        }

        // If we have any windows in our terminal manager we don't do anything.
        // This is possible with flag set to false if there a race where the
        // window is still initializing and is not visible but the user clicked
        // the dock icon.
        if !TerminalController::all().is_empty() {
            return true;
        }

        // If the application isn't active yet then we don't want to process
        // this because we're not ready. This happens sometimes in Xcode runs
        // but I haven't seen it happen in releases. I'm unsure why.
        if !self.ivars().application_has_become_active.get() {
            return true;
        }

        // No visible windows, open a new one.
        let _ = TerminalController::new_window(&self.ivars().ghostty, None);
        false
    }

    /// `ghostty`.
    #[must_use]
    pub fn ghostty(&self) -> Rc<App> {
        self.ivars().ghostty.clone()
    }

    /// `undoManager`.
    #[must_use]
    pub fn undo_manager(
        &self,
    ) -> Retained<crate::helpers::expiring_undo_manager::ExpiringUndoManager> {
        self.ivars().undo_manager.clone()
    }

    /// `updateController`.
    #[must_use]
    pub fn update_controller(&self) -> &UpdateController {
        &self.ivars().update_controller
    }

    /// `hiddenState`.
    #[must_use]
    pub fn has_hidden_state(&self) -> bool {
        self.ivars().hidden_state.borrow().is_some()
    }

    fn menu_item(&self, outlet: Outlet) -> Option<Retained<NSMenuItem>> {
        self.ivars().outlets.borrow().get(&outlet).cloned()
    }

    /// The elapsed time since the process was started.
    fn time_since_launch(&self) -> f64 {
        NSProcessInfo::processInfo().systemUptime() - self.ivars().application_launch_time.get()
    }

    /// Our quick terminal. This starts out uninitialized and only initializes if used.
    #[must_use]
    pub fn quick_controller(&self) -> Retained<QuickTerminalController> {
        let ivars = self.ivars();
        let mut state = ivars.quick_terminal_state.borrow_mut();
        match &*state {
            QuickTerminalState::Initialized(controller) => controller.clone(),
            QuickTerminalState::PendingRestore(restore) => {
                let controller = QuickTerminalController::new(
                    &ivars.ghostty,
                    ivars.derived_config.get().quick_terminal_position,
                    restore.base_config(),
                    Some(restore),
                );
                *state = QuickTerminalState::Initialized(controller.clone());
                controller
            }
            QuickTerminalState::Uninitialized => {
                let controller = QuickTerminalController::new(
                    &ivars.ghostty,
                    ivars.derived_config.get().quick_terminal_position,
                    None,
                    None,
                );
                *state = QuickTerminalState::Initialized(controller.clone());
                controller
            }
        }
    }

    /// `findSurface(forUUID:)` (GhosttyAppDelegate) and
    /// `ghosttySurface(id:)` (Ghostty.Delegate, `AppDelegate+Ghostty.swift`).
    #[must_use]
    pub fn find_surface(&self, uuid: &NSUUID) -> Option<Retained<SurfaceView>> {
        for c in TerminalController::all() {
            for view in c.surface_tree() {
                if view.id().isEqual(Some(uuid)) {
                    return Some(view);
                }
            }
        }
        None
    }

    // MARK: Launch

    fn did_finish_launching(&self) {
        let mtm = self.mtm();
        let app = NSApplication::sharedApplication(mtm);
        let ivars = self.ivars();

        // System settings overrides
        super::support::register_defaults(&[
            // Disable this so that repeated key events make it through to our terminal views.
            ("ApplePressAndHoldEnabled", false),
        ]);

        // Store our start time
        ivars
            .application_launch_time
            .set(NSProcessInfo::processInfo().systemUptime());

        // Check if secure input was enabled when we last quit.
        if user_defaults::ghostty().boolForKey(&NSString::from_str("SecureInput"))
            != SecureInput::shared().enabled()
        {
            self.set_secure_input(SetSecureInput::Toggle);
        }

        // Initial config loading
        self.ghostty_config_did_change(&ivars.ghostty.config());

        // Start our update checker.
        ivars.update_controller.start_updater();

        // Register our service provider. This must happen after everything is initialized.
        let provider = ServiceProvider::new();
        // SAFETY: the provider is retained by NSApplication's `servicesProvider`.
        unsafe { app.setServicesProvider(Some(&provider)) };

        // This registers the Ghostty => Services menu to exist.
        app.setServicesMenu(ivars.services_menu.borrow().as_deref());

        // Setup a local event monitor for app-level keyboard shortcuts. See
        // localEventHandler for more info why.
        self.install_local_monitor();

        // Notifications
        self.install_observers();

        // Configure user notifications
        self.configure_user_notifications();

        // Observe our appearance so we can report the correct value to libghostty.
        // SAFETY: KVO registration with a live observer (self lives for the whole
        // process as the application delegate).
        unsafe {
            let observer: &AnyObject = self;
            let _: () = msg_send![
                &*app,
                addObserver: observer,
                forKeyPath: &*NSString::from_str("effectiveAppearance"),
                options: objc2_foundation::NSKeyValueObservingOptions::New
                    | objc2_foundation::NSKeyValueObservingOptions::Initial,
                context: std::ptr::null_mut::<std::ffi::c_void>()
            ];
        }

        // Setup our menu
        self.setup_menu_images();

        // Setup signal handlers
        self.setup_signals();

        match launch_source() {
            LaunchSource::App => {
                // Don't have to do anything.
            }
            LaunchSource::ZigRun | LaunchSource::Cli => {
                // Part of launch services (clicking an app, using `open`, etc.) activates
                // the application and brings it to the front. When using the CLI we don't
                // get this behavior, so we have to do it manually.

                // This never gets called until we click the dock icon. This forces it
                // activate immediately.
                self.did_become_active();

                // We run in the background, this forces us to the front.
                DispatchQueue::main().exec_async(|| {
                    let Some(mtm) = MainThreadMarker::new() else {
                        return;
                    };
                    let app = NSApplication::sharedApplication(mtm);
                    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
                    #[allow(deprecated)]
                    app.activateIgnoringOtherApps(true);
                    app.unhide(None);
                    app.arrangeInFront(None);
                });
            }
        }
    }

    fn did_become_active(&self) {
        let ivars = self.ivars();
        // If we're back manually then clear the hidden state because macOS handles it.
        *ivars.hidden_state.borrow_mut() = None;

        // First launch stuff
        if !ivars.application_has_become_active.get() {
            ivars.application_has_become_active.set(true);

            // Let's launch our first window. We only do this if we have no other windows. It
            // is possible to have other windows in a few scenarios:
            //   - if we're opening a URL since `application(_:openFile:)` is called before this.
            //   - if we're restoring from persisted state
            if TerminalController::all().is_empty() && ivars.derived_config.get().initial_window {
                ivars.undo_manager.disableUndoRegistration();
                let _ = TerminalController::new_window(&ivars.ghostty, None);
                ivars.undo_manager.enableUndoRegistration();
            }
        }
    }

    fn should_terminate(&self, sender: &NSApplication) -> NSApplicationTerminateReply {
        let windows = sender.windows();
        if windows.is_empty() {
            return NSApplicationTerminateReply::TerminateNow;
        }

        // If we've already accepted to install an update, then we don't need to
        // confirm quit. The user is already expecting the update to happen.
        if self.ivars().update_controller.is_installing() {
            return NSApplicationTerminateReply::TerminateNow;
        }

        // This probably isn't fully safe. The isEmpty check above is aspirational, it doesn't
        // quite work with SwiftUI because windows are retained on close. So instead we check
        // if there are any that are visible. I'm guessing this breaks under certain scenarios.
        //
        // NOTE(mitchellh): I don't think we need this check at all anymore. I'm keeping it
        // here because I don't want to remove it in a patch release cycle but we should
        // target removing it soon.
        if windows.iter().all(|w| !w.isVisible()) {
            return NSApplicationTerminateReply::TerminateNow;
        }

        // If the user is shutting down, restarting, or logging out, we don't confirm quit.
        if let Some(event) =
            objc2_foundation::NSAppleEventManager::sharedAppleEventManager().currentAppleEvent()
        {
            // If all Ghostty windows are in the background (i.e. you Cmd-Q from the Cmd-Tab
            // view), then this is null. I don't know why (pun intended) but we have to
            // guard against it.
            let keyword = u32::from_be_bytes(*b"why?");
            // SAFETY: `attributeDescriptorForKeyword:` takes an AEKeyword (u32) and
            // returns an optional descriptor.
            let why: Option<Retained<objc2_foundation::NSAppleEventDescriptor>> =
                unsafe { msg_send![&*event, attributeDescriptorForKeyword: keyword] };
            if let Some(why) = why {
                const K_AE_SHUT_DOWN: u32 = u32::from_be_bytes(*b"shut");
                const K_AE_RESTART: u32 = u32::from_be_bytes(*b"rest");
                const K_AE_REALLY_LOG_OUT: u32 = u32::from_be_bytes(*b"rlgo");
                if matches!(
                    why.typeCodeValue(),
                    K_AE_SHUT_DOWN | K_AE_RESTART | K_AE_REALLY_LOG_OUT
                ) {
                    return NSApplicationTerminateReply::TerminateNow;
                }
            }
        }

        // If our app says we don't need to confirm, we can exit now.
        if !self.ivars().ghostty.needs_confirm_quit() {
            return NSApplicationTerminateReply::TerminateNow;
        }

        // We have some visible window. Show an app-wide modal to confirm quitting.
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str("Quit Ghostty?"));
        alert.setInformativeText(&NSString::from_str(
            "All terminal sessions will be terminated.",
        ));
        alert.addButtonWithTitle(&NSString::from_str("Close Ghostty"));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));
        alert.setAlertStyle(NSAlertStyle::Warning);
        if alert.runModal() == NSAlertFirstButtonReturn {
            NSApplicationTerminateReply::TerminateNow
        } else {
            NSApplicationTerminateReply::TerminateCancel
        }
    }

    /// `application(_:openFile:)`.
    fn open_file(&self, filename: &str) -> bool {
        // Ghostty will validate as well but we can avoid creating an entirely new
        // surface by doing our own validation here. We can also show a useful error
        // this way.
        let path = std::path::Path::new(filename);
        if !path.exists() {
            return false;
        }
        let is_directory = path.is_dir();

        // Set to true if confirmation is required before starting up the
        // new terminal.
        let mut requires_confirm = false;

        // Initialize the surface config which will be used to create the tab or window for the opened file.
        let mut config = SurfaceConfiguration::default();

        if is_directory {
            // When opening a directory, check the configuration to decide
            // whether to open in a new tab or new window.
            config.working_directory = Some(filename.to_string());
        } else {
            // Unconditionally require confirmation in the file execution case.
            // In the future I have ideas about making this more fine-grained if
            // we can not inherit of unsandboxed state. For now, we need to confirm
            // because there is a sandbox escape possible if a sandboxed application
            // somehow is tricked into `open`-ing a non-sandboxed application.
            requires_confirm = true;

            // When opening a file, we want to execute the file. To do this, we
            // don't override the command directly, because it won't load the
            // profile/rc files for the shell, which is super important on macOS
            // due to things like Homebrew. Instead, we set the command to
            // `<filename>; exit` which is what Terminal and iTerm2 do.
            config.initial_input = Some(format!("{}; exit\n", shell::quote(filename)));

            // For commands executed directly, we want to ensure we wait after exit
            // because in most cases scripts don't block on exit and we don't want
            // the window to just flash closed once complete.
            config.wait_after_command = true;

            // Set the parent directory to our working directory so that relative
            // paths in scripts work.
            config.working_directory = Some(
                NSString::from_str(filename)
                    .stringByDeletingLastPathComponent()
                    .to_string(),
            );
        }

        if requires_confirm {
            // Confirmation required. We use an app-wide NSAlert for now. In the future we
            // may want to show this as a sheet on the focused window (especially if we're
            // opening a tab). I'm not sure.
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(&NSString::from_str(&format!(
                "Allow Ghostty to execute \"{filename}\"?"
            )));
            alert.addButtonWithTitle(&NSString::from_str("Allow"));
            alert.addButtonWithTitle(&NSString::from_str("Cancel"));
            alert.setAlertStyle(NSAlertStyle::Warning);
            let response: NSModalResponse = alert.runModal();
            if response != NSAlertFirstButtonReturn {
                return false;
            }
        }

        match self.ivars().ghostty.config().macos_dock_drop_behavior() {
            MacDockDropBehavior::NewTab => {
                let parent = TerminalController::preferred_parent().and_then(|c| c.window());
                let _ = TerminalController::new_tab(
                    &self.ivars().ghostty,
                    parent.as_deref(),
                    Some(&config),
                );
            }
            MacDockDropBehavior::NewWindow => {
                let _ = TerminalController::new_window(&self.ivars().ghostty, Some(&config));
            }
        }

        true
    }

    /// Setup signal handlers.
    fn setup_signals(&self) {
        // Register a signal handler for config reloading. It appears that all
        // of this is required. I've commented each line because its a bit unclear.
        // Warning: signal handlers don't work when run via Xcode. They have to be
        // run on a real app bundle.

        // We need to ignore signals we register with makeSignalSource or they
        // don't seem to handle.
        // SAFETY: `signal` with SIG_IGN is always sound.
        unsafe { libc::signal(libc::SIGUSR2, libc::SIG_IGN) };

        // Make the signal source and register our event handle. We keep a weak
        // ref to ourself so we don't create a retain cycle.
        // SAFETY: `_dispatch_source_type_signal` is the documented signal
        // source type and the handle is a signal number.
        let sigusr2 = unsafe {
            DispatchSource::new(
                std::ptr::addr_of!(dispatch2::_dispatch_source_type_signal).cast_mut(),
                libc::SIGUSR2 as usize,
                0,
                Some(DispatchQueue::main()),
            )
        };
        let weak = Weak::from_retained(&self.retain());
        let handler = RcBlock::new(move || {
            let Some(this) = weak.load() else { return };
            log::info!("reloading configuration in response to SIGUSR2");
            this.ivars().ghostty.reload_config(false);
        });
        // SAFETY: the block is copied by libdispatch.
        unsafe { sigusr2.set_event_handler_with_block(RcBlock::as_ptr(&handler).cast()) };

        // The signal source starts unactivated, so we have to resume it once
        // we setup the event handler.
        sigusr2.resume();

        // We need to keep a strong reference to it so it isn't disabled.
        self.ivars().signals.borrow_mut().push(sigusr2);
    }

    fn weak_self(&self) -> Weak<Self> {
        Weak::from_retained(&self.retain())
    }

    // MARK: Notifications and Events

    fn install_local_monitor(&self) {
        let weak = self.weak_self();
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit passes a valid event.
            let event_ref = unsafe { event.as_ref() };
            let Some(this) = weak.load() else {
                return event.as_ptr();
            };
            match this.local_event_handler(event_ref) {
                true => event.as_ptr(),
                false => std::ptr::null_mut(),
            }
        });
        // SAFETY: the handler block has the signature AppKit expects (event in,
        // event or nil out) and is copied by AppKit.
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler)
        };
        *self.ivars().local_monitor.borrow_mut() = monitor;
    }

    /// `localEventHandler`: returns true if the event should continue (Swift
    /// returned the event), false if it was consumed (Swift returned nil).
    fn local_event_handler(&self, event: &NSEvent) -> bool {
        if event.r#type() == NSEventType::KeyDown {
            return self.local_event_key_down(event);
        }
        true
    }

    /// This handles events from the NSEvent.addLocalEventMonitor. We use this so
    /// we can get events without any terminal windows open.
    fn local_event_key_down(&self, event: &NSEvent) -> bool {
        let mtm = self.mtm();
        let app = NSApplication::sharedApplication(mtm);

        // If the tab overview is visible and escape is pressed, close it.
        // This can't POSSIBLY be right and is probably a FirstResponder problem
        // that we should handle elsewhere in our program. But this works and it
        // is guarded by the tab overview currently showing.
        if event.keyCode() == 0x35 {
            if let Some(window) = app.keyWindow() {
                if let Some(tab_group) = window.tabGroup() {
                    if tab_group.isOverviewVisible() {
                        window.toggleTabOverview(None);
                        return false;
                    }
                }
            }
        }

        // If we have a main window then we don't process any of the keys
        // because we let it capture and propagate.
        if app.mainWindow().is_some() {
            return true;
        }

        let ghostty = self.ivars().ghostty.clone();

        // If this event as-is would result in a key binding then we send it.
        if let Some(app_ptr) = ghostty.app_ptr() {
            let mut ghostty_event =
                crate::ghostty::ns_event::ghostty_key_event(event, GHOSTTY_ACTION_PRESS, None);
            let chars = event
                .characters()
                .map(|s| s.to_string())
                .unwrap_or_default();
            let text = CString::new(chars).unwrap_or_default();
            ghostty_event.text = text.as_ptr();
            // SAFETY: `app_ptr` is a live app handle and `text` outlives the calls.
            let matched = unsafe {
                if ghostty_app_key_is_binding(app_ptr, ghostty_event) {
                    ghostty_app_key(app_ptr, ghostty_event)
                } else {
                    false
                }
            };

            // If the key was handled by Ghostty we stop the event chain. If
            // the key wasn't handled then we let it fall through and continue
            // processing. This is important because some bindings may have no
            // affect at this scope.
            if matched {
                return false;
            }
        }

        // If this event would be handled by our menu then we do nothing.
        if let Some(main_menu) = app.mainMenu() {
            if main_menu.performKeyEquivalent(event) {
                return false;
            }
        }

        // If we reach this point then we try to process the key event
        // through the Ghostty key mechanism.

        // Ghostty must be loaded
        let Some(app_ptr) = ghostty.app_ptr() else {
            return true;
        };

        // Build our event input and call ghostty
        let key = crate::ghostty::ns_event::ghostty_key_event(event, GHOSTTY_ACTION_PRESS, None);
        // SAFETY: `app_ptr` is a live app handle; `text` is null.
        if unsafe { ghostty_app_key(app_ptr, key) } {
            // The key was used so we want to stop it from going to our Mac app
            log::debug!("local key event handled");
            return false;
        }

        true
    }

    fn install_observers(&self) {
        let ivars = self.ivars();
        let mtm = self.mtm();
        let _ = mtm;

        // AppKit notifications.
        let weak = self.weak_self();
        ivars.observers.borrow_mut().push(ns_notification::observe(
            ns_notification::name_window_did_become_key(),
            None,
            move |n| {
                if let Some(this) = weak.load() {
                    this.window_did_become_key(n);
                }
            },
        ));

        // Ghostty's own notifications.
        let nc = NotificationCenter::default_center();
        let mut bus = ivars.bus_observers.borrow_mut();

        let weak = self.weak_self();
        bus.push(
            nc.add_observer(QUICK_TERMINAL_DID_CHANGE_VISIBILITY, None, move |n| {
                if let Some(this) = weak.load() {
                    this.quick_terminal_did_change_visibility(n);
                }
            }),
        );
        let weak = self.weak_self();
        bus.push(
            nc.add_observer(gn::GHOSTTY_CONFIG_DID_CHANGE, None, move |n| {
                if let Some(this) = weak.load() {
                    this.ghostty_config_did_change_notification(n);
                }
            }),
        );
        let weak = self.weak_self();
        bus.push(nc.add_observer(gn::GHOSTTY_BELL_DID_RING, None, move |_| {
            if let Some(this) = weak.load() {
                this.ghostty_bell_did_ring();
            }
        }));
        let weak = self.weak_self();
        bus.push(nc.add_observer(
            gn::TERMINAL_WINDOW_BELL_DID_CHANGE_NOTIFICATION,
            None,
            move |n| {
                if let Some(this) = weak.load() {
                    this.terminal_window_has_bell(n);
                }
            },
        ));
        let weak = self.weak_self();
        bus.push(nc.add_observer(gn::GHOSTTY_NEW_WINDOW, None, move |n| {
            if let Some(this) = weak.load() {
                this.ghostty_new_window(n);
            }
        }));
        let weak = self.weak_self();
        bus.push(nc.add_observer(gn::GHOSTTY_NEW_TAB, None, move |n| {
            if let Some(this) = weak.load() {
                this.ghostty_new_tab(n);
            }
        }));
    }

    fn configure_user_notifications(&self) {
        let show = UNNotificationAction::actionWithIdentifier_title_options(
            &NSString::from_str(crate::ghostty::package::USER_NOTIFICATION_ACTION_SHOW),
            &NSString::from_str("Show"),
            UNNotificationActionOptions::empty(),
        );
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let category =
            UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
                &NSString::from_str(crate::ghostty::package::USER_NOTIFICATION_CATEGORY),
                &NSArray::from_retained_slice(&[show]),
                &NSArray::new(),
                UNNotificationCategoryOptions::CustomDismissAction,
            );
        center.setNotificationCategories(&NSSet::from_retained_slice(&[category]));
        center.setDelegate(Some(ProtocolObject::from_ref(self)));
    }

    /// Report the effective appearance to libghostty (the KVO callback body).
    fn report_color_scheme(&self) {
        let Some(app) = self.ivars().ghostty.app_ptr() else {
            return;
        };
        let appearance = NSApplication::sharedApplication(self.mtm()).effectiveAppearance();
        let scheme =
            if crate::helpers::extensions::ns_appearance::NSAppearanceExt::is_dark(&*appearance) {
                GHOSTTY_COLOR_SCHEME_DARK
            } else {
                GHOSTTY_COLOR_SCHEME_LIGHT
            };
        // SAFETY: `app` is a live app handle.
        unsafe { ghostty_app_set_color_scheme(app, scheme) };
    }

    fn window_did_become_key(&self, notification: &NSNotification) {
        let window = notification
            .object()
            .and_then(|o| o.downcast::<NSWindow>().ok());
        self.sync_float_on_top_menu(window.as_deref());
    }

    fn quick_terminal_did_change_visibility(&self, n: &Notification) {
        use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn};
        let Some(controller) = n
            .object
            .as_ref()
            .and_then(|o| o.downcast_ref::<Retained<QuickTerminalController>>())
        else {
            return;
        };
        if let Some(item) = self.menu_item(Outlet::MenuQuickTerminal) {
            item.setState(if controller.visible() {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }

    fn ghostty_config_did_change_notification(&self, n: &Notification) {
        // We only care if the configuration is a global configuration, not a surface one.
        if n.object.is_some() {
            return;
        }

        // Get our managed configuration object out
        let Some(config) = n.user_info.get::<Rc<Config>>(gn::GHOSTTY_CONFIG_CHANGE_KEY) else {
            return;
        };

        self.ghostty_config_did_change(config);
    }

    fn ghostty_bell_did_ring(&self) {
        let config = self.ivars().ghostty.config();
        let features = config.bell_features();
        if features.contains(BellFeatures::SYSTEM) {
            // SAFETY: NSBeep has no preconditions.
            unsafe { NSBeep() };
        }

        if features.contains(BellFeatures::AUDIO) {
            if let Some(path) = config.bell_audio_path() {
                let sound = NSSound::initWithContentsOfFile_byReference(
                    NSSound::alloc(),
                    &NSString::from_str(&path.path),
                    false,
                );
                if let Some(sound) = sound {
                    sound.setVolume(config.bell_audio_volume());
                    sound.play();
                }
            }
        }

        if features.contains(BellFeatures::ATTENTION) {
            // Bounce the dock icon if we're not focused.
            NSApplication::sharedApplication(self.mtm()).requestUserAttention(
                objc2_app_kit::NSRequestUserAttentionType::InformationalRequest,
            );
        }
    }

    fn terminal_window_has_bell(&self, n: &Notification) {
        if n.object
            .as_ref()
            .and_then(|o| o.downcast_ref::<Retained<BaseTerminalController>>())
            .is_none()
        {
            return;
        }
        self.sync_dock_badge();
    }

    fn sync_dock_badge(&self) {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let center2 = center.clone();
        let handler = RcBlock::new(
            move |settings: NonNull<objc2_user_notifications::UNNotificationSettings>| {
                // SAFETY: valid settings object for the duration of the callback.
                let settings = unsafe { settings.as_ref() };
                match settings.authorizationStatus() {
                    UNAuthorizationStatus::Authorized => {
                        // If we're authorized and allow badges, then set the badge.
                        if settings.badgeSetting() == UNNotificationSetting::Enabled {
                            DispatchQueue::main().exec_async(set_dock_badge_on_main);
                        }
                    }
                    UNAuthorizationStatus::NotDetermined => {
                        // Not determined yet, request authorization for badge
                        let completion = RcBlock::new(
                            |granted: objc2::runtime::Bool,
                             error: *mut objc2_foundation::NSError| {
                                if !error.is_null() {
                                    // SAFETY: non-null error pointer provided by the framework.
                                    let e = unsafe { &*error };
                                    log::warn!(
                                        "Error requesting badge authorization: {}",
                                        e.localizedDescription()
                                    );
                                    return;
                                }
                                if granted.as_bool() {
                                    // Permission granted, set the badge
                                    DispatchQueue::main().exec_async(set_dock_badge_on_main);
                                }
                            },
                        );
                        center2.requestAuthorizationWithOptions_completionHandler(
                            UNAuthorizationOptions::Badge,
                            &completion,
                        );
                    }
                    // In these known non-authorized states, do not attempt to set the badge;
                    // future unknown states do nothing either.
                    _ => {}
                }
            },
        );
        center.getNotificationSettingsWithCompletionHandler(&handler);
    }

    fn ghostty_new_window(&self, n: &Notification) {
        let config = n
            .user_info
            .get::<SurfaceConfiguration>(gn::NEW_SURFACE_CONFIG_KEY);
        let _ = TerminalController::new_window(&self.ivars().ghostty, config);
    }

    fn ghostty_new_tab(&self, n: &Notification) {
        let Some(surface_view) = n
            .object
            .as_ref()
            .and_then(|o| o.downcast_ref::<Retained<SurfaceView>>())
        else {
            return;
        };
        let Some(window) = surface_view.window() else {
            return;
        };

        // We only want to listen to new tabs if the focused parent is
        // a regular terminal controller.
        let is_terminal = window
            .windowController()
            .is_some_and(|c| c.downcast::<TerminalController>().is_ok());
        if !is_terminal {
            return;
        }

        let config = n
            .user_info
            .get::<SurfaceConfiguration>(gn::NEW_SURFACE_CONFIG_KEY);
        let _ = TerminalController::new_tab(&self.ivars().ghostty, Some(&window), config);
    }

    /// `ghosttyConfigDidChange(config:)`.
    fn ghostty_config_did_change(&self, config: &Rc<Config>) {
        let ivars = self.ivars();
        let mtm = self.mtm();
        let app = NSApplication::sharedApplication(mtm);

        // Update the config we need to store
        ivars.derived_config.set(DerivedConfig::new(config));

        // Depending on the "window-save-state" setting we have to set the NSQuitAlwaysKeepsWindows
        // configuration. This is the only way to carefully control whether macOS invokes the
        // state restoration system.
        let ud = user_defaults::ghostty();
        let key = NSString::from_str("NSQuitAlwaysKeepsWindows");
        match config.window_save_state().as_str() {
            "never" => ud.setBool_forKey(false, &key),
            "always" => ud.setBool_forKey(true, &key),
            _ => ud.removeObjectForKey(&key),
        }

        // Sync our auto-update settings. If SUEnableAutomaticChecks (in our Info.plist) is
        // explicitly false (NO), auto-updates are disabled. Otherwise, we use the behavior
        // defined by our "auto-update" configuration (if set) or fall back to Sparkle
        // user-based defaults.
        let uc = &ivars.update_controller;
        if super::support::bundle_info_bool("SUEnableAutomaticChecks") == Some(false) {
            uc.automatically_checks_for_updates.set(false);
            uc.automatically_downloads_updates.set(false);
        } else if let Some(auto_update) = config.auto_update() {
            uc.automatically_checks_for_updates
                .set(auto_update == AutoUpdate::Check || auto_update == AutoUpdate::Download);
            uc.automatically_downloads_updates
                .set(auto_update == AutoUpdate::Download);
            // To test `auto-update` easily, uncomment the line below and
            // delete `SUEnableAutomaticChecks` in Ghostty-Info.plist.
            //
            // Note: When `auto-update = download`, you may need to
            // `Clean Build Folder` if a background install has already begun.
            // updateController.updater.checkForUpdatesInBackground()
        }

        // Config could change keybindings, so update everything that depends on that
        self.sync_menu_shortcuts(config);
        for c in TerminalController::all() {
            c.relabel_tabs();
        }

        // Update our badge since config can change what we show.
        self.sync_dock_badge();

        // Config could change window appearance. We wrap this in an async queue because when
        // this is called as part of application launch it can deadlock with an internal
        // AppKit mutex on the appearance.
        let weak = self.weak_self();
        let config_for_appearance = config.clone();
        main_async_local(move || {
            if let Some(this) = weak.load() {
                this.sync_appearance(&config_for_appearance);
            }
        });

        // Decide whether to hide/unhide app from dock and app switcher
        match config.macos_hidden() {
            MacHidden::Never => {
                app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            }
            MacHidden::Always => {
                app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            }
        }

        // If we have configuration errors, we need to show them.
        let c = ConfigurationErrorsController::shared_instance();
        c.set_errors(config.errors());
        if !c.errors().is_empty() && !c.is_window_visible() {
            c.show_window();
        }

        // We need to handle our global event tap depending on if there are global
        // events that we care about in Ghostty.
        let has_global = ivars
            .ghostty
            .app_ptr()
            // SAFETY: live app handle.
            .is_some_and(|a| unsafe { ghostty_app_has_global_keybinds(a) });
        if has_global {
            if self.time_since_launch() > 5.0 {
                // If the process has been running for awhile we enable right away
                // because no windows are likely to pop up.
                GlobalEventTap::shared().enable();
            } else {
                // If the process just started, we wait a couple seconds to allow
                // the initial windows and so on to load so our permissions dialog
                // doesn't get buried.
                crate::helpers::dispatch::main_async_after(
                    std::time::Duration::from_secs(2),
                    || GlobalEventTap::shared().enable(),
                );
            }
        } else {
            GlobalEventTap::shared().disable();
        }

        self.update_app_icon(config);
    }

    /// Sync the appearance of our app with the theme specified in the config.
    fn sync_appearance(&self, config: &Config) {
        let app = NSApplication::sharedApplication(self.mtm());
        let appearance =
            crate::helpers::extensions::ns_appearance::appearance_from_ghostty_config(config);
        app.setAppearance(appearance.as_deref());
    }

    fn update_app_icon(&self, config: &Config) {
        // Since this is called after `DockTilePlugin` has been running,
        // clean it up here to trigger a correct update of the current config.
        user_defaults::ghostty().removeObjectForKey(&NSString::from_str("CustomGhosttyIcon"));
        // TODO(MA4): `DispatchQueue.global().async { UserDefaults.ghostty.appIcon =
        // AppIcon(config: config); post .ghosttyIconDidChange on the distributed
        // notification center }` (Custom App Icon feature).
        let _ = config;
        crate::features::custom_app_icon::post_icon_did_change();
    }

    // MARK: - Global State

    /// `setSecureInput(_:)`.
    pub fn set_secure_input(&self, mode: SetSecureInput) {
        use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn};
        let input = SecureInput::shared();
        match mode {
            SetSecureInput::On => input.set_global(true),
            SetSecureInput::Off => input.set_global(false),
            SetSecureInput::Toggle => input.set_global(!input.global()),
        }
        if let Some(item) = self.menu_item(Outlet::MenuSecureInput) {
            item.setState(if input.global() {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
        user_defaults::ghostty().setBool_forKey(input.global(), &NSString::from_str("SecureInput"));
    }

    /// `@IBAction checkForUpdates` (called by Ghostty.App's `checkForUpdates` action).
    pub fn check_for_updates(&self) {
        self.ivars().update_controller.check_for_updates();
    }

    /// `@IBAction closeAllWindows` (called by Ghostty.App's action).
    pub fn close_all_windows(&self) {
        TerminalController::close_all_windows();
        AboutController::shared().hide();
    }

    /// `@IBAction toggleQuickTerminal`.
    pub fn toggle_quick_terminal(&self) {
        self.quick_controller().toggle();
    }

    /// `@IBAction toggleVisibility`.
    pub fn toggle_visibility(&self) {
        let app = NSApplication::sharedApplication(self.mtm());
        // If we have focus, then we hide all windows.
        if app.isActive() {
            // Toggle visibility doesn't do anything if the focused window is native
            // fullscreen. This is only relevant if Ghostty is active.
            let Some(key_window) = app.keyWindow() else {
                return;
            };
            if key_window
                .styleMask()
                .contains(NSWindowStyleMask::FullScreen)
            {
                return;
            }

            app.hide(None);
            return;
        }

        // If we're not active, we want to become active
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);

        // Bring all windows to the front. Note: we don't use NSApp.unhide because
        // that will unhide ALL hidden windows. We want to only bring forward the
        // ones that we hid.
        if let Some(state) = self.ivars().hidden_state.borrow_mut().take() {
            state.restore();
        }
    }

    fn set_as_default_terminal(&self) {
        let mtm = self.mtm();
        let handler = RcBlock::new(|error: *mut objc2_foundation::NSError| {
            if error.is_null() {
                return;
            }
            // SAFETY: non-null error pointer provided by the framework.
            let desc = unsafe { &*error }.localizedDescription().to_string();
            DispatchQueue::main().exec_async(move || {
                let Some(mtm) = MainThreadMarker::new() else {
                    return;
                };
                let alert = NSAlert::new(mtm);
                alert.setMessageText(&NSString::from_str("Failed to Set Default Terminal"));
                alert.setInformativeText(&NSString::from_str(&format!(
                    "Ghostty could not be set as the default terminal application.\n\nError: {desc}"
                )));
                alert.setAlertStyle(NSAlertStyle::Warning);
                alert.runModal();
            });
        });
        let _ = mtm;
        super::support::set_default_terminal(&handler);
    }
}

// MARK: Menu

impl AppDelegate {
    fn reload_dock_menu(&self) {
        let mtm = self.mtm();
        let dock_menu = &self.ivars().dock_menu;
        dock_menu.removeAllItems();
        for (title, selector) in DOCK_MENU {
            // SAFETY: valid title/selector/key equivalent.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(main_menu::selector(selector)),
                    &NSString::from_str(""),
                )
            };
            dock_menu.addItem(&item);
        }
    }

    /// Setup all the images for our menu items.
    fn setup_menu_images(&self) {
        // Note: This COULD Be done all in the xib file, but I find it easier to
        // modify this stuff as code.
        for (outlet, symbol) in MENU_IMAGES {
            if let Some(item) = self.menu_item(*outlet) {
                item.set_image_if_desired(symbol);
            }
        }
    }

    /// Sync all of our menu item keyboard shortcuts with the Ghostty configuration.
    fn sync_menu_shortcuts(&self, config: &Config) {
        if self.ivars().ghostty.readiness() != Readiness::Ready {
            return;
        }

        // Reset our shortcut index since we're about to rebuild all menu bindings.
        self.ivars().menu_items_by_shortcut.borrow_mut().clear();

        for (action, outlet) in SYNCED_SHORTCUTS {
            self.sync_menu_shortcut(config, action, self.menu_item(*outlet).as_deref());
        }

        // This menu item is NOT synced with the configuration because it disables macOS
        // global fullscreen keyboard shortcut. The shortcut in the Ghostty config will continue
        // to work but it won't be reflected in the menu item.
        //
        // syncMenuShortcut(config, action: "toggle_fullscreen", menuItem: self.menuToggleFullScreen)

        // Dock menu
        self.reload_dock_menu();
    }

    /// Syncs a single menu shortcut for the given action. The action string is the same
    /// action string used for the Ghostty configuration.
    fn sync_menu_shortcut(&self, config: &Config, action: &str, menu_item: Option<&NSMenuItem>) {
        let Some(menu) = menu_item else { return };

        let Some(shortcut) = config.keyboard_shortcut(action) else {
            // No shortcut, clear the menu item
            menu.setKeyEquivalent(&NSString::from_str(""));
            menu.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
            return;
        };

        let key_equivalent = shortcut.key.character.to_string();
        let modifier_mask =
            crate::helpers::extensions::event_modifiers::ns_flags_from_swiftui(shortcut.modifiers);
        menu.setKeyEquivalent(&NSString::from_str(&key_equivalent));
        menu.setKeyEquivalentModifierMask(modifier_mask);

        // Build a direct lookup for key-equivalent dispatch so we don't need to
        // linearly walk the full menu hierarchy at event time.
        let Some(key) = MenuShortcutKey::new(&key_equivalent, modifier_mask.0) else {
            return;
        };

        // Later registrations intentionally override earlier ones for the same key.
        // SAFETY: the item is alive (we hold `menu`); Weak::new needs a retained ref.
        let retained = unsafe { Retained::retain(menu as *const NSMenuItem as *mut NSMenuItem) };
        if let Some(retained) = retained {
            self.ivars()
                .menu_items_by_shortcut
                .borrow_mut()
                .insert(key, Weak::from_retained(&retained));
        }
    }

    /// Attempts to perform a menu key equivalent only for menu items that represent
    /// Ghostty keybind actions. This is important because it lets our surface dispatch
    /// bindings through the menu so they flash but also lets our surface override macOS built-ins
    /// like Cmd+H.
    pub fn perform_ghostty_binding_menu_key_equivalent(&self, event: &NSEvent) -> bool {
        // Convert this event into the same normalized lookup key we use when
        // syncing menu shortcuts from configuration.
        let Some(chars) = event.charactersIgnoringModifiers() else {
            return false;
        };
        let Some(key) = MenuShortcutKey::new(&chars.to_string(), event.modifierFlags().0) else {
            return false;
        };

        // If we don't have an entry for this key combo, no Ghostty-owned
        // menu shortcut exists for this event.
        let Some(weak_item) = self
            .ivars()
            .menu_items_by_shortcut
            .borrow()
            .get(&key)
            .map(|w| w.load())
        else {
            return false;
        };

        // Weak references can be nil if a menu item was deallocated after sync.
        let Some(item) = weak_item else {
            self.ivars()
                .menu_items_by_shortcut
                .borrow_mut()
                .remove(&key);
            return false;
        };

        // SAFETY: plain property getter.
        let Some(parent_menu) = (unsafe { item.menu() }) else {
            return false;
        };

        // Keep enablement state fresh in case menu validation hasn't run yet.
        parent_menu.update();
        if !item.isEnabled() {
            return false;
        }

        let index = parent_menu.indexOfItem(&item);
        if index < 0 {
            return false;
        }

        parent_menu.performActionForItemAtIndex(index);
        true
    }

    // MARK: Floating Windows

    /// `syncFloatOnTopMenu`.
    pub fn sync_float_on_top_menu(&self, window: Option<&NSWindow>) {
        use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn};
        let Some(item) = self.menu_item(Outlet::MenuFloatOnTop) else {
            return;
        };
        let key = NSApplication::sharedApplication(self.mtm()).keyWindow();
        let window: Option<&NSWindow> = window.or(key.as_deref());
        let terminal = window.and_then(|w| {
            let obj: &AnyObject = w;
            obj.downcast_ref::<TerminalWindow>()
        });
        let Some(window) = terminal else {
            // If some other window became key we always turn this off
            item.setState(NSControlStateValueOff);
            return;
        };
        item.setState(if window.level() == FLOATING_WINDOW_LEVEL {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
    }

    // MARK: NSMenuItemValidation

    fn validate(&self, item: &NSMenuItem) -> bool {
        let Some(action) = item.action() else {
            return true;
        };
        let app = NSApplication::sharedApplication(self.mtm());
        if action == sel!(setAsDefaultTerminal:) {
            let current = NSWorkspace::sharedWorkspace().default_terminal();
            let bundle = objc2_foundation::NSBundle::mainBundle().bundleURL();
            return current.map(|u| u.isEqual(Some(&bundle))) != Some(true);
        }
        if action == sel!(floatOnTop:) || action == sel!(useAsDefault:) {
            // Float on top items only active if the key window is a primary
            // terminal window (not quick terminal).
            return app.keyWindow().is_some_and(|w| {
                let obj: &AnyObject = &w;
                obj.downcast_ref::<TerminalWindow>().is_some()
            });
        }
        let um = &self.ivars().undo_manager;
        if action == sel!(undo:) {
            if um.canUndo() {
                item.setTitle(&NSString::from_str(&format!(
                    "Undo {}",
                    um.undoActionName()
                )));
            } else {
                item.setTitle(&NSString::from_str("Undo"));
            }
            return um.canUndo();
        }
        if action == sel!(redo:) {
            if um.canRedo() {
                item.setTitle(&NSString::from_str(&format!(
                    "Redo {}",
                    um.redoActionName()
                )));
            } else {
                item.setTitle(&NSString::from_str("Redo"));
            }
            return um.canRedo();
        }
        true
    }
}

/// `setDockBadge()` on the main thread.
fn set_dock_badge_on_main() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(delegate) = AppDelegate::shared(mtm) else {
        return;
    };
    delegate.set_dock_badge();
}

impl AppDelegate {
    fn set_dock_badge(&self) {
        let app = NSApplication::sharedApplication(self.mtm());
        let bell_count = app
            .windows()
            .iter()
            .filter_map(|w| w.windowController())
            .filter_map(|c| c.downcast::<BaseTerminalController>().ok())
            .filter(|c| c.bell())
            .count();
        let wants_badge = self
            .ivars()
            .ghostty
            .config()
            .bell_features()
            .contains(BellFeatures::ATTENTION)
            && bell_count > 0;
        let label = if wants_badge {
            Some(if bell_count > 99 {
                "99+".to_string()
            } else {
                bell_count.to_string()
            })
        } else {
            None
        };
        let tile = app.dockTile();
        tile.setBadgeLabel(label.map(|l| NSString::from_str(&l)).as_deref());
        tile.display();
    }
}

/// `DispatchQueue.main.async { ... }` for closures that capture main-thread
/// only state (weak refs to AppKit objects). Must be called on the main thread.
fn main_async_local(f: impl FnOnce() + 'static) {
    /// Wrapper asserting the closure is only run on the main queue.
    struct MainOnly<F>(F);
    // SAFETY: the closure is created on the main thread and only ever executed
    // on the main queue (i.e. the main thread), so its captured state never
    // crosses threads.
    unsafe impl<F> Send for MainOnly<F> {}
    let wrapped = MainOnly(f);
    DispatchQueue::main().exec_async(move || {
        let w = wrapped;
        (w.0)();
    });
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSBeep();
}
