//! `Ghostty.App` (`Ghostty.App.swift`): the app-global libghostty state.
//!
//! Owns the `ghostty_app_t`, the app configuration and the runtime callbacks
//! libghostty uses to talk to the host (wakeup, action, clipboard, close
//! surface). In the Swift app these callbacks are C function pointers into
//! `static func`s of `Ghostty.App`; here they are `extern "C" fn`s in this
//! module passed in the very same `ghostty_runtime_config_s`.
//!
//! Threading: all state is main-thread only (`Rc`/`RefCell`/`Published`).
//! The single exception is libghostty's `wakeup` callback which may be invoked
//! from any thread: it only captures the *address* of the App allocation and
//! hops to the main queue, where the live App is re-resolved through a
//! main-thread registry ([`REGISTRY`]) before it is touched.
//!
//! `Ghostty.Delegate` / `GhosttyAppDelegate` (protocol with
//! `findSurface(forUUID:)`) is not a trait here: the only conformer is the
//! `AppDelegate` class, held weakly by [`App::set_delegate`].
#![allow(unsafe_code)]
// The App operations mirror Swift instance methods (some do not use `self`).
#![allow(clippy::unused_self)]
// `consts` is the generated list of header enumerators; the notification
// module is the Swift `Notification.Name` namespace.
#![allow(clippy::wildcard_imports)]
// The C action structs are passed by reference like the Swift handlers do, and
// the surface handles are opaque pointers owned by libghostty that callers get
// from libghostty (they are never dereferenced here).
#![allow(clippy::trivially_copy_pass_by_ref, clippy::not_unsafe_ptr_arg_deref)]

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr::NonNull,
    rc::{Rc, Weak as RcWeak},
    time::Duration,
};

use block2::RcBlock;
use dispatch2::{DispatchQueue, MainThreadBound};
use log::{debug, error, info, warn};
use objc2::{
    AnyThread,
    MainThreadMarker,
    rc::{Retained, Weak},
};
use objc2_app_kit::{
    NSAlert,
    NSAlertStyle,
    NSApplication,
    NSApplicationDidBecomeActiveNotification,
    NSApplicationDidResignActiveNotification,
    NSFloatingWindowLevel,
    NSNormalWindowLevel,
    NSPasteboard,
    NSPasteboardTypeString,
    NSTextInputContextKeyboardSelectionDidChangeNotification,
    NSWindow,
    NSWorkspace,
    NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{NSArray, NSSize, NSString, NSURL, NSUUID};
use objc2_user_notifications::{
    UNAuthorizationOptions,
    UNAuthorizationStatus,
    UNNotification,
    UNNotificationResponse,
    UNNotificationSettings,
    UNUserNotificationCenter,
};

use crate::{
    app::app_delegate::AppDelegate,
    features::terminal::{BaseTerminalController, TerminalWindow},
    ghostty::{
        action::{
            ColorChange,
            KeyTable,
            MoveTab,
            OpenUrl,
            OpenUrlKind,
            ProgressReport,
            ProgressReportState,
            PromptTitle,
            Scrollbar,
            StartSearch,
        },
        capi::{
            consts::*,
            ghostty_action_s,
            ghostty_app_free,
            ghostty_app_keyboard_changed,
            ghostty_app_needs_confirm_quit,
            ghostty_app_new,
            ghostty_app_set_focus,
            ghostty_app_t,
            ghostty_app_tick,
            ghostty_app_update_config,
            ghostty_app_userdata,
            ghostty_clipboard_content_s,
            ghostty_config_open_path,
            ghostty_runtime_config_s,
            ghostty_surface_app,
            ghostty_surface_binding_action,
            ghostty_surface_complete_clipboard_request,
            ghostty_surface_inherited_config,
            ghostty_surface_request_close,
            ghostty_surface_split,
            ghostty_surface_split_equalize,
            ghostty_surface_split_focus,
            ghostty_surface_split_resize,
            ghostty_surface_t,
            ghostty_surface_update_config,
            ghostty_surface_userdata,
            ghostty_target_s,
        },
        config::Config,
        duration_format,
        fullscreen_mode,
        input,
        notification::*,
        package::{
            AllocatedString,
            ClipboardContent,
            ClipboardRequest,
            SetFloatWindow,
            SetSecureInput,
            SplitFocusDirection,
            SplitResizeDirection,
            USER_NOTIFICATION_ACTION_SHOW,
        },
        surface_configuration::SurfaceConfiguration,
        surface_view::{SearchState, SurfaceView, move_focus},
    },
    helpers::{
        extensions::{
            ns_notification::{NsObserver, observe},
            ns_pasteboard::{self, NSPasteboardExt, pasteboard_type_from_mime},
            ns_workspace::NSWorkspaceExt,
        },
        notification_center::{Name, NotificationCenter, Sender, UserInfo},
        published::Published,
    },
};

// MARK: Main-thread helpers

/// `DispatchQueue.main.async { ... }` for non-`Send` closures. Must be called
/// from the main thread (the closure also runs there).
pub fn dispatch_main_async(f: impl FnOnce() + 'static) {
    let mtm = crate::helpers::main_thread();
    let bound = MainThreadBound::new(f, mtm);
    DispatchQueue::main().exec_async(move || {
        let mtm = MainThreadMarker::new().expect("main queue runs on the main thread");
        (bound.into_inner(mtm))();
    });
}

// MARK: Registry

thread_local! {
    /// Live Apps by the address of their allocation (the `userdata` handed to
    /// libghostty). Main thread only.
    static REGISTRY: RefCell<HashMap<usize, RcWeak<App>>> = RefCell::new(HashMap::new());
}

fn registered(addr: usize) -> Option<Rc<App>> {
    REGISTRY.with(|r| r.borrow().get(&addr).and_then(RcWeak::upgrade))
}

// MARK: App

/// `Ghostty.App.Readiness`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Loading,
    Error,
    Ready,
}

/// `Ghostty.App.FontSizeModification`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontSizeModification {
    Increase(i32),
    Decrease(i32),
    Reset,
}

/// `Ghostty.App`.
pub struct App {
    /// Optional delegate (`weak var delegate`).
    delegate: RefCell<Option<Weak<AppDelegate>>>,
    /// The readiness value of the state.
    readiness: Published<Readiness>,
    /// The global app configuration. This defines the app level configuration
    /// plus any behavior for new windows, tabs, etc. Note that when creating a
    /// new window, it may inherit some configuration (i.e. font size) from the
    /// previously focused window. This would override this.
    config: Published<Rc<Config>>,
    /// Preferred config file than the default ones.
    config_path: Option<String>,
    /// The ghostty app instance. We only have one of these for the entire app.
    handle: Cell<ghostty_app_t>,
    /// The AppKit observers (removed on drop).
    observers: RefCell<Vec<NsObserver>>,
}

impl App {
    /// `init(configPath:)`.
    #[must_use]
    pub fn new(config_path: Option<&str>) -> Rc<App> {
        // Initialize the global configuration.
        let config = Config::new(config_path, true);
        let loaded = config.loaded();
        let this = Rc::new(App {
            delegate: RefCell::new(None),
            readiness: Published::new(Readiness::Loading),
            config: Published::new(Rc::new(config)),
            config_path: config_path.map(str::to_owned),
            handle: Cell::new(std::ptr::null_mut()),
            observers: RefCell::new(Vec::new()),
        });
        if !loaded {
            this.readiness.set(Readiness::Error);
            return this;
        }

        // The userdata handed to libghostty is the address of this allocation.
        let userdata = Rc::as_ptr(&this) as *mut c_void;
        REGISTRY.with(|r| {
            r.borrow_mut()
                .insert(userdata as usize, Rc::downgrade(&this))
        });

        // Create our "runtime" config. The "runtime" is the configuration that
        // ghostty uses to interface with the application runtime environment.
        let runtime_cfg = ghostty_runtime_config_s {
            userdata,
            supports_selection_clipboard: true,
            wakeup_cb: Some(wakeup_cb),
            action_cb: Some(action_cb),
            read_clipboard_cb: Some(read_clipboard_cb),
            confirm_read_clipboard_cb: Some(confirm_read_clipboard_cb),
            write_clipboard_cb: Some(write_clipboard_cb),
            close_surface_cb: Some(close_surface_cb),
        };

        // Create the ghostty app.
        // SAFETY: valid runtime config and config handle; the callbacks above
        // are sound for the userdata we pass.
        let app = unsafe { ghostty_app_new(&raw const runtime_cfg, this.config().as_ptr()) };
        if app.is_null() {
            error!("ghostty_app_new failed");
            this.readiness.set(Readiness::Error);
            return this;
        }
        this.handle.set(app);

        // Set our initial focus state.
        let mtm = crate::helpers::main_thread();
        // SAFETY: valid app handle.
        unsafe { ghostty_app_set_focus(app, NSApplication::sharedApplication(mtm).isActive()) };

        let mut observers = Vec::new();
        let weak = Rc::downgrade(&this);
        // Called when the selected keyboard changes. We have to notify Ghostty
        // so that it can reload the keyboard mapping for input.
        // SAFETY: statics provided by AppKit.
        let kb_name = unsafe { NSTextInputContextKeyboardSelectionDidChangeNotification };
        observers.push(observe(kb_name, None, {
            let weak = weak.clone();
            move |_| {
                if let Some(this) = weak.upgrade()
                    && let Some(app) = this.app_ptr()
                {
                    // SAFETY: valid app handle.
                    unsafe { ghostty_app_keyboard_changed(app) };
                }
            }
        }));
        // Called when the app becomes active.
        // SAFETY: statics provided by AppKit.
        let active = unsafe { NSApplicationDidBecomeActiveNotification };
        observers.push(observe(active, None, {
            let weak = weak.clone();
            move |_| {
                if let Some(this) = weak.upgrade()
                    && let Some(app) = this.app_ptr()
                {
                    // SAFETY: valid app handle.
                    unsafe { ghostty_app_set_focus(app, true) };
                }
            }
        }));
        // Called when the app becomes inactive.
        // SAFETY: statics provided by AppKit.
        let resign = unsafe { NSApplicationDidResignActiveNotification };
        observers.push(observe(resign, None, {
            move |_| {
                if let Some(this) = weak.upgrade()
                    && let Some(app) = this.app_ptr()
                {
                    // SAFETY: valid app handle.
                    unsafe { ghostty_app_set_focus(app, false) };
                }
            }
        }));
        *this.observers.borrow_mut() = observers;

        this.readiness.set(Readiness::Ready);
        this
    }

