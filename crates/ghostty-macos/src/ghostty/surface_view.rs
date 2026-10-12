//! `Ghostty.SurfaceView` (`Surface View/SurfaceView_AppKit.swift`,
//! `SurfaceView.swift`): the NSView of a terminal surface.
//!
//! TODO(MA2): this is the *model-only first cut* written by MA1 so that
//! `Ghostty.App`'s action glue compiles and behaves like the Swift app for
//! everything that only reads/writes the view's published state. It contains
//! the instance state those callbacks touch (title, pwd, cell size, hover URL,
//! progress report, search state, scrollbar, cursor style, notification ids,
//! ...) and the methods `Ghostty.App` calls. All NSView behaviour -- the
//! `init(_:baseConfig:uuid:)` that creates the `ghostty_surface_t`, layer/
//! Metal hosting, first responder & key/mouse/IME handling, tracking areas,
//! drag and drop, scrolling, the SwiftUI wrappers -- is MA2's. Every unported
//! piece is marked `TODO(MA2)` with its Swift origin.
#![allow(unsafe_code)]
// TODO(MA2) methods that do not use `self` yet.
#![allow(clippy::unused_self)]

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    ffi::c_int,
    ptr::NonNull,
    rc::Rc,
    sync::Arc,
};

use block2::RcBlock;
use dispatch2::{DispatchQueue, MainThreadBound};
use log::error;
use objc2::{
    DefinedClass,
    MainThreadMarker,
    MainThreadOnly,
    Message,
    define_class,
    msg_send,
    rc::{Retained, Weak},
    runtime::AnyObject,
};
use objc2_app_kit::{
    NSAlert,
    NSAlertFirstButtonReturn,
    NSAlertStyle,
    NSCursor,
    NSModalResponse,
    NSResponder,
    NSTextField,
    NSView,
};
use objc2_foundation::{
    NSArray,
    NSDictionary,
    NSError,
    NSNumber,
    NSObject,
    NSObjectProtocol,
    NSPoint,
    NSRect,
    NSSize,
    NSString,
    NSTimer,
    NSUUID,
};
use objc2_user_notifications::{
    UNMutableNotificationContent,
    UNNotification,
    UNNotificationRequest,
    UNNotificationSound,
    UNUserNotificationCenter,
};

use crate::{
    ghostty::{
        action::{ProgressReport, ProgressReportState, Scrollbar, StartSearch},
        capi::{
            consts::{
                GHOSTTY_MOUSE_SHAPE_CONTEXT_MENU,
                GHOSTTY_MOUSE_SHAPE_CROSSHAIR,
                GHOSTTY_MOUSE_SHAPE_DEFAULT,
                GHOSTTY_MOUSE_SHAPE_E_RESIZE,
                GHOSTTY_MOUSE_SHAPE_EW_RESIZE,
                GHOSTTY_MOUSE_SHAPE_GRAB,
                GHOSTTY_MOUSE_SHAPE_GRABBING,
                GHOSTTY_MOUSE_SHAPE_N_RESIZE,
                GHOSTTY_MOUSE_SHAPE_NOT_ALLOWED,
                GHOSTTY_MOUSE_SHAPE_NS_RESIZE,
                GHOSTTY_MOUSE_SHAPE_POINTER,
                GHOSTTY_MOUSE_SHAPE_S_RESIZE,
                GHOSTTY_MOUSE_SHAPE_TEXT,
                GHOSTTY_MOUSE_SHAPE_VERTICAL_TEXT,
                GHOSTTY_MOUSE_SHAPE_W_RESIZE,
            },
            ghostty_surface_needs_confirm_quit,
            ghostty_surface_process_exited,
            ghostty_surface_t,
        },
        package::USER_NOTIFICATION_CATEGORY,
        surface::Surface,
    },
    helpers::{cursor::CursorStyle, keyboard_shortcut::KeyboardShortcut, published::Published},
};