    // MARK: State

    /// `delegate = ...`.
    pub fn set_delegate(&self, delegate: Weak<AppDelegate>) {
        *self.delegate.borrow_mut() = Some(delegate);
    }

    fn delegate(&self) -> Option<Retained<AppDelegate>> {
        self.delegate.borrow().as_ref().and_then(Weak::load)
    }

    /// `readiness`.
    #[must_use]
    pub fn readiness(&self) -> Readiness {
        self.readiness.get()
    }

    /// `$readiness`.
    #[must_use]
    pub fn readiness_published(&self) -> Published<Readiness> {
        self.readiness.clone()
    }

    /// `config`.
    #[must_use]
    pub fn config(&self) -> Rc<Config> {
        self.config.get()
    }

    /// `$config`.
    #[must_use]
    pub fn config_published(&self) -> Published<Rc<Config>> {
        self.config.clone()
    }

    /// `app` (the `ghostty_app_t`), if created.
    #[must_use]
    pub fn app_ptr(&self) -> Option<ghostty_app_t> {
        let p = self.handle.get();
        if p.is_null() { None } else { Some(p) }
    }

    /// True if we need to confirm before quitting.
    #[must_use]
    pub fn needs_confirm_quit(&self) -> bool {
        match self.app_ptr() {
            // SAFETY: valid app handle.
            Some(app) => unsafe { ghostty_app_needs_confirm_quit(app) },
            None => false,
        }
    }

    // MARK: App Operations

    /// `appTick()`.
    pub fn app_tick(&self) {
        let Some(app) = self.app_ptr() else { return };
        // SAFETY: valid app handle.
        unsafe { ghostty_app_tick(app) };
    }

    /// `static func openConfig()`.
    pub fn open_config() {
        // SAFETY: returns an owned string freed by `AllocatedString`.
        let s = AllocatedString::new(ghostty_config_open_path()).string();
        if s.is_empty() {
            return;
        }
        // Deviation: Swift passes `fileURL.count` (characters) as the byte
        // length, truncating URLs with non-ASCII paths; we use the full URL.
        let file_url = NSURL::fileURLWithPath(&NSString::from_str(&s));
        let url = file_url
            .absoluteString()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let _ = open_url(&OpenUrl {
            kind: OpenUrlKind::Text,
            url,
        });
    }

    /// `reloadConfig(soft:)`: reload the configuration.
    pub fn reload_config(&self, soft: bool) {
        let Some(app) = self.app_ptr() else { return };

        // Soft updates just call with our existing config.
        if soft {
            // SAFETY: valid handles.
            unsafe { ghostty_app_update_config(app, self.config().as_ptr()) };
            return;
        }

        // Hard or full updates have to reload the full configuration.
        let new_config = Config::new(self.config_path.as_deref(), true);
        if !new_config.loaded() {
            warn!("failed to reload configuration");
            return;
        }

        // SAFETY: valid handles.
        unsafe { ghostty_app_update_config(app, new_config.as_ptr()) };
        // applied config will be updated in `config_change`.
    }

    /// `reloadConfig(surface:soft:)`.
    pub fn reload_config_surface(&self, surface: ghostty_surface_t, soft: bool) {
        // Soft updates just call with our existing config.
        if soft {
            // SAFETY: valid handles.
            unsafe { ghostty_surface_update_config(surface, self.config().as_ptr()) };
            return;
        }

        // Hard or full updates have to reload the full configuration.
        // NOTE: We never set this on self.config because this is a
        // surface-only config. We free it after the call.
        let new_config = Config::new(self.config_path.as_deref(), true);
        if !new_config.loaded() {
            warn!("failed to reload configuration");
            return;
        }

        // SAFETY: valid handles.
        unsafe { ghostty_surface_update_config(surface, new_config.as_ptr()) };
    }

    /// Request that the given surface is closed. This will trigger the full
    /// normal surface close event cycle which will call our close surface
    /// callback.
    pub fn request_close(&self, surface: ghostty_surface_t) {
        // SAFETY: valid surface handle (caller).
        unsafe { ghostty_surface_request_close(surface) };
    }

    fn binding_action(surface: ghostty_surface_t, action: &str) {
        // SAFETY: valid surface handle; pointer/length describe `action`.
        if !unsafe { ghostty_surface_binding_action(surface, action.as_ptr().cast(), action.len()) }
        {
            warn!("action failed action={action}");
        }
    }

    /// `newTab(surface:)`.
    pub fn new_tab(&self, surface: ghostty_surface_t) {
        Self::binding_action(surface, "new_tab");
    }

    /// `newWindow(surface:)`.
    pub fn new_window(&self, surface: ghostty_surface_t) {
        Self::binding_action(surface, "new_window");
    }

    /// `split(surface:direction:)` (`ghostty_action_split_direction_e`).
    pub fn split(&self, surface: ghostty_surface_t, direction: c_int) {
        // SAFETY: valid surface handle.
        unsafe { ghostty_surface_split(surface, direction) };
    }

    /// `splitMoveFocus(surface:direction:)`.
    pub fn split_move_focus(&self, surface: ghostty_surface_t, direction: SplitFocusDirection) {
        // SAFETY: valid surface handle.
        unsafe { ghostty_surface_split_focus(surface, direction.to_native()) };
    }

    /// `splitResize(surface:direction:amount:)`.
    pub fn split_resize(
        &self,
        surface: ghostty_surface_t,
        direction: SplitResizeDirection,
        amount: u16,
    ) {
        // SAFETY: valid surface handle.
        unsafe { ghostty_surface_split_resize(surface, direction.to_native(), amount) };
    }

    /// `splitEqualize(surface:)`.
    pub fn split_equalize(&self, surface: ghostty_surface_t) {
        // SAFETY: valid surface handle.
        unsafe { ghostty_surface_split_equalize(surface) };
    }

    /// `splitToggleZoom(surface:)`.
    pub fn split_toggle_zoom(&self, surface: ghostty_surface_t) {
        Self::binding_action(surface, "toggle_split_zoom");
    }

    /// `toggleFullscreen(surface:)`.
    pub fn toggle_fullscreen(&self, surface: ghostty_surface_t) {
        Self::binding_action(surface, "toggle_fullscreen");
    }

    /// `changeFontSize(surface:_:)`.
    pub fn change_font_size(&self, surface: ghostty_surface_t, change: FontSizeModification) {
        let action = match change {
            FontSizeModification::Increase(amount) => format!("increase_font_size:{amount}"),
            FontSizeModification::Decrease(amount) => format!("decrease_font_size:{amount}"),
            FontSizeModification::Reset => "reset_font_size".to_owned(),
        };
        Self::binding_action(surface, &action);
    }

    /// `toggleTerminalInspector(surface:)`.
    pub fn toggle_terminal_inspector(&self, surface: ghostty_surface_t) {
        Self::binding_action(surface, "inspector:toggle");
    }

    /// `resetTerminal(surface:)`.
    pub fn reset_terminal(&self, surface: ghostty_surface_t) {
        Self::binding_action(surface, "reset");
    }

    // MARK: User Notifications

    /// Determine if a given notification should be presented to the user when
    /// Ghostty is running in the foreground.
    #[must_use]
    pub fn should_present_notification(&self, notification: &UNNotification) -> bool {
        // SAFETY: plain property getters.
        let user_info = notification.request().content().userInfo();

        // We always require the notification to be attached to a surface.
        let Some(surface) =
            user_info_surface(&user_info).and_then(|uuid| self.delegate()?.find_surface(&uuid))
        else {
            return false;
        };
        let Some(window) = surface.window() else {
            return false;
        };

        // If we don't require focus then we're good!
        let require_focus = user_info
            .objectForKey(&NSString::from_str("requireFocus"))
            .and_then(|v| v.downcast::<objc2_foundation::NSNumber>().ok())
            .is_none_or(|n| n.boolValue());
        if !require_focus {
            return true;
        }

        !window.isKeyWindow() || !surface.focused()
    }

    /// Handle a received user notification. This is called when a user
    /// notification is clicked or dismissed by the user.
    pub fn handle_user_notification(&self, response: &UNNotificationResponse) {
        let notification = response.notification();
        let user_info = notification.request().content().userInfo();
        let action_identifier = response.actionIdentifier().to_string();
        let Some(surface) =
            user_info_surface(&user_info).and_then(|uuid| self.delegate()?.find_surface(&uuid))
        else {
            return;
        };

        // SAFETY: statics provided by UserNotifications.
        let (default_id, dismiss_id) = unsafe {
            (
                objc2_user_notifications::UNNotificationDefaultActionIdentifier.to_string(),
                objc2_user_notifications::UNNotificationDismissActionIdentifier.to_string(),
            )
        };
        if action_identifier == default_id || action_identifier == USER_NOTIFICATION_ACTION_SHOW {
            // The user clicked on a notification
            surface.handle_user_notification(&notification, true);
        } else if action_identifier == dismiss_id {
            // The user dismissed the notification
            surface.handle_user_notification(&notification, false);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // `app = nil` -> the didSet frees the old value.
        let app = self.handle.replace(std::ptr::null_mut());
        if !app.is_null() {
            // SAFETY: we own the handle.
            unsafe { ghostty_app_free(app) };
        }
        REGISTRY.with(|r| {
            r.borrow_mut().remove(&(std::ptr::from_ref(self) as usize));
        });
        // The observers are removed by their own Drop.
    }
}

fn user_info_surface(user_info: &objc2_foundation::NSDictionary) -> Option<Retained<NSUUID>> {
    let s = user_info
        .objectForKey(&NSString::from_str("surface"))?
        .downcast::<NSString>()
        .ok()?;
    NSUUID::initWithUUIDString(NSUUID::alloc(), &s)
}

// MARK: Pointer helpers

/// `Unmanaged<SurfaceView>.fromOpaque(userdata!).takeUnretainedValue()`
/// (retained for the duration of the use).
fn surface_userdata(userdata: *mut c_void) -> Option<Retained<SurfaceView>> {
    // SAFETY: the userdata of every surface (and of the surface callbacks) is
    // the address of a live `SurfaceView`, set when the surface was created.
    unsafe { Retained::retain(userdata.cast::<SurfaceView>()) }
}

/// `surfaceView(from: surface)`.
fn surface_view(surface: ghostty_surface_t) -> Option<Retained<SurfaceView>> {
    // SAFETY: valid surface handle (from libghostty's target).
    let ud = unsafe { ghostty_surface_userdata(surface) };
    if ud.is_null() {
        None
    } else {
        surface_userdata(ud)
    }
}

/// `appState(fromView:)`.
fn app_state(view: &SurfaceView) -> Option<Rc<App>> {
    let surface = view.surface()?;
    // SAFETY: valid surface handle.
    let app = unsafe { ghostty_surface_app(surface) };
    if app.is_null() {
        return None;
    }
    app_from_handle(app)
}

/// `Unmanaged<App>.fromOpaque(ghostty_app_userdata(app)).takeUnretainedValue()`.
fn app_from_handle(app: ghostty_app_t) -> Option<Rc<App>> {
    // SAFETY: valid app handle.
    let ud = unsafe { ghostty_app_userdata(app) };
    if ud.is_null() {
        None
    } else {
        registered(ud as usize)
    }
}

/// The sender of notifications posted for `view` (`object: surfaceView`).
fn sender(view: &Retained<SurfaceView>) -> Sender {
    Sender::new(Retained::as_ptr(view) as usize, view.clone())
}

fn post(name: Name, view: Option<&Retained<SurfaceView>>, user_info: UserInfo) {
    NotificationCenter::default_center().post(name, view.map(sender), user_info);
}

fn app_delegate() -> Option<Retained<AppDelegate>> {
    AppDelegate::shared(crate::helpers::main_thread())
}

fn base_controller(view: &SurfaceView) -> Option<Retained<BaseTerminalController>> {
    view.window()?
        .windowController()?
        .downcast::<BaseTerminalController>()
        .ok()
}

fn tab_count(view: &SurfaceView) -> usize {
    view.window()
        .and_then(|w| w.tabGroup())
        .map_or(0, |g| g.windows().count())
}

/// Resolve an action target to a surface view for handlers that "do nothing
/// with an app target". Logs like the Swift `Ghostty.logger.warning`.
fn surface_target(what: &str, target: &ghostty_target_s) -> Option<Retained<SurfaceView>> {
    match target.tag {
        GHOSTTY_TARGET_APP => {
            warn!("{what} does nothing with an app target");
            None
        }
        GHOSTTY_TARGET_SURFACE => {
            // SAFETY: the tag selects the `surface` member.
            let surface = unsafe { target.target.surface };
            if surface.is_null() {
                None
            } else {
                surface_view(surface)
            }
        }
        _ => {
            error!("unknown action target={}", target.tag);
            None
        }
    }
}

fn c_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: libghostty passes NUL terminated strings.
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

// MARK: Callbacks

unsafe extern "C" fn wakeup_cb(userdata: *mut c_void) {
    // Wakeup can be called from any thread so we schedule the app tick from
    // the main thread. There is probably some improvements we can make to
    // coalesce multiple ticks but I don't think it matters from a performance
    // standpoint since we don't do this much.
    let addr = userdata as usize;
    DispatchQueue::main().exec_async(move || {
        if let Some(app) = registered(addr) {
            app.app_tick();
        }
    });
}

unsafe extern "C" fn close_surface_cb(userdata: *mut c_void, process_alive: bool) {
    let Some(surface) = surface_userdata(userdata) else {
        return;
    };
    post(
        GHOSTTY_CLOSE_SURFACE,
        Some(&surface),
        UserInfo::new().with("process_alive", process_alive),
    );
}

unsafe extern "C" fn read_clipboard_cb(
    userdata: *mut c_void,
    location: c_int,
    state: *mut c_void,
) -> bool {
    let Some(view) = surface_userdata(userdata) else {
        return false;
    };
    let Some(surface) = view.surface() else {
        return false;
    };

    // Get our pasteboard
    let Some(pasteboard) = ns_pasteboard::ghostty(location) else {
        return false;
    };

    // Return false if there is no text-like clipboard content so performable
    // paste bindings can pass through to the terminal.
    let Some(str) = pasteboard.get_opinionated_string_contents() else {
        return false;
    };

    complete_clipboard_request(surface, &str, state, false);
    true
}

unsafe extern "C" fn confirm_read_clipboard_cb(
    userdata: *mut c_void,
    string: *const c_char,
    state: *mut c_void,
    request: c_int,
) {
    let Some(surface) = surface_userdata(userdata) else {
        return;
    };
    let Some(value) = c_str(string) else { return };
    let Some(request) = ClipboardRequest::from_request(request) else {
        return;
    };
    post(
        CONFIRM_CLIPBOARD,
        Some(&surface),
        UserInfo::new()
            .with(CONFIRM_CLIPBOARD_STR_KEY, value)
            // The opaque request state pointer, as an address.
            .with(CONFIRM_CLIPBOARD_STATE_KEY, state as usize)
            .with(CONFIRM_CLIPBOARD_REQUEST_KEY, request),
    );
}

/// `static func completeClipboardRequest(_:data:state:confirmed:)`.
pub fn complete_clipboard_request(
    surface: ghostty_surface_t,
    data: &str,
    state: *mut c_void,
    confirmed: bool,
) {
    let end = data.find('\0').unwrap_or(data.len());
    let c = CString::new(&data.as_bytes()[..end]).unwrap_or_default();
    // SAFETY: valid surface; the string lives for the call; `state` is the
    // request state libghostty gave us.
    unsafe { ghostty_surface_complete_clipboard_request(surface, c.as_ptr(), state, confirmed) };
}

unsafe extern "C" fn write_clipboard_cb(
    userdata: *mut c_void,
    location: c_int,
    content: *const ghostty_clipboard_content_s,
    len: usize,
    confirm: bool,
) {
    let Some(surface) = surface_userdata(userdata) else {
        return;
    };
    let Some(pasteboard) = ns_pasteboard::ghostty(location) else {
        return;
    };
    if content.is_null() || len == 0 {
        return;
    }

    // Convert the C array to a Rust vector.
    // SAFETY: libghostty passes `len` valid entries.
    let slice = unsafe { std::slice::from_raw_parts(content, len) };
    // SAFETY: entries hold NUL terminated strings or null.
    let contents: Vec<ClipboardContent> = slice
        .iter()
        .filter_map(|c| unsafe { ClipboardContent::from_c(c) })
        .collect();
    if contents.is_empty() {
        return;
    }

    // Assert there is only one text/plain entry. For security reasons we need
    // to guarantee this for now since our confirmation dialog only shows one.
    debug_assert!(
        contents.iter().filter(|c| c.mime == "text/plain").count() <= 1,
        "clipboard contents should have at most one text/plain entry"
    );

    if !confirm {
        // Declare all types
        let types: Vec<Retained<NSString>> = contents
            .iter()
            .map(|c| pasteboard_type_from_mime(&c.mime))
            .collect();
        let types_array = NSArray::from_retained_slice(&types);
        // SAFETY: plain pasteboard API calls with valid arguments.
        unsafe { pasteboard.declareTypes_owner(&types_array, None) };

        // Set data for each type
        for (item, ty) in contents.iter().zip(&types) {
            let _ = pasteboard.setString_forType(&NSString::from_str(&item.data), ty);
        }
        return;
    }

    // For confirmation, use the text/plain content if it exists
    let Some(text_plain) = contents.iter().find(|c| c.mime == "text/plain") else {
        return;
    };

    post(
        CONFIRM_CLIPBOARD,
        Some(&surface),
        UserInfo::new()
            .with(CONFIRM_CLIPBOARD_STR_KEY, text_plain.data.clone())
            .with(
                CONFIRM_CLIPBOARD_REQUEST_KEY,
                ClipboardRequest::Osc52Write(Some(Retained::as_ptr(&pasteboard) as usize)),
            ),
    );
}

// MARK: Actions

unsafe extern "C" fn action_cb(
    app: ghostty_app_t,
    target: ghostty_target_s,
    action: ghostty_action_s,
) -> bool {
    action_dispatch(app, &target, &action)
}

fn action_dispatch(
    app: ghostty_app_t,
    target: &ghostty_target_s,
    action: &ghostty_action_s,
) -> bool {
    // Make sure it a target we understand so all our action handlers can assert
    match target.tag {
        GHOSTTY_TARGET_APP | GHOSTTY_TARGET_SURFACE => {}
        other => {
            warn!("unknown action target={other}");
            return false;
        }
    }

    // SAFETY (all union reads below): `action.tag` selects the active member.
    let v = &action.action;
    // Action dispatch
    unsafe {
        match action.tag {
            GHOSTTY_ACTION_QUIT => quit(),
            GHOSTTY_ACTION_NEW_WINDOW => new_window(target),
            GHOSTTY_ACTION_NEW_TAB => new_tab(target),
            GHOSTTY_ACTION_NEW_SPLIT => new_split(target, v.new_split),
            GHOSTTY_ACTION_CLOSE_TAB => close_tab(target, v.close_tab_mode),
            GHOSTTY_ACTION_CLOSE_WINDOW => close_window(target),
            GHOSTTY_ACTION_TOGGLE_FULLSCREEN => toggle_fullscreen(target, v.toggle_fullscreen),
            GHOSTTY_ACTION_MOVE_TAB => return move_tab(target, &v.move_tab),
            GHOSTTY_ACTION_GOTO_TAB => return goto_tab(target, v.goto_tab),
            GHOSTTY_ACTION_GOTO_SPLIT => return goto_split(target, v.goto_split),
            GHOSTTY_ACTION_GOTO_WINDOW => return goto_window(v.goto_window),
            GHOSTTY_ACTION_RESIZE_SPLIT => return resize_split(target, &v.resize_split),
            GHOSTTY_ACTION_EQUALIZE_SPLITS => equalize_splits(target),
            GHOSTTY_ACTION_TOGGLE_SPLIT_ZOOM => return toggle_split_zoom(target),
            GHOSTTY_ACTION_INSPECTOR => control_inspector(target, v.inspector),
            GHOSTTY_ACTION_RENDER_INSPECTOR => render_inspector(target),
            GHOSTTY_ACTION_DESKTOP_NOTIFICATION => {
                show_desktop_notification_action(target, &v.desktop_notification);
            }
            GHOSTTY_ACTION_SET_TITLE => set_title(target, &v.set_title),
            GHOSTTY_ACTION_SET_TAB_TITLE => return set_tab_title(target, &v.set_tab_title),
            GHOSTTY_ACTION_PROMPT_TITLE => return prompt_title(target, v.prompt_title),
            GHOSTTY_ACTION_PWD => pwd_changed(target, &v.pwd),
            GHOSTTY_ACTION_OPEN_CONFIG => App::open_config(),
            GHOSTTY_ACTION_FLOAT_WINDOW => toggle_float_window(target, v.float_window),
            GHOSTTY_ACTION_SECURE_INPUT => toggle_secure_input(target, v.secure_input),
            GHOSTTY_ACTION_MOUSE_SHAPE => set_mouse_shape(target, v.mouse_shape),
            GHOSTTY_ACTION_MOUSE_VISIBILITY => set_mouse_visibility(target, v.mouse_visibility),
            GHOSTTY_ACTION_MOUSE_OVER_LINK => set_mouse_over_link(target, &v.mouse_over_link),
            GHOSTTY_ACTION_INITIAL_SIZE => set_initial_size(target, &v.initial_size),
            GHOSTTY_ACTION_RESET_WINDOW_SIZE => reset_window_size(target),
            GHOSTTY_ACTION_CELL_SIZE => set_cell_size(target, &v.cell_size),
            GHOSTTY_ACTION_RENDERER_HEALTH => renderer_health(target, v.renderer_health),
            GHOSTTY_ACTION_TOGGLE_COMMAND_PALETTE => toggle_command_palette(target),
            GHOSTTY_ACTION_TOGGLE_MAXIMIZE => toggle_maximize(target),
            GHOSTTY_ACTION_TOGGLE_QUICK_TERMINAL => toggle_quick_terminal(),
            GHOSTTY_ACTION_TOGGLE_VISIBILITY => toggle_visibility(),
            GHOSTTY_ACTION_TOGGLE_BACKGROUND_OPACITY => toggle_background_opacity(target),
            GHOSTTY_ACTION_KEY_SEQUENCE => key_sequence(target, &v.key_sequence),
            GHOSTTY_ACTION_KEY_TABLE => key_table(target, &v.key_table),
            GHOSTTY_ACTION_PROGRESS_REPORT => progress_report(target, &v.progress_report),
            GHOSTTY_ACTION_CONFIG_CHANGE => config_change(app, target, &v.config_change),
            GHOSTTY_ACTION_RELOAD_CONFIG => config_reload(app, target, &v.reload_config),
            GHOSTTY_ACTION_COLOR_CHANGE => color_change(target, &v.color_change),
            GHOSTTY_ACTION_RING_BELL => ring_bell(target),
            GHOSTTY_ACTION_READONLY => set_readonly(target, v.readonly),
            GHOSTTY_ACTION_CHECK_FOR_UPDATES => check_for_updates(),
            GHOSTTY_ACTION_OPEN_URL => return open_url(&OpenUrl::from_c(&v.open_url)),
            GHOSTTY_ACTION_UNDO => return undo(target),
            GHOSTTY_ACTION_REDO => return redo(target),
            GHOSTTY_ACTION_SCROLLBAR => scrollbar(target, &v.scrollbar),
            GHOSTTY_ACTION_CLOSE_ALL_WINDOWS => close_all_windows(),
            GHOSTTY_ACTION_START_SEARCH => start_search(target, &v.start_search),
            GHOSTTY_ACTION_END_SEARCH => end_search(target),
            GHOSTTY_ACTION_SEARCH_TOTAL => search_total(target, v.search_total.total),
            GHOSTTY_ACTION_SEARCH_SELECTED => search_selected(target, v.search_selected.selected),
            GHOSTTY_ACTION_COMMAND_FINISHED => command_finished(target, &v.command_finished),
            GHOSTTY_ACTION_PRESENT_TERMINAL => return present_terminal(target),

            GHOSTTY_ACTION_TOGGLE_TAB_OVERVIEW
            | GHOSTTY_ACTION_TOGGLE_WINDOW_DECORATIONS
            | GHOSTTY_ACTION_SIZE_LIMIT
            | GHOSTTY_ACTION_QUIT_TIMER
            | GHOSTTY_ACTION_SHOW_CHILD_EXITED => {
                info!("known but unimplemented action action={}", action.tag);
                return false;
            }
            GHOSTTY_ACTION_COPY_TITLE_TO_CLIPBOARD => return copy_title_to_clipboard(target),
            other => {
                warn!("unknown action action={other}");
                return false;
            }
        }
    }

    // If we reached here then we assume performed since all unknown actions
    // are captured in the switch and return false.
    true
}

fn quit() {
    // We want to quit, start that process.
    NSApplication::sharedApplication(crate::helpers::main_thread()).terminate(None);
}

fn check_for_updates() {
    if let Some(delegate) = app_delegate() {
        delegate.check_for_updates();
    }
}

/// `openURL(_:)`.
pub fn open_url(action: &OpenUrl) -> bool {
    // If the URL doesn't have a valid scheme we assume its a file path. The URL
    // initializer will gladly take invalid URLs (e.g. plain file paths) and turn
    // them into schema-less URLs, but these won't open properly in text editors.
    // See: https://github.com/ghostty-org/ghostty/issues/8763
    let url = match NSURL::URLWithString(&NSString::from_str(&action.url)) {
        Some(candidate) if candidate.scheme().is_some() => candidate,
        _ => {
            // Expand ~ to the user's home directory so that file paths like
            // ~/Documents/file.txt resolve correctly.
            let expanded = NSString::from_str(&action.url).stringByStandardizingPath();
            NSURL::fileURLWithPath(&expanded)
        }
    };

    match action.kind {
        OpenUrlKind::Text => {
            // Open with the default editor for `*.ghostty` file or just system text editor
            let ext = url
                .pathExtension()
                .map(|e| e.to_string())
                .unwrap_or_default();
            let workspace = NSWorkspace::sharedWorkspace();
            let editor = workspace
                .default_application_url_for_extension(&ext)
                .or_else(|| workspace.default_text_editor());
            if let Some(text_editor) = editor {
                let urls = NSArray::from_slice(&[&*url]);
                workspace.openURLs_withApplicationAtURL_configuration_completionHandler(
                    &urls,
                    &text_editor,
                    &NSWorkspaceOpenConfiguration::configuration(),
                    None,
                );
                return true;
            }
        }
        // The extension will be HTML and we do the right thing automatically.
        OpenUrlKind::Html | OpenUrlKind::Unknown => {}
    }

    // Open with the default application for the URL
    NSWorkspace::sharedWorkspace().openURL(&url);
    true
}

fn undo_manager_for(
    target: &ghostty_target_s,
) -> Option<Retained<objc2_foundation::NSUndoManager>> {
    match target.tag {
        GHOSTTY_TARGET_APP => app_delegate().map(|d| Retained::into_super(d.undo_manager())),
        GHOSTTY_TARGET_SURFACE => {
            // SAFETY: the tag selects the `surface` member.
            let surface = unsafe { target.target.surface };
            if surface.is_null() {
                return None;
            }
            surface_view(surface)?.undoManager()
        }
        _ => None,
    }
}

fn undo(target: &ghostty_target_s) -> bool {
    let Some(um) = undo_manager_for(target) else {
        return false;
    };
    if !um.canUndo() {
        return false;
    }
    um.undo();
    true
}

fn redo(target: &ghostty_target_s) -> bool {
    let Some(um) = undo_manager_for(target) else {
        return false;
    };
    if !um.canRedo() {
        return false;
    }
    um.redo();
    true
}

/// Shared by new_window/new_tab/new_split.
fn inherited_config(surface: ghostty_surface_t, context: c_int) -> SurfaceConfiguration {
    // SAFETY: valid surface; the returned strings live as long as the surface.
    unsafe { SurfaceConfiguration::from_c(&ghostty_surface_inherited_config(surface, context)) }
}

fn surface_of(target: &ghostty_target_s) -> Option<ghostty_surface_t> {
    // SAFETY: only called for targets tagged as surface.
    let s = unsafe { target.target.surface };
    if s.is_null() { None } else { Some(s) }
}

fn new_window(target: &ghostty_target_s) {
    match target.tag {
        GHOSTTY_TARGET_APP => post(GHOSTTY_NEW_WINDOW, None, UserInfo::new()),
        GHOSTTY_TARGET_SURFACE => {
            let Some(surface) = surface_of(target) else {
                return;
            };
            let Some(view) = surface_view(surface) else {
                return;
            };
            post(
                GHOSTTY_NEW_WINDOW,
                Some(&view),
                UserInfo::new().with(
                    NEW_SURFACE_CONFIG_KEY,
                    inherited_config(surface, GHOSTTY_SURFACE_CONTEXT_WINDOW),
                ),
            );
        }
        _ => debug_assert!(false),
    }
}

fn new_tab(target: &ghostty_target_s) {
    match target.tag {
        GHOSTTY_TARGET_APP => post(GHOSTTY_NEW_TAB, None, UserInfo::new()),
        GHOSTTY_TARGET_SURFACE => {
            let Some(surface) = surface_of(target) else {
                return;
            };
            let Some(view) = surface_view(surface) else {
                return;
            };
            let Some(state) = app_state(&view) else {
                return;
            };
            if !state.config().window_decorations() {
                let alert = NSAlert::new(crate::helpers::main_thread());
                alert.setMessageText(&NSString::from_str("Tabs are disabled"));
                alert.setInformativeText(&NSString::from_str(
                    "Enable window decorations to use tabs",
                ));
                alert.addButtonWithTitle(&NSString::from_str("OK"));
                alert.setAlertStyle(NSAlertStyle::Warning);
                let _ = alert.runModal();
                return;
            }

            post(
                GHOSTTY_NEW_TAB,
                Some(&view),
                UserInfo::new().with(
                    NEW_SURFACE_CONFIG_KEY,
                    inherited_config(surface, GHOSTTY_SURFACE_CONTEXT_TAB),
                ),
            );
        }
        _ => debug_assert!(false),
    }
}

fn new_split(target: &ghostty_target_s, direction: c_int) {
    match target.tag {
        GHOSTTY_TARGET_APP => {
            // New split does nothing with an app target
            warn!("new split does nothing with an app target");
        }
        GHOSTTY_TARGET_SURFACE => {
            let Some(surface) = surface_of(target) else {
                return;
            };
            let Some(view) = surface_view(surface) else {
                return;
            };
            post(
                GHOSTTY_NEW_SPLIT,
                Some(&view),
                UserInfo::new().with("direction", direction).with(
                    NEW_SURFACE_CONFIG_KEY,
                    inherited_config(surface, GHOSTTY_SURFACE_CONTEXT_SPLIT),
                ),
            );
        }
        _ => debug_assert!(false),
    }
}

fn present_terminal(target: &ghostty_target_s) -> bool {
    match target.tag {
        GHOSTTY_TARGET_APP => false,
        GHOSTTY_TARGET_SURFACE => {
            let Some(surface) = surface_of(target) else {
                return false;
            };
            let Some(view) = surface_view(surface) else {
                return false;
            };
            post(GHOSTTY_PRESENT_TERMINAL, Some(&view), UserInfo::new());
            true
        }
        _ => false,
    }
}

fn close_tab(target: &ghostty_target_s, mode: c_int) {
    let Some(view) = surface_target("close tabs", target) else {
        return;
    };
    match mode {
        GHOSTTY_ACTION_CLOSE_TAB_MODE_THIS => post(GHOSTTY_CLOSE_TAB, Some(&view), UserInfo::new()),
        GHOSTTY_ACTION_CLOSE_TAB_MODE_OTHER => {
            post(GHOSTTY_CLOSE_OTHER_TABS, Some(&view), UserInfo::new());
        }
        GHOSTTY_ACTION_CLOSE_TAB_MODE_RIGHT => {
            post(
                GHOSTTY_CLOSE_TABS_ON_THE_RIGHT,
                Some(&view),
                UserInfo::new(),
            );
        }
        _ => debug_assert!(false),
    }
}

fn close_window(target: &ghostty_target_s) {
    let Some(view) = surface_target("close window", target) else {
        return;
    };
    post(GHOSTTY_CLOSE_WINDOW, Some(&view), UserInfo::new());
}

fn close_all_windows() {
    let Some(delegate) = app_delegate() else {
        return;
    };
    delegate.close_all_windows();
}

fn toggle_fullscreen(target: &ghostty_target_s, raw: c_int) {
    let Some(view) = surface_target("toggle fullscreen", target) else {
        return;
    };
    let Some(mode) = fullscreen_mode::fullscreen_mode_from_ghostty(raw) else {
        warn!("unknown fullscreen mode raw={raw}");
        return;
    };
    post(
        GHOSTTY_TOGGLE_FULLSCREEN,
        Some(&view),
        UserInfo::new().with(FULLSCREEN_MODE_KEY, mode),
    );
}

fn toggle_command_palette(target: &ghostty_target_s) {
    let Some(view) = surface_target("toggle command palette", target) else {
        return;
    };
    post(
        GHOSTTY_COMMAND_PALETTE_DID_TOGGLE,
        Some(&view),
        UserInfo::new(),
    );
}

fn toggle_maximize(target: &ghostty_target_s) {
    let Some(view) = surface_target("toggle maximize", target) else {
        return;
    };
    post(GHOSTTY_MAXIMIZE_DID_TOGGLE, Some(&view), UserInfo::new());
}

fn toggle_visibility() {
    let Some(delegate) = app_delegate() else {
        return;
    };
    delegate.toggle_visibility();
}

fn ring_bell(target: &ghostty_target_s) {
    // Technically we could still request app attention here but there are no
    // known cases where the bell is rang with an app target so I think its
    // better to warn.
    let Some(view) = surface_target("ring bell", target) else {
        return;
    };
    post(GHOSTTY_BELL_DID_RING, Some(&view), UserInfo::new());
}

fn set_readonly(target: &ghostty_target_s, v: c_int) {
    let Some(view) = surface_target("set readonly", target) else {
        return;
    };
    post(
        GHOSTTY_DID_CHANGE_READONLY,
        Some(&view),
        UserInfo::new().with(READONLY_KEY, v == GHOSTTY_READONLY_ON),
    );
}

fn move_tab(
    target: &ghostty_target_s,
    mv: &crate::ghostty::capi::ghostty_action_move_tab_s,
) -> bool {
    let Some(view) = surface_target("move tab", target) else {
        return false;
    };

    // See goto_tab for notes on this check.
    if tab_count(&view) <= 1 {
        return false;
    }

    post(
        GHOSTTY_MOVE_TAB,
        Some(&view),
        UserInfo::new().with(GHOSTTY_MOVE_TAB_KEY, MoveTab::from_c(mv)),
    );
    true
}

fn goto_tab(target: &ghostty_target_s, tab: c_int) -> bool {
    let Some(view) = surface_target("goto tab", target) else {
        return false;
    };

    // Similar to goto_split (see comment there) about our performability, we
    // should make this more accurate later.
    if tab_count(&view) <= 1 {
        return false;
    }

    post(
        GHOSTTY_GOTO_TAB,
        Some(&view),
        UserInfo::new().with(GOTO_TAB_KEY, tab),
    );
    true
}

fn goto_split(target: &ghostty_target_s, direction: c_int) -> bool {
    let Some(view) = surface_target("goto split", target) else {
        return false;
    };
    let Some(controller) = base_controller(&view) else {
        return false;
    };

    // If the window has no splits, the action is not performable
    if !controller.surface_tree_is_split() {
        return false;
    }

    // Convert the C API direction to our type
    let Some(split_direction) = SplitFocusDirection::from_direction(direction) else {
        return false;
    };

    // Check if a split actually exists in the target direction before
    // returning true. This ensures performable keybinds only consume the key
    // event when we actually perform navigation.
    if !controller.focus_target_exists(split_direction, &view) {
        return false;
    }

    // We have a valid target, post the notification to perform the navigation
    post(
        GHOSTTY_FOCUS_SPLIT,
        Some(&view),
        UserInfo::new().with(SPLIT_DIRECTION_KEY, split_direction),
    );
    true
}

fn goto_window(direction: c_int) -> bool {
    let mtm = crate::helpers::main_thread();
    // Collect candidate windows: visible terminal windows that are either
    // standalone or the currently selected tab in their tab group. This treats
    // each native tab group as a single "window" for navigation purposes,
    // since goto_tab handles per-tab navigation.
    let candidates: Vec<Retained<NSWindow>> = NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .filter(|window| {
            let is_terminal = window
                .windowController()
                .is_some_and(|c| c.downcast::<BaseTerminalController>().is_ok());
            if !is_terminal || !window.isVisible() || window.isMiniaturized() {
                return false;
            }
            // For native tabs, only include the selected tab in each group
            if let Some(group) = window.tabGroup()
                && group
                    .selectedWindow()
                    .is_some_and(|s| Retained::as_ptr(&s) != Retained::as_ptr(window))
            {
                return false;
            }
            true
        })
        .collect();

    // Need at least two windows to navigate between
    if candidates.len() <= 1 {
        return false;
    }

    // Find starting index from the current key/main window
    let start_index = candidates
        .iter()
        .position(|w| w.isKeyWindow())
        .or_else(|| candidates.iter().position(|w| w.isMainWindow()))
        .unwrap_or(0);

    let step: isize = match direction {
        GHOSTTY_GOTO_WINDOW_NEXT => 1,
        GHOSTTY_GOTO_WINDOW_PREVIOUS => -1,
        _ => return false,
    };

    // Iterate with wrap-around until we find a valid window or return to start
    let count = candidates.len() as isize;
    let mut index = (start_index as isize + step + count) % count;

    while index != start_index as isize {
        let candidate = &candidates[index as usize];
        if candidate.isVisible() && !candidate.isMiniaturized() {
            candidate.makeKeyAndOrderFront(None);
            // Also focus the terminal surface within the window
            if let Some(controller) = candidate
                .windowController()
                .and_then(|c| c.downcast::<BaseTerminalController>().ok())
            {
                if let Some(surface) = controller.focused_surface() {
                    move_focus(&surface, None, None);
                }
            }
            return true;
        }
        index = (index + step + count) % count;
    }

    false
}

fn resize_split(
    target: &ghostty_target_s,
    resize: &crate::ghostty::capi::ghostty_action_resize_split_s,
) -> bool {
    let Some(view) = surface_target("resize split", target) else {
        return false;
    };
    let Some(controller) = base_controller(&view) else {
        return false;
    };

    // If the window has no splits, the action is not performable
    if !controller.surface_tree_is_split() {
        return false;
    }

    let Some(direction) = SplitResizeDirection::from_direction(resize.direction) else {
        return false;
    };
    post(
        DID_RESIZE_SPLIT,
        Some(&view),
        UserInfo::new()
            .with(RESIZE_SPLIT_DIRECTION_KEY, direction)
            .with(RESIZE_SPLIT_AMOUNT_KEY, resize.amount),
    );
    true
}

fn equalize_splits(target: &ghostty_target_s) {
    let Some(view) = surface_target("equalize splits", target) else {
        return;
    };
    post(DID_EQUALIZE_SPLITS, Some(&view), UserInfo::new());
}

fn toggle_split_zoom(target: &ghostty_target_s) -> bool {
    let Some(view) = surface_target("toggle split zoom", target) else {
        return false;
    };
    let Some(controller) = base_controller(&view) else {
        return false;
    };

    // If the window has no splits, the action is not performable
    if !controller.surface_tree_is_split() {
        return false;
    }

    post(DID_TOGGLE_SPLIT_ZOOM, Some(&view), UserInfo::new());
    true
}

fn control_inspector(target: &ghostty_target_s, mode: c_int) {
    let Some(view) = surface_target("toggle inspector", target) else {
        return;
    };
    post(
        DID_CONTROL_INSPECTOR,
        Some(&view),
        UserInfo::new().with("mode", mode),
    );
}

fn show_desktop_notification_action(
    target: &ghostty_target_s,
    n: &crate::ghostty::capi::ghostty_action_desktop_notification_s,
) {
    let Some(view) = surface_target("desktop notification", target) else {
        return;
    };
    let Some(title) = c_str(n.title) else { return };
    let Some(body) = c_str(n.body) else { return };
    show_desktop_notification(&view, &title, &body, true);
}

fn show_desktop_notification(
    view: &Retained<SurfaceView>,
    title: &str,
    body: &str,
    require_focus: bool,
) {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let request_done = RcBlock::new(
        |_granted: objc2::runtime::Bool, error: *mut objc2_foundation::NSError| {
            // SAFETY: null or a valid NSError for the call.
            if let Some(error) = unsafe { error.as_ref() } {
                error!(
                    "Error while requesting notification authorization: {}",
                    error.localizedDescription()
                );
            }
        },
    );
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &request_done,
    );