/// `Ghostty.SurfaceView.SearchState`.
pub struct SearchState {
    pub needle: Published<String>,
    pub selected: Published<Option<usize>>,
    pub total: Published<Option<usize>>,
}

impl SearchState {
    /// `init(from startSearch:)`.
    #[must_use]
    pub fn new(start_search: &StartSearch) -> Self {
        Self {
            needle: Published::new(start_search.needle.clone().unwrap_or_default()),
            selected: Published::new(None),
            total: Published::new(None),
        }
    }
}

/// The instance state of a [`SurfaceView`].
pub struct SurfaceViewIvars {
    /// `id`: unique ID per surface.
    pub id: Retained<NSUUID>,
    /// `surfaceModel` (its C value is `surface`).
    surface_model: RefCell<Option<Surface>>,

    /// `title`: the title as defined by the pty (or set manually).
    pub title: Published<String>,
    /// `pwd`.
    pub pwd: Published<Option<String>>,
    /// `cellSize`.
    pub cell_size: Published<NSSize>,
    /// `healthy` (renderer health).
    pub healthy: Published<bool>,
    /// `hoverUrl`.
    pub hover_url: Published<Option<String>>,
    /// `progressReport`; setting it (re)starts the 15s expiry timer.
    pub progress_report: Published<Option<ProgressReport>>,
    /// `keySequence`.
    pub key_sequence: Published<Vec<KeyboardShortcut>>,
    /// `keyTables`.
    pub key_tables: Published<Vec<String>>,
    /// `searchState`.
    pub search_state: Published<Option<Rc<SearchState>>>,
    /// `pointerStyle`.
    pub pointer_style: Published<CursorStyle>,
    /// `cursorVisible`.
    pub cursor_visible: Published<bool>,
    /// `bell`.
    pub bell: Published<bool>,
    /// `readonly`.
    pub readonly: Published<bool>,
    /// `initialSize`.
    pub initial_size: Cell<Option<NSSize>>,
    /// `passwordInput`.
    password_input: Cell<bool>,
    /// `scrollbar`.
    pub scrollbar: RefCell<Option<Scrollbar>>,
    /// `notificationIdentifiers`.
    pub notification_identifiers: RefCell<HashSet<String>>,
    /// `focused` (default true, like Swift).
    focused: Cell<bool>,

    title_change_timer: RefCell<Option<Retained<NSTimer>>>,
    title_fallback_timer: RefCell<Option<Retained<NSTimer>>>,
    progress_report_timer: RefCell<Option<Retained<NSTimer>>>,
    /// `titleFromTerminal`.
    title_from_terminal: RefCell<Option<String>>,

    // Keeps the `title` observer (which cancels the fallback timer) alive.
    _title_sub: RefCell<Option<crate::helpers::published::Subscription>>,
}

impl SurfaceViewIvars {
    fn new(id: Retained<NSUUID>) -> Self {
        Self {
            id,
            surface_model: RefCell::new(None),
            title: Published::new(String::new()),
            pwd: Published::new(None),
            cell_size: Published::new(NSSize::new(0.0, 0.0)),
            healthy: Published::new(true),
            hover_url: Published::new(None),
            progress_report: Published::new(None),
            key_sequence: Published::new(Vec::new()),
            key_tables: Published::new(Vec::new()),
            search_state: Published::new(None),
            pointer_style: Published::new(CursorStyle::HorizontalText),
            cursor_visible: Published::new(true),
            bell: Published::new(false),
            readonly: Published::new(false),
            initial_size: Cell::new(None),
            password_input: Cell::new(false),
            scrollbar: RefCell::new(None),
            notification_identifiers: RefCell::new(HashSet::new()),
            focused: Cell::new(true),
            title_change_timer: RefCell::new(None),
            title_fallback_timer: RefCell::new(None),
            progress_report_timer: RefCell::new(None),
            title_from_terminal: RefCell::new(None),
            _title_sub: RefCell::new(None),
        }
    }
}

define_class!(
    /// The NSView implementation for a terminal surface.
    // SAFETY: NSView has no subclassing requirements we violate; no Drop impl.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "GhosttySurfaceView"]
    #[ivars = SurfaceViewIvars]
    pub struct SurfaceView;

    unsafe impl NSObjectProtocol for SurfaceView {}

    impl SurfaceView {
        /// We need to support being a first responder so that we can get input events.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }
    }

    // TODO(MA2): everything else of SurfaceView_AppKit.swift: input events,
    // NSTextInputClient, tracking areas, layers, drag & drop, services.
);

impl SurfaceView {
    /// Create a view without a `ghostty_surface_t` yet.
    /// TODO(MA2): Swift `init(_ app:baseConfig:uuid:)` (SurfaceView_AppKit.swift:256)
    /// creates the surface with `ghostty_surface_new` and sets up the layer,
    /// tracking areas, the app-wide config and the title fallback.
    #[must_use]
    pub fn new_model(mtm: MainThreadMarker, uuid: Option<Retained<NSUUID>>) -> Retained<Self> {
        let id = uuid.unwrap_or_default();
        let this = Self::alloc(mtm).set_ivars(SurfaceViewIvars::new(id));
        // SAFETY: `initWithFrame:` is the designated initializer of NSView.
        let this: Retained<Self> = unsafe {
            msg_send![super(this), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0))]
        };

        // `title` didSet: a non-empty title cancels the fallback timer.
        let weak = Weak::from_retained(&this);
        let bound = MainThreadBound::new(weak, mtm);
        let sub = this.ivars().title.sink(move |title| {
            if title.is_empty() {
                return;
            }
            // Always on the main thread.
            let mtm = MainThreadMarker::new().expect("main thread");
            if let Some(view) = bound.get(mtm).load()
                && let Some(t) = view.ivars().title_fallback_timer.borrow_mut().take()
            {
                t.invalidate();
            }
        });
        *this.ivars()._title_sub.borrow_mut() = Some(sub);
        this
    }

    // MARK: Identity and surface

    /// `id` (the UUID).
    #[must_use]
    pub fn id(&self) -> &NSUUID {
        &self.ivars().id
    }

    /// `id.uuidString`.
    #[must_use]
    pub fn id_string(&self) -> String {
        self.ivars().id.UUIDString().to_string()
    }

    /// `surfaceModel = ...` (set once the C surface exists).
    pub fn set_surface_model(&self, surface: Option<Surface>) {
        *self.ivars().surface_model.borrow_mut() = surface;
    }

    /// `surface`: the underlying C value.
    #[must_use]
    pub fn surface(&self) -> Option<ghostty_surface_t> {
        self.ivars()
            .surface_model
            .borrow()
            .as_ref()
            .map(Surface::unsafe_c_value)
    }

    /// `needsConfirmQuit`.
    #[must_use]
    pub fn needs_confirm_quit(&self) -> bool {
        match self.surface() {
            // SAFETY: valid surface owned by this view.
            Some(s) => unsafe { ghostty_surface_needs_confirm_quit(s) },
            None => false,
        }
    }

    /// `processExited`.
    #[must_use]
    pub fn process_exited(&self) -> bool {
        match self.surface() {
            // SAFETY: valid surface owned by this view.
            Some(s) => unsafe { ghostty_surface_process_exited(s) },
            None => true,
        }
    }

    // MARK: Published state

    /// `title`.
    #[must_use]
    pub fn title(&self) -> String {
        self.ivars().title.get()
    }

    /// `focused`.
    #[must_use]
    pub fn focused(&self) -> bool {
        self.ivars().focused.get()
    }

    /// TODO(MA2): `focusDidChange(_:)` (SurfaceView_AppKit.swift:~447) is the
    /// real setter (notifies libghostty, secure input, notification cleanup).
    pub fn set_focused_model(&self, focused: bool) {
        self.ivars().focused.set(focused);
    }

    /// `passwordInput`.
    #[must_use]
    pub fn password_input(&self) -> bool {
        self.ivars().password_input.get()
    }

    /// `passwordInput = ...`.
    /// TODO(integration): the Swift `didSet` updates `SecureInput.shared`
    /// (`setScoped`/`removeScoped`) -- wire once features/secure_input lands (MA4).
    pub fn set_password_input(&self, value: bool) {
        self.ivars().password_input.set(value);
    }

    /// `pwd = ...`.
    pub fn set_pwd(&self, pwd: Option<String>) {
        self.ivars().pwd.set(pwd);
    }

    /// `cellSize = ...`.
    pub fn set_cell_size(&self, size: NSSize) {
        self.ivars().cell_size.set(size);
    }

    /// `initialSize = ...`.
    pub fn set_initial_size(&self, size: Option<NSSize>) {
        self.ivars().initial_size.set(size);
    }

    /// `hoverUrl = ...`.
    pub fn set_hover_url(&self, url: Option<String>) {
        self.ivars().hover_url.set(url);
    }

    /// `progressReport = ...`: also restarts the 15 second expiry timer.
    pub fn set_progress_report(&self, report: Option<ProgressReport>) {
        // Cancel any existing timer.
        if let Some(t) = self.ivars().progress_report_timer.borrow_mut().take() {
            t.invalidate();
        }
        let has_report = report.is_some();
        self.ivars().progress_report.set(report);

        // If we have a new progress report, start a timer to remove it after 15 seconds.
        if has_report {
            let weak = Weak::from_retained(&self.retain());
            let bound = MainThreadBound::new(weak, self.mtm());
            let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
                let mtm = MainThreadMarker::new().expect("main thread");
                if let Some(view) = bound.get(mtm).load() {
                    // Like Swift's `self?.progressReport = nil` (which re-runs didSet).
                    view.set_progress_report(None);
                    *view.ivars().progress_report_timer.borrow_mut() = None;
                }
            });
            // SAFETY: the block only runs on the main run loop.
            let timer = unsafe {
                NSTimer::scheduledTimerWithTimeInterval_repeats_block(15.0, false, &block)
            };
            *self.ivars().progress_report_timer.borrow_mut() = Some(timer);
        }
    }

    /// `searchState = ...`.
    /// TODO(MA2): SurfaceView_AppKit.swift:72 `didSet` -- the Combine pipeline
    /// that sends `search:<needle>` (debounced 300ms when 1-2 chars) and the
    /// `end_search` binding action when cleared.
    pub fn set_search_state(&self, state: Option<Rc<SearchState>>) {
        let had = self.ivars().search_state.get().is_some();
        let clearing = state.is_none() && had;
        self.ivars().search_state.set(state);
        if clearing && let Some(surface) = self.ivars().surface_model.borrow().as_ref() {
            let _ = surface.perform("end_search");
        }
    }

    /// `searchState`.
    #[must_use]
    pub fn search_state(&self) -> Option<Rc<SearchState>> {
        self.ivars().search_state.get()
    }

    // MARK: Cursor

    /// `setCursorShape(_:)`.
    pub fn set_cursor_shape(&self, shape: c_int) {
        let style = match shape {
            GHOSTTY_MOUSE_SHAPE_DEFAULT => CursorStyle::Default,
            GHOSTTY_MOUSE_SHAPE_TEXT => CursorStyle::HorizontalText,
            GHOSTTY_MOUSE_SHAPE_GRAB => CursorStyle::GrabIdle,
            GHOSTTY_MOUSE_SHAPE_GRABBING => CursorStyle::GrabActive,
            GHOSTTY_MOUSE_SHAPE_POINTER => CursorStyle::Link,
            GHOSTTY_MOUSE_SHAPE_W_RESIZE => CursorStyle::ResizeLeft,
            GHOSTTY_MOUSE_SHAPE_E_RESIZE => CursorStyle::ResizeRight,
            GHOSTTY_MOUSE_SHAPE_N_RESIZE => CursorStyle::ResizeUp,
            GHOSTTY_MOUSE_SHAPE_S_RESIZE => CursorStyle::ResizeDown,
            GHOSTTY_MOUSE_SHAPE_NS_RESIZE => CursorStyle::ResizeUpDown,
            GHOSTTY_MOUSE_SHAPE_EW_RESIZE => CursorStyle::ResizeLeftRight,
            GHOSTTY_MOUSE_SHAPE_VERTICAL_TEXT => CursorStyle::VerticalText,
            GHOSTTY_MOUSE_SHAPE_CONTEXT_MENU => CursorStyle::ContextMenu,
            GHOSTTY_MOUSE_SHAPE_CROSSHAIR => CursorStyle::Crosshair,
            GHOSTTY_MOUSE_SHAPE_NOT_ALLOWED => CursorStyle::OperationNotAllowed,
            // We ignore unknown shapes.
            _ => return,
        };
        // TODO(MA2): the Swift view applies `pointerStyle` through SwiftUI's
        // `.backport.pointerStyle` / cursor rects.
        self.ivars().pointer_style.set(style);
    }

    /// `setCursorVisibility(_:)`.
    pub fn set_cursor_visibility(&self, visible: bool) {
        self.ivars().cursor_visible.set(visible);
        // Technically this action could be called anytime we want to change the
        // mouse visibility but at the time of writing this mouse-hide-while-typing
        // is the only use case so this is the preferred method.
        NSCursor::setHiddenUntilMouseMoves(!visible);
    }

    // MARK: Title

    /// `setTitle(_:)`: coalesces rapid changes with a 75 ms timer.
    pub fn set_title(&self, title: &str) {
        if let Some(t) = self.ivars().title_change_timer.borrow_mut().take() {
            t.invalidate();
        }
        let weak = Weak::from_retained(&self.retain());
        let bound = MainThreadBound::new(weak, self.mtm());
        let title = title.to_owned();
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            let mtm = MainThreadMarker::new().expect("main thread");
            let Some(view) = bound.get(mtm).load() else {
                return;
            };
            // Set the title if it wasn't manually set.
            if view.ivars().title_from_terminal.borrow().is_some() {
                *view.ivars().title_from_terminal.borrow_mut() = Some(title.clone());
                return;
            }
            view.ivars().title.set(title.clone());
        });
        // SAFETY: the block only runs on the main run loop.
        let timer =
            unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.075, false, &block) };
        *self.ivars().title_change_timer.borrow_mut() = Some(timer);
    }

    /// `promptTitle()`: set the title by prompting the user.
    pub fn prompt_title(&self) {
        let mtm = self.mtm();
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("Change Terminal Title"));
        alert.setInformativeText(&NSString::from_str("Leave blank to restore the default."));
        alert.setAlertStyle(NSAlertStyle::Informational);

        // Add a text field to the alert.
        let text_field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(250.0, 24.0)),
        );
        text_field.setStringValue(&NSString::from_str(&self.title()));
        alert.setAccessoryView(Some(&text_field));

        alert.addButtonWithTitle(&NSString::from_str("OK"));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));

        // Make the text field the first responder so it gets focus.
        alert.window().setInitialFirstResponder(Some(&text_field));

        let weak = Weak::from_retained(&self.retain());
        let field = text_field.clone();
        let completion = move |response: NSModalResponse| {
            let Some(this) = weak.load() else { return };

            // Check if the user clicked "OK".
            if response != NSAlertFirstButtonReturn {
                return;
            }

            // Get the input text.
            let new_title = field.stringValue().to_string();
            if new_title.is_empty() {
                // Empty means that user wants the title to be set automatically.
                // We also need to reload the config for the "title" property to be
                // used again by this tab.
                let prev_title = this
                    .ivars()
                    .title_from_terminal
                    .borrow_mut()
                    .take()
                    .unwrap_or_else(|| "\u{1F47B}".to_owned());
                this.set_title(&prev_title);
            } else {
                // Set the title and prevent it from being changed automatically.
                *this.ivars().title_from_terminal.borrow_mut() = Some(this.title());
                this.ivars().title.set(new_title);
            }
        };

        // We prefer to run our alert in a sheet modal if we have a window.
        if let Some(window) = self.window() {
            let block = RcBlock::new(completion);
            alert.beginSheetModalForWindow_completionHandler(&window, Some(&block));
        } else {
            // On macOS 26 RC, this codepath results in the "OK" button not being
            // visible (upstream note); the sheet path is taken most times.
            completion(alert.runModal());
        }
    }

    // MARK: User notifications

    /// `showUserNotification(title:body:requireFocus:)`: show a user
    /// notification and associate it with this surface.
    pub fn show_user_notification(&self, title: &str, body: &str, require_focus: bool) {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setSubtitle(&NSString::from_str(&self.title()));
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        content.setCategoryIdentifier(&NSString::from_str(USER_NOTIFICATION_CATEGORY));

        let surface_key = NSString::from_str("surface");
        let focus_key = NSString::from_str("requireFocus");
        let surface_val: Retained<AnyObject> =
            Retained::into_super(Retained::into_super(NSString::from_str(&self.id_string())));
        let focus_val: Retained<AnyObject> = Retained::into_super(Retained::into_super(
            Retained::into_super(NSNumber::new_bool(require_focus)),
        ));
        let user_info: Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_retained_objects(
                &[&*surface_key, &*focus_key],
                &[surface_val, focus_val],
            );
        // SAFETY: the dictionary holds property list objects.
        unsafe { content.setUserInfo(&Retained::cast_unchecked::<NSDictionary>(user_info)) };

        let uuid = NSUUID::new().UUIDString().to_string();

        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&uuid),
            &content,
            None,
        );

        // The completion handler may be executed on a background thread so we
        // hop to the main queue (Swift: `@MainActor error in`) before touching
        // view state. `MainThreadBound` is `Send + Sync` and only yields its
        // value on the main thread.
        let mtm = self.mtm();
        let bound = Arc::new(MainThreadBound::new(
            (Weak::from_retained(&self.retain()), uuid),
            mtm,
        ));
        let handler = RcBlock::new(move |err: *mut NSError| {
            // SAFETY: the error pointer is null or a valid NSError for the call.
            if let Some(err) = unsafe { err.as_ref() } {
                error!(
                    "Error scheduling user notification: {}",
                    err.localizedDescription()
                );
                return;
            }
            let bound = bound.clone();
            DispatchQueue::main().exec_async(move || {
                let mtm = MainThreadMarker::new().expect("main thread");
                let (weak, uuid) = bound.get(mtm);
                let Some(this) = weak.load() else { return };

                // We need to keep track of this notification so we can remove it
                // under certain circumstances.
                this.ivars()
                    .notification_identifiers
                    .borrow_mut()
                    .insert(uuid.clone());

                // If we're focused then we schedule to remove the notification
                // after a few seconds. If we gain focus we automatically remove it
                // in focusDidChange.
                if this.focused() {
                    let inner =
                        MainThreadBound::new((Weak::from_retained(&this), uuid.clone()), mtm);
                    let block = RcBlock::new(move |_t: NonNull<NSTimer>| {
                        let mtm = MainThreadMarker::new().expect("main thread");
                        let (weak, uuid) = inner.get(mtm);
                        if let Some(this) = weak.load() {
                            this.ivars()
                                .notification_identifiers
                                .borrow_mut()
                                .remove(uuid);
                        }
                        let ids = NSArray::from_retained_slice(&[NSString::from_str(uuid)]);
                        UNUserNotificationCenter::currentNotificationCenter()
                            .removeDeliveredNotificationsWithIdentifiers(&ids);
                    });
                    // SAFETY: the block only runs on the main run loop.
                    let _ = unsafe {
                        NSTimer::scheduledTimerWithTimeInterval_repeats_block(3.0, false, &block)
                    };
                }
            });
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, Some(&handler));
    }

    /// `handleUserNotification(notification:focus:)`: handle a user
    /// notification click.
    pub fn handle_user_notification(&self, notification: &UNNotification, focus: bool) {
        // SAFETY: plain property getters.
        let id = notification.request().identifier().to_string();
        if !self
            .ivars()
            .notification_identifiers
            .borrow_mut()
            .remove(&id)
        {
            return;
        }
        if focus {
            if let Some(window) = self.window() {
                window.makeKeyAndOrderFront(Some(&self.retain()));
            }
            move_focus(self, None, None);
        }
    }

    /// `cellSize = convertFromBacking(size)` helper used by `Ghostty.App.setCellSize`.
    #[must_use]
    pub fn convert_size_from_backing(&self, size: NSSize) -> NSSize {
        self.convertSizeFromBacking(size)
    }

    /// `ProgressReport` removal check used by `Ghostty.App.progressReport`.
    #[must_use]
    pub fn is_progress_remove(report: &ProgressReport) -> bool {
        report.state == ProgressReportState::Remove
    }
}