    // The settings callback runs on a background thread; hop to the main
    // queue before touching the view (Swift calls the view directly).
    let mtm = crate::helpers::main_thread();
    let bound = std::sync::Arc::new(MainThreadBound::new(
        (Weak::from_retained(view), title.to_owned(), body.to_owned()),
        mtm,
    ));
    let handler = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: valid settings object for the call.
        if unsafe { settings.as_ref().authorizationStatus() } != UNAuthorizationStatus::Authorized {
            return;
        }
        let bound = bound.clone();
        DispatchQueue::main().exec_async(move || {
            let mtm = MainThreadMarker::new().expect("main queue");
            let (weak, title, body) = bound.get(mtm);
            if let Some(view) = weak.load() {
                view.show_user_notification(title, body, require_focus);
            }
        });
    });
    center.getNotificationSettingsWithCompletionHandler(&handler);
}

fn command_finished(
    target: &ghostty_target_s,
    v: &crate::ghostty::capi::ghostty_action_command_finished_s,
) {
    use crate::ghostty::config_types::{NotifyOnCommandFinish, NotifyOnCommandFinishAction};

    let Some(view) = surface_target("command finished", target) else {
        return;
    };

    // Determine if we even care about command finish notifications
    let Some(config) = app_delegate().map(|d| d.ghostty().config()) else {
        return;
    };
    match config.notify_on_command_finish() {
        NotifyOnCommandFinish::Never => return,
        NotifyOnCommandFinish::Unfocused => {
            if view.focused() {
                return;
            }
        }
        NotifyOnCommandFinish::Always => {}
    }

    // Determine if the command was slow enough
    let duration = Duration::from_nanos(v.duration);
    if duration < config.notify_on_command_finish_after() {
        return;
    }

    let actions = config.notify_on_command_finish_action();

    if actions.contains(NotifyOnCommandFinishAction::BELL) {
        post(GHOSTTY_BELL_DID_RING, Some(&view), UserInfo::new());
    }

    if actions.contains(NotifyOnCommandFinishAction::NOTIFY) {
        let title = match v.exit_code.cmp(&0) {
            std::cmp::Ordering::Less => "Command Finished",
            std::cmp::Ordering::Equal => "Command Succeeded",
            std::cmp::Ordering::Greater => "Command Failed",
        };

        let formatted = duration_format::format_duration_abbreviated(duration);
        let body = if v.exit_code < 0 {
            format!("Command took {formatted}.")
        } else {
            format!(
                "Command took {formatted} and exited with code {}.",
                v.exit_code
            )
        };

        show_desktop_notification(&view, title, &body, false);
    }
}

fn toggle_float_window(target: &ghostty_target_s, mode_raw: c_int) {
    let Some(mode) = SetFloatWindow::from_c(mode_raw) else {
        return;
    };
    let Some(view) = surface_target("toggle float window", target) else {
        return;
    };
    let Some(window) = view.window() else { return };
    let Ok(window) = window.downcast::<TerminalWindow>() else {
        return;
    };

    // SAFETY: plain constants.
    let (floating, normal) = (NSFloatingWindowLevel, NSNormalWindowLevel);
    match mode {
        SetFloatWindow::On => window.setLevel(floating),
        SetFloatWindow::Off => window.setLevel(normal),
        SetFloatWindow::Toggle => window.setLevel(if window.level() == floating {
            normal
        } else {
            floating
        }),
    }

    if let Some(delegate) = app_delegate() {
        delegate.sync_float_on_top_menu(Some(&window));
    }
}

fn toggle_background_opacity(target: &ghostty_target_s) {
    let Some(view) = surface_target("toggle background opacity", target) else {
        return;
    };
    let Some(controller) = base_controller(&view) else {
        return;
    };
    controller.toggle_background_opacity();
}

fn toggle_secure_input(target: &ghostty_target_s, mode_raw: c_int) {
    let Some(mode) = SetSecureInput::from_c(mode_raw) else {
        return;
    };

    match target.tag {
        GHOSTTY_TARGET_APP => {
            let Some(delegate) = app_delegate() else {
                return;
            };
            delegate.set_secure_input(mode);
        }
        GHOSTTY_TARGET_SURFACE => {
            let Some(surface) = surface_of(target) else {
                return;
            };
            let Some(view) = surface_view(surface) else {
                return;
            };
            let Some(state) = app_state(&view) else {
                return;
            };
            if !state.config().auto_secure_input() {
                return;
            }

            match mode {
                SetSecureInput::On => view.set_password_input(true),
                SetSecureInput::Off => view.set_password_input(false),
                SetSecureInput::Toggle => view.set_password_input(!view.password_input()),
            }
        }
        _ => debug_assert!(false),
    }
}