/// `Ghostty.moveFocus(to:from:delay:)` (`Surface View/SurfaceView.swift:1150`).
///
/// Moves first-responder focus to `to`, retrying with a doubling back-off if
/// the view is not in a window yet.
pub fn move_focus(to: &SurfaceView, from: Option<&SurfaceView>, delay: Option<f64>) {
    // The whole delay machinery is a bit of a hack to work around a situation
    // where the window is destroyed and the surface view will never be attached
    // to a window. Realistically, we should handle this upstream but we also
    // don't want this function to be a source of infinite loops.
    let max_delay = 0.5;
    if delay.unwrap_or(0.0) >= max_delay {
        return;
    }

    // We start at a 50 millisecond delay and do a doubling backoff.
    let next_delay = delay.map_or(0.05, |d| d * 2.0);

    let mtm = to.mtm();
    let to_weak = Weak::from_retained(&to.retain());
    let from_weak = from.map(|f| Weak::from_retained(&f.retain()));
    let bound = MainThreadBound::new((to_weak, from_weak), mtm);

    let work = move || {
        let mtm = MainThreadMarker::new().expect("main thread");
        let (to_weak, from_weak) = bound.get(mtm);
        let Some(to) = to_weak.load() else { return };
        let from = from_weak.as_ref().and_then(Weak::load);

        // If the callback runs before the surface is attached to a view then the
        // window will be nil. We just reschedule in that case.
        let Some(window) = to.window() else {
            move_focus(&to, from.as_deref(), Some(next_delay));
            return;
        };

        // If we had a previously focused node and it's not where we're sending
        // focus, make sure that we explicitly tell it to lose focus. In theory we
        // should NOT have to do this but the focus callback isn't getting called
        // for some reason.
        if let Some(from) = from {
            let _ = from.resignFirstResponder();
        }

        let _ = window.makeFirstResponder(Some(&to));
    };

    match delay {
        // The Swift version dispatches async on the main queue (no delay) when
        // none was given, otherwise asyncAfter.
        None => DispatchQueue::main().exec_async(work),
        Some(d) => {
            let work = std::cell::RefCell::new(Some(work));
            let block = RcBlock::new(move |_t: NonNull<NSTimer>| {
                if let Some(w) = work.borrow_mut().take() {
                    w();
                }
            });
            // SAFETY: the block only runs on the main run loop.
            let _ =
                unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(d, false, &block) };
        }
    }
}