fn toggle_quick_terminal() {
    let Some(delegate) = app_delegate() else {
        return;
    };
    delegate.toggle_quick_terminal();
}

fn set_title(target: &ghostty_target_s, v: &crate::ghostty::capi::ghostty_action_set_title_s) {
    let Some(view) = surface_target("set title", target) else {
        return;
    };
    let Some(title) = c_str(v.title) else { return };
    view.set_title(&title);
}

fn set_tab_title(
    target: &ghostty_target_s,
    v: &crate::ghostty::capi::ghostty_action_set_title_s,
) -> bool {
    let Some(view) = surface_target("set tab title", target) else {
        return false;
    };
    let Some(title) = c_str(v.title) else {
        return false;
    };
    let title_override = if title.is_empty() { None } else { Some(title) };
    let Some(controller) = base_controller(&view) else {
        return false;
    };
    controller.set_title_override(title_override);
    true
}

fn copy_title_to_clipboard(target: &ghostty_target_s) -> bool {
    if target.tag != GHOSTTY_TARGET_SURFACE {
        return false;
    }
    let Some(surface) = surface_of(target) else {
        return false;
    };
    let Some(view) = surface_view(surface) else {
        return false;
    };
    let title = view.title();
    if title.is_empty() {
        return false;
    }
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    // SAFETY: valid constant.
    let _ = pasteboard.setString_forType(&NSString::from_str(&title), unsafe {
        NSPasteboardTypeString
    });
    true
}

fn prompt_title(target: &ghostty_target_s, v: c_int) -> bool {
    match PromptTitle::from_c(v) {
        PromptTitle::Surface => match target.tag {
            GHOSTTY_TARGET_APP => {
                warn!("set title prompt does nothing with an app target");
                false
            }
            GHOSTTY_TARGET_SURFACE => {
                let Some(surface) = surface_of(target) else {
                    return false;
                };
                let Some(view) = surface_view(surface) else {
                    return false;
                };
                view.prompt_title();
                true
            }
            _ => false,
        },
        PromptTitle::Tab => match target.tag {
            GHOSTTY_TARGET_APP => {
                let app = NSApplication::sharedApplication(crate::helpers::main_thread());
                let Some(window) = app.mainWindow().or_else(|| app.keyWindow()) else {
                    return false;
                };
                let Some(controller) = window
                    .windowController()
                    .and_then(|c| c.downcast::<BaseTerminalController>().ok())
                else {
                    return false;
                };
                controller.prompt_tab_title();
                true
            }
            GHOSTTY_TARGET_SURFACE => {
                let Some(surface) = surface_of(target) else {
                    return false;
                };
                let Some(view) = surface_view(surface) else {
                    return false;
                };
                let Some(controller) = base_controller(&view) else {
                    return false;
                };
                controller.prompt_tab_title();
                true
            }
            _ => false,
        },
    }
}

fn pwd_changed(target: &ghostty_target_s, v: &crate::ghostty::capi::ghostty_action_pwd_s) {
    let Some(view) = surface_target("pwd change", target) else {
        return;
    };
    let Some(pwd) = c_str(v.pwd) else { return };
    view.set_pwd(Some(pwd));
}

fn set_mouse_shape(target: &ghostty_target_s, shape: c_int) {
    let Some(view) = surface_target("set mouse shape", target) else {
        return;
    };
    view.set_cursor_shape(shape);
}

fn set_mouse_visibility(target: &ghostty_target_s, v: c_int) {
    let Some(view) = surface_target("set mouse visibility", target) else {
        return;
    };
    match v {
        GHOSTTY_MOUSE_VISIBLE => view.set_cursor_visibility(true),
        GHOSTTY_MOUSE_HIDDEN => view.set_cursor_visibility(false),
        _ => {}
    }
}

fn set_mouse_over_link(
    target: &ghostty_target_s,
    v: &crate::ghostty::capi::ghostty_action_mouse_over_link_s,
) {
    let Some(view) = surface_target("mouse over link", target) else {
        return;
    };
    if v.len == 0 || v.url.is_null() {
        view.set_hover_url(None);
        return;
    }

    // SAFETY: libghostty passes `len` valid bytes.
    let buffer = unsafe { std::slice::from_raw_parts(v.url.cast::<u8>(), v.len) };
    view.set_hover_url(String::from_utf8(buffer.to_vec()).ok());
}

fn set_initial_size(
    target: &ghostty_target_s,
    v: &crate::ghostty::capi::ghostty_action_initial_size_s,
) {
    let Some(view) = surface_target("initial size", target) else {
        return;
    };
    view.set_initial_size(Some(NSSize::new(f64::from(v.width), f64::from(v.height))));
}

fn reset_window_size(target: &ghostty_target_s) {
    let Some(view) = surface_target("reset window size", target) else {
        return;
    };
    post(GHOSTTY_RESET_WINDOW_SIZE, Some(&view), UserInfo::new());
}

fn set_cell_size(target: &ghostty_target_s, v: &crate::ghostty::capi::ghostty_action_cell_size_s) {
    let Some(view) = surface_target("cell size", target) else {
        return;
    };
    let backing_size = NSSize::new(f64::from(v.width), f64::from(v.height));
    let weak = Weak::from_retained(&view);
    dispatch_main_async(move || {
        let Some(view) = weak.load() else { return };
        let size = view.convert_size_from_backing(backing_size);
        view.set_cell_size(size);
    });
}

fn render_inspector(target: &ghostty_target_s) {
    let Some(view) = surface_target("render inspector", target) else {
        return;
    };
    post(INSPECTOR_NEEDS_DISPLAY, Some(&view), UserInfo::new());
}

fn renderer_health(target: &ghostty_target_s, v: c_int) {
    let Some(view) = surface_target("renderer health", target) else {
        return;
    };
    post(
        DID_UPDATE_RENDERER_HEALTH,
        Some(&view),
        UserInfo::new().with("health", v),
    );
}

fn key_sequence(
    target: &ghostty_target_s,
    v: &crate::ghostty::capi::ghostty_action_key_sequence_s,
) {
    let Some(view) = surface_target("key sequence", target) else {
        return;
    };
    if v.active {
        post(
            DID_CONTINUE_KEY_SEQUENCE,
            Some(&view),
            UserInfo::new().with(KEY_SEQUENCE_KEY, input::keyboard_shortcut(&v.trigger)),
        );
    } else {
        post(DID_END_KEY_SEQUENCE, Some(&view), UserInfo::new());
    }
}

fn key_table(target: &ghostty_target_s, v: &crate::ghostty::capi::ghostty_action_key_table_s) {
    let Some(view) = surface_target("key table", target) else {
        return;
    };
    // SAFETY: the C struct comes straight from libghostty.
    let Some(action) = (unsafe { KeyTable::from_c(v) }) else {
        return;
    };
    post(
        DID_CHANGE_KEY_TABLE,
        Some(&view),
        UserInfo::new().with(KEY_TABLE_KEY, action),
    );
}

fn progress_report(
    target: &ghostty_target_s,
    v: &crate::ghostty::capi::ghostty_action_progress_report_s,
) {
    let Some(view) = surface_target("progress report", target) else {
        return;
    };
    let Some(config) = app_delegate().map(|d| d.ghostty().config()) else {
        return;
    };

    if !config.progress_style() {
        debug!("progress_report action blocked by config");
        let weak = Weak::from_retained(&view);
        dispatch_main_async(move || {
            if let Some(view) = weak.load() {
                view.set_progress_report(None);
            }
        });
        return;
    }

    let report = ProgressReport::from_c(v);
    let weak = Weak::from_retained(&view);
    dispatch_main_async(move || {
        let Some(view) = weak.load() else { return };
        if report.state == ProgressReportState::Remove {
            view.set_progress_report(None);
        } else {
            view.set_progress_report(Some(report));
        }
    });
}

fn scrollbar(target: &ghostty_target_s, v: &crate::ghostty::capi::ghostty_action_scrollbar_s) {
    let Some(view) = surface_target("scrollbar", target) else {
        return;
    };
    post(
        GHOSTTY_DID_UPDATE_SCROLLBAR,
        Some(&view),
        UserInfo::new().with(SCROLLBAR_KEY, Scrollbar::from_c(v)),
    );
}

fn start_search(
    target: &ghostty_target_s,
    v: &crate::ghostty::capi::ghostty_action_start_search_s,
) {
    let Some(view) = surface_target("start_search", target) else {
        return;
    };
    // SAFETY: the needle is null or a valid C string.
    let start = unsafe { StartSearch::from_c(v) };
    let weak = Weak::from_retained(&view);
    dispatch_main_async(move || {
        let Some(view) = weak.load() else { return };
        if let Some(search_state) = view.search_state() {
            if let Some(needle) = start.needle.as_ref().filter(|n| !n.is_empty()) {
                search_state.needle.set(needle.clone());
            }
        } else {
            view.set_search_state(Some(Rc::new(SearchState::new(&start))));
        }

        post(GHOSTTY_SEARCH_FOCUS, Some(&view), UserInfo::new());
    });
}

fn end_search(target: &ghostty_target_s) {
    let Some(view) = surface_target("end_search", target) else {
        return;
    };
    let weak = Weak::from_retained(&view);
    dispatch_main_async(move || {
        if let Some(view) = weak.load() {
            view.set_search_state(None);
        }
    });
}

fn search_total(target: &ghostty_target_s, total: isize) {
    let Some(view) = surface_target("search_total", target) else {
        return;
    };
    let total = usize::try_from(total).ok();
    let weak = Weak::from_retained(&view);
    dispatch_main_async(move || {
        if let Some(state) = weak.load().and_then(|v| v.search_state()) {
            state.total.set(total);
        }
    });
}

fn search_selected(target: &ghostty_target_s, selected: isize) {
    let Some(view) = surface_target("search_selected", target) else {
        return;
    };
    let selected = usize::try_from(selected).ok();
    let weak = Weak::from_retained(&view);
    dispatch_main_async(move || {
        if let Some(state) = weak.load().and_then(|v| v.search_state()) {
            state.selected.set(selected);
        }
    });
}

fn config_reload(
    app: ghostty_app_t,
    target: &ghostty_target_s,
    v: &crate::ghostty::capi::ghostty_action_reload_config_s,
) {
    info!("config reload notification");

    let Some(ghostty) = app_from_handle(app) else {
        return;
    };

    match target.tag {
        GHOSTTY_TARGET_APP => ghostty.reload_config(v.soft),
        GHOSTTY_TARGET_SURFACE => {
            let Some(surface) = surface_of(target) else {
                return;
            };
            ghostty.reload_config_surface(surface, v.soft);
        }
        _ => debug_assert!(false),
    }
}

fn config_change(
    app: ghostty_app_t,
    target: &ghostty_target_s,
    v: &crate::ghostty::capi::ghostty_action_config_change_s,
) {
    info!("config change notification");

    // Clone the config so we own the memory. It'd be nicer to not have to do
    // this but since we async send the config out below we have to own the
    // lifetime. A future improvement might be to add reference counting to
    // config or something so apprt's do not have to do this.
    // SAFETY: `v.config` is valid for the duration of the callback.
    let config = Rc::new(unsafe { Config::clone_from_ptr(v.config) });

    match target.tag {
        GHOSTTY_TARGET_APP => {
            // Notify the world that the app config changed
            post(
                GHOSTTY_CONFIG_DID_CHANGE,
                None,
                UserInfo::new().with(GHOSTTY_CONFIG_CHANGE_KEY, config.clone()),
            );

            // We also REPLACE our app-level config when this happens. This lets
            // all the various things that depend on this but are still theme
            // specific such as split border color work.
            let Some(ghostty) = app_from_handle(app) else {
                return;
            };
            ghostty.config.set(config);
        }
        GHOSTTY_TARGET_SURFACE => {
            let Some(surface) = surface_of(target) else {
                return;
            };
            let Some(view) = surface_view(surface) else {
                return;
            };
            post(
                GHOSTTY_CONFIG_DID_CHANGE,
                Some(&view),
                UserInfo::new().with(GHOSTTY_CONFIG_CHANGE_KEY, config),
            );
        }
        _ => debug_assert!(false),
    }
}

fn color_change(
    target: &ghostty_target_s,
    change: &crate::ghostty::capi::ghostty_action_color_change_s,
) {
    let Some(view) = surface_target("color change", target) else {
        return;
    };
    post(
        GHOSTTY_COLOR_DID_CHANGE,
        Some(&view),
        UserInfo::new().with(GHOSTTY_COLOR_CHANGE_KEY, ColorChange::from_c(change)),
    );
}
