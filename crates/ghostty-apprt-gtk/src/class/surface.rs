//! Port of apprt/gtk/class/surface.zig and apprt/gtk/Surface.zig.
//!
//! `GhosttySurface`: the GTK widget that hosts a terminal surface. It is a
//! `AdwBin` subclass implementing `GtkScrollable` (so it can live in a
//! scrolled window), embedding a `GtkGLArea` that the core renders into.
//!
//! The "apprt Surface" of Zig (`Surface.zig`, the object the core calls back
//! into) is [`RtSurfaceHandle`], which implements [`ghostty_apprt::runtime_api::RtSurface`]
//! and holds only a weak reference to the widget.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io::Write as _,
    rc::Rc,
    sync::OnceLock,
};

use adw::{prelude::*, subclass::prelude::*};
use ghostty_apprt::{
    action::{CommandFinished, Inspector, KeySequence, KeyTable, Readonly},
    runtime_api::{
        ClipboardError,
        CoreSurface,
        DesiredFontSize,
        InputEffect,
        MIN_WINDOW_HEIGHT_CELLS,
        MIN_WINDOW_WIDTH_CELLS,
        RtSurface,
        SurfaceId,
    },
    structs::{Clipboard, ClipboardRequest, ContentScale, CursorPos, ImePos, SurfaceSize},
    surface::{ChildExited, NewSurfaceContext, Scrollbar},
};
use ghostty_config::{
    command::Command,
    config::Config as CoreConfig,
    path::Path as ConfigPath,
    types_a::WorkingDirectory,
    types_b::{
        BellFeatures,
        NotifyOnCommandFinish,
        ResizeOverlay as ResizeOverlayMode,
        ResizeOverlayPosition,
    },
};
use ghostty_input::{Action as KeyAction, Key, KeyEvent, Mods, MouseButton, MouseButtonState};
use ghostty_vt::{
    mouse_shape::MouseShape,
    osc::{ProgressReport, ProgressState},
};
use gtk::{
    gdk,
    gio,
    glib,
    glib::{
        subclass::Signal,
        translate::{FromGlib, IntoGlib},
    },
};

use crate::{
    class::{
        application::Application,
        clipboard_confirmation_dialog::ClipboardConfirmationDialog,
        config::Config,
        inspector_window::InspectorWindow,
        key_state_overlay::KeyStateOverlay,
        resize_overlay::ResizeOverlay,
        search_overlay::SearchOverlay,
        surface_child_exited::SurfaceChildExited,
        title_dialog::{self, TitleDialog},
        window::Window,
    },
    key as gtk_key,
};

/// Simple dimensions struct for the surface used by various properties
/// (Zig: `Surface.Size`, GType `GhosttySurfaceSize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, glib::Boxed)]
#[boxed_type(name = "GhosttySurfaceSize", nullable)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// Boxed `font.face.DesiredSize` for the `font-size-request` property.
#[derive(Debug, Clone, Copy, PartialEq, glib::Boxed)]
#[boxed_type(name = "GhosttyFontDesiredSize", nullable)]
pub struct FontSizeRequest(pub DesiredFontSize);

/// The state of the key event while we're doing IM composition.
/// See `Surface::key_event` for detailed descriptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IMKeyEvent {
    /// Not in a key event.
    #[default]
    False,
    /// In a key event but `im_composing` was true prior to IME processing.
    Composing,
    /// In a key event but `im_composing` was false prior to IME processing.
    NotComposing,
}

/// Input method signals for [`Surface::debug_im`].
#[derive(Debug, Clone, PartialEq)]
pub enum DebugIm {
    PreeditStart,
    PreeditChanged,
    PreeditEnd,
    Commit(String),
}

/// What the simulated input method does with a key press in
/// [`Surface::debug_im_key`].
#[derive(Debug, Clone, PartialEq)]
pub enum DebugImKey {
    /// Handled; starts/updates the preedit (dead key, compose).
    Preedit,
    /// Handled; commit then preedit-end (ibus order).
    Commit(String),
    /// Handled; preedit-end then commit (simple order).
    CommitSimple(String),
    /// Handled; commit without composing (plain character translation).
    HandledCommit(String),
    /// Not handled and no signals.
    Pass,
}

/// Overrides applied on top of the config when the core surface is created
/// (Zig: the anonymous struct taking by `Surface.new`).
#[derive(Debug, Clone, Default)]
pub struct SurfaceOverrides {
    pub command: Option<Command>,
    pub working_directory: Option<ghostty_os::osbytes::EncodedOs>,
    pub title: Option<String>,
}

thread_local! {
    /// Registry of live surfaces: id -> weak ref. Used to map
    /// `Target::Surface(id)` coming from the core back to widgets.
    static SURFACES: RefCell<HashMap<SurfaceId, glib::WeakRef<Surface>>> =
        RefCell::new(HashMap::new());
    static NEXT_ID: Cell<SurfaceId> = const { Cell::new(1) };
}

/// Seconds before a stalled progress bar is removed.
const PROGRESS_BAR_TIMEOUT_SECONDS: u32 = 15;

/// `font.face.default_dpi` on Linux.
const DEFAULT_DPI: f32 = 96.0;

fn app() -> Application {
    Application::default()
}

glib::wrapper! {
    pub struct Surface(ObjectSubclass<imp::Surface>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Scrollable, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

// Marker wrapper over the same GType: `glib::Properties` generates property
// accessors (`title()`, `set_title()`, `notify_title()`, ...) on its
// `wrapper_type`. We want Zig-shaped hand-written accessors on `Surface`
// instead, so the generated ones land on this private alias type.
glib::wrapper! {
    #[allow(dead_code)]
    pub(self) struct SurfaceProps(ObjectSubclass<imp::Surface>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Scrollable, gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Surface {
    fn default() -> Self {
        Self::new(SurfaceOverrides::default())
    }
}

// ---------------------------------------------------------------------
// Construction / lookup

/// Restore the `LANG` the application was launched with (as bytes; it need
/// not be UTF-8), or remove it if it was unset.
fn restore_language(env: &mut ghostty_os::EnvMap, saved: Option<&std::ffi::OsStr>) {
    if let Some(language) = saved {
        env.insert("LANG", language);
    } else {
        env.remove("LANG");
    }
}

impl Surface {
    /// Create a new surface widget (Zig: `Surface.new`).
    pub fn new(overrides: SurfaceOverrides) -> Self {
        crate::register_resources();
        let title = overrides.title.clone();
        let obj: Self = glib::Object::builder()
            .property("title-override", title)
            .build();
        obj.imp().overrides.replace(SurfaceOverrides {
            title: None,
            ..overrides
        });
        obj
    }

    /// The surface id handed to the core (`Target::Surface(id)`).
    pub fn id(&self) -> SurfaceId {
        self.imp().id.get()
    }

    /// Look up a live surface by id.
    pub fn from_id(id: SurfaceId) -> Option<Surface> {
        SURFACES.with(|m| m.borrow().get(&id).and_then(glib::WeakRef::upgrade))
    }

    /// All live surfaces.
    pub fn all() -> Vec<Surface> {
        SURFACES.with(|m| {
            m.borrow()
                .values()
                .filter_map(glib::WeakRef::upgrade)
                .collect()
        })
    }

    /// The core surface, if initialized (Zig: `core()`).
    pub fn core(&self) -> Option<Rc<dyn CoreSurface>> {
        self.imp().core_surface.borrow().clone()
    }

    /// The apprt surface handle (Zig: `rt()`).
    pub fn rt(&self) -> Rc<RtSurfaceHandle> {
        let existing = self.imp().rt_surface.borrow().clone();
        existing.unwrap_or_else(|| {
            let h = Rc::new(RtSurfaceHandle {
                surface: self.downgrade(),
            });
            self.imp().rt_surface.replace(Some(h.clone()));
            h
        })
    }

    /// Set the parent of this surface. This will extract the information
    /// required to initialize this surface with the proper values but doesn't
    /// retain any memory.
    ///
    /// If the surface is already realized this does nothing.
    ///
    /// (Zig took the parent `*CoreSurface` and went through its rt surface;
    /// here the parent widget is passed directly.)
    pub fn set_parent(&self, parent: &Surface, context: NewSurfaceContext) {
        let imp = self.imp();

        // This is a mistake! We can only set a parent before surface
        // realization. We log this because this is probably a logic error.
        if imp.core_surface.borrow().is_some() {
            log::warn!("setParent called after surface is already realized");
            return;
        }

        // Store the context so init_surface can use it
        imp.context.set(context);

        // Setup our font size
        if let Some(core) = parent.core() {
            imp.font_size_request
                .replace(Some(FontSizeRequest(core.font_size())));
            self.notify("font-size-request");
        }

        // Remainder needs a config. If there is no config we just assume
        // we aren't inheriting any of these values.
        let config = imp.config.borrow().clone();
        if let Some(config_obj) = config {
            // Setup our cwd if configured to inherit
            if ghostty_apprt::surface::should_inherit_working_directory(context, &config_obj.get())
                && let Some(pwd) = parent.get_pwd()
            {
                imp.pwd.replace(Some(pwd));
                self.notify("pwd");
            }
        }
    }

    /// Force the surface to redraw itself. Ghostty often will only redraw
    /// the terminal in reaction to internal changes. If there are external
    /// events that invalidate the surface, such as the widget moving parents,
    /// then we should force a redraw.
    pub fn redraw(&self) {
        self.imp().gl_area.queue_render();
    }

    pub fn toggle_fullscreen(&self) {
        self.emit_by_name::<()>("toggle-fullscreen", &[]);
    }

    pub fn toggle_maximize(&self) {
        self.emit_by_name::<()>("toggle-maximize", &[]);
    }

    pub fn toggle_command_palette(&self) -> bool {
        // TODO: pass the surface with the action
        self.activate_action("win.toggle-command-palette", None)
            .is_ok()
    }

    /// Show/hide the inspector (Zig: `controlInspector`).
    pub fn control_inspector(&self, value: Inspector) -> bool {
        // Let's see if we have an inspector already.
        let priv_ = self.imp();
        if let Some(inspector) = priv_.inspector.upgrade() {
            match value {
                Inspector::Show => {}
                // Our weak ref will set our private value to null
                Inspector::Toggle | Inspector::Hide => inspector.destroy(),
            }
        } else {
            match value {
                Inspector::Toggle | Inspector::Show => {
                    let inspector = InspectorWindow::new(self);
                    inspector.present_window();
                    priv_.inspector.set(Some(&inspector));
                }
                Inspector::Hide => {}
            }
        }

        true
    }

    /// Redraw our inspector, if there is one associated with this surface.
    pub fn redraw_inspector(&self) {
        if let Some(v) = self.imp().inspector.upgrade() {
            v.queue_render();
        }
    }

    /// Debug input replay: click in the inspector window. Returns false if
    /// there is no inspector.
    pub fn debug_inspector_click(&self, x: f64, y: f64) -> bool {
        let Some(v) = self.imp().inspector.upgrade() else {
            return false;
        };
        v.debug_click(x, y);
        true
    }

    /// Debug input replay: scroll in the inspector window. Returns false if
    /// there is no inspector.
    pub fn debug_inspector_scroll(&self, x: f64, y: f64, dy: f64) -> bool {
        let Some(v) = self.imp().inspector.upgrade() else {
            return false;
        };
        v.debug_scroll(x, y, dy);
        true
    }

    /// Debug input replay: write the next rendered frame of the inspector
    /// window to `path` (PNG). Returns false if there is no inspector.
    pub fn debug_inspector_screenshot(&self, path: &str) -> bool {
        let Some(v) = self.imp().inspector.upgrade() else {
            return false;
        };
        v.debug_screenshot(path);
        true
    }

    /// Handle a key sequence action from the apprt.
    pub fn key_sequence_action(&self, value: &KeySequence) {
        let imp = self.imp();
        let _guard = self.freeze_notify();
        self.notify("key-sequence");

        match value {
            KeySequence::Trigger(trigger) => {
                // Convert the trigger to a human-readable label
                if let Some(label) = gtk_key::label_from_trigger(*trigger) {
                    imp.key_sequence.borrow_mut().push(label);
                }
            }
            KeySequence::End => imp.key_sequence.borrow_mut().clear(),
        }
    }

    /// Handle a key table action from the apprt.
    pub fn key_table_action(&self, value: &KeyTable) {
        let imp = self.imp();
        let _guard = self.freeze_notify();
        self.notify("key-table");

        match value {
            KeyTable::Activate(name) => imp.key_tables.borrow_mut().push(name.to_text_lossy()),
            KeyTable::Deactivate => {
                imp.key_tables.borrow_mut().pop();
            }
            KeyTable::DeactivateAll => imp.key_tables.borrow_mut().clear(),
        }
    }

    /// `showOnScreenKeyboard(null)`: used by the `show_on_screen_keyboard`
    /// action.
    pub fn show_on_screen_keyboard(&self) -> bool {
        self.show_on_screen_keyboard_event(None)
    }

    /// `showOnScreenKeyboard(event)`.
    pub fn show_on_screen_keyboard_event(&self, event: Option<&gdk::Event>) -> bool {
        self.imp().im_context.activate_osk(event)
    }

    /// Set the scrollbar state for this surface. This will setup the
    /// properties for our `Gtk.Scrollable` interface properly.
    pub fn set_scrollbar(&self, scrollbar: Scrollbar) {
        // Update existing adjustment in-place. If we don't have an
        // adjustment then we do nothing because we're not part of a
        // scrolled window.
        let Some(vadj) = self.get_vadjustment() else {
            return;
        };

        // Check if values match existing adjustment and skip update if so
        let value = scrollbar.offset as f64;
        let upper = scrollbar.total as f64;
        let page_size = scrollbar.len as f64;

        if (vadj.value() - value).abs() < 0.001
            && (vadj.upper() - upper).abs() < 0.001
            && (vadj.page_size() - page_size).abs() < 0.001
        {
            return;
        }

        // During manual scrollbar changes from Ghostty core we don't
        // want to emit value-changed signals so we block them. This would
        // cause a waste of resources at best and infinite loops at worst.
        let handler = self.imp().vadj_handler.borrow();
        if let Some((a, id)) = handler.as_ref() {
            a.block_signal(id);
        }
        vadj.configure(
            value,     // value: current scroll position
            0.0,       // lower: minimum value
            upper,     // upper: maximum value (total scrollable area)
            1.0,       // step_increment: amount to scroll on arrow click
            page_size, // page_increment: amount to scroll on page up/down
            page_size, // page_size: size of visible area
        );
        if let Some((a, id)) = handler.as_ref() {
            a.unblock_signal(id);
        }
    }

    /// Set the current progress report state.
    pub fn set_progress_report(&self, value: ProgressReport) {
        let imp = self.imp();

        // No matter what, we stop the timer because if we're removing
        // then we're done and otherwise we restart it.
        if let Some(timer) = imp.progress_bar_timer.take() {
            timer.remove();
        }

        let config = imp.config.borrow().clone();
        if let Some(config) = config
            && !config.get().progress_style
        {
            log::debug!("progress_report action blocked by config");
            imp.progress_bar_overlay.set_visible(false);
            return;
        }

        let progress_bar = &*imp.progress_bar_overlay;
        match value.state {
            // Remove the progress bar
            ProgressState::Remove => {
                progress_bar.set_visible(false);
                return;
            }

            // Set the progress bar to a fixed value if one was provided,
            // otherwise pulse. Remove the `error` CSS class so that the
            // progress bar shows as normal.
            ProgressState::Set => {
                progress_bar.remove_css_class("error");
                match value.progress {
                    Some(p) => progress_bar.set_fraction(compute_fraction(p)),
                    None => progress_bar.pulse(),
                }
            }

            // Same, with the `error` CSS class set.
            ProgressState::Error => {
                progress_bar.add_css_class("error");
                match value.progress {
                    Some(p) => progress_bar.set_fraction(compute_fraction(p)),
                    None => progress_bar.pulse(),
                }
            }

            // The state of progress is unknown, so pulse the progress bar to
            // indicate that things are still happening.
            ProgressState::Indeterminate => progress_bar.pulse(),

            // If a progress value was provided, set the progress bar to that
            // value. Don't pulse the progress bar as that would indicate that
            // things were happening. Otherwise this is mainly used to keep
            // the progress bar on screen instead of timing out.
            ProgressState::Pause => {
                if let Some(p) = value.progress {
                    progress_bar.set_fraction(compute_fraction(p));
                }
            }
        }

        // Assume all states lead to visibility
        progress_bar.set_visible(true);

        // Start our timer to remove bad actor programs that stall
        // the progress bar.
        let weak = self.downgrade();
        let id = glib::timeout_add_seconds_local(PROGRESS_BAR_TIMEOUT_SECONDS, move || {
            if let Some(this) = weak.upgrade() {
                // The source is removed by returning Break; forget the id.
                let _ = this.imp().progress_bar_timer.take();
                this.set_progress_report(ProgressReport {
                    state: ProgressState::Remove,
                    progress: None,
                });
            }
            glib::ControlFlow::Break
        });
        imp.progress_bar_timer.set(Some(id));
    }

    /// Request that this terminal come to the front and become focused.
    /// It is up to the embedding widget to react to this.
    pub fn present(&self) {
        self.emit_by_name::<()>("present-request", &[]);
    }

    pub fn command_finished(&self, value: &CommandFinished) -> bool {
        let imp = self.imp();

        let notify_next_command_finish = 'notify: {
            let Some(group) = imp.action_group.borrow().clone() else {
                break 'notify false;
            };
            let Some(state) = group.action_state("notify-on-next-command-finish") else {
                break 'notify false;
            };
            let Some(notify) = state.get::<bool>() else {
                break 'notify false;
            };
            group.change_action_state("notify-on-next-command-finish", &false.to_variant());
            notify
        };

        let Some(config_obj) = imp.config.borrow().clone() else {
            return false;
        };
        let cfg = config_obj.get();

        if !notify_next_command_finish {
            if cfg.notify_on_command_finish == NotifyOnCommandFinish::Never {
                return true;
            }
            if cfg.notify_on_command_finish == NotifyOnCommandFinish::Unfocused
                && self.get_focused()
            {
                return true;
            }
        }

        if value.duration.lte(cfg.notify_on_command_finish_after) {
            return true;
        }

        let action = cfg.notify_on_command_finish_action;

        if action.bell {
            self.set_bell_ringing(true);
        }

        if action.notify {
            use ghostty_os::i18n::tr;
            let title = match value.exit_code {
                None => tr("Command Finished"),
                Some(0) => tr("Command Succeeded"),
                Some(_) => tr("Command Failed"),
            };
            let rounded = value.duration.round(1_000_000);
            let body = match value.exit_code {
                None => format!("Command took {rounded}."),
                Some(code) => format!("Command took {rounded} and exited with code {code}."),
            };
            self.send_desktop_notification(&title, &body);
        }

        true
    }

    /// Get the readonly state from the core surface.
    pub fn get_readonly(&self) -> bool {
        self.core().is_some_and(|c| c.readonly())
    }

    /// Notify anyone interested that the readonly status has changed.
    pub fn set_readonly(&self, _value: Readonly) -> bool {
        self.notify("readonly");
        true
    }

    /// Key press event (press or release).
    ///
    /// At a high level, we want to construct an `input.KeyEvent` and
    /// pass that to `key_callback`. At a low level, this is more complicated
    /// than it appears because we need to construct all of this information
    /// and its not given to us.
    ///
    /// For all events, we run the GdkEvent through the input method context.
    /// This allows the input method to capture the event and trigger
    /// callbacks such as preedit, commit, etc.
    ///
    /// There are a couple important aspects to the prior paragraph: we must
    /// send ALL events through the input method context. This is because
    /// input methods use both key press and key release events to determine
    /// the state of the input method. For example, fcitx uses key release
    /// events on modifiers (i.e. ctrl+shift) to switch the input method.
    ///
    /// We set some state to note we're in a key event (`in_keyevent`)
    /// because some of the input method callbacks change behavior based on
    /// this state. For example, we don't want to send character events
    /// like "a" via the input "commit" event if we're actively processing
    /// a keypress because we'd lose access to the keycode information.
    /// However, a "commit" event may still happen outside of a keypress
    /// event from e.g. a tablet or on-screen keyboard.
    ///
    /// Finally, we take all of the information in order to determine if we have
    /// a unicode character or if we have to map the keyval to a code to
    /// get the underlying logical key, etc.
    ///
    /// Then we can emit the `key_callback`.
    pub fn key_event(
        &self,
        action: KeyAction,
        ec_key: &gtk::EventControllerKey,
        keyval: u32,
        keycode: u32,
        gtk_mods: gdk::ModifierType,
    ) -> bool {
        let imp = self.imp();
        let Some(event) = ec_key.current_event() else {
            return false;
        };
        let Ok(key_event) = event.clone().downcast::<gdk::KeyEvent>() else {
            return false;
        };

        // The input method handling lives in `im_filter` (shared with the
        // debug replay, which drives it with a simulated input method).
        if self.im_filter(|| imp.im_context.filter_keypress(&event)) {
            return true;
        }

        // We always reset the length of the im buffer. There's only one
        // scenario we reach this point with im_len > 0 and that's if we
        // received a commit event from the input method. We don't want to
        // keep that state around since we've handled it here.
        let result = self.key_event_encode(
            action,
            &KeySource::Event {
                event: &event,
                key_event: &key_event,
            },
            keyval,
            keycode,
            gtk_mods,
        );
        imp.im_buf.borrow_mut().clear();
        result
    }

    /// The input method half of [`Surface::key_event`]. `filter` runs the
    /// event through the input method context (`filter_keypress`) and returns
    /// whether it was handled. Returns true if the key event must not be
    /// encoded.
    fn im_filter(&self, filter: impl FnOnce() -> bool) -> bool {
        let imp = self.imp();
        // The block below is all related to input method handling. See the
        // function comment for some high level details and then the comments
        // within the block for more specifics.
        {
            // This can trigger an input method so we need to notify the im
            // context where the cursor is so it can render the dropdowns in
            // the correct place.
            if let Some(surface) = self.core() {
                let p: ImePos = surface.ime_point();
                imp.im_context
                    .set_cursor_location(&gdk::Rectangle::new(p.x as i32, p.y as i32, 1, 1));
            }

            // We note that we're in a keypress because we want some logic to
            // depend on this. For example, we don't want to send character
            // events like "a" via the input "commit" event if we're actively
            // processing a keypress because we'd lose access to the keycode
            // information.
            //
            // We have to maintain some additional state here of whether we
            // were composing because different input methods call the
            // callbacks in different orders. For example, ibus calls commit
            // THEN preedit end but simple calls preedit end THEN commit.
            imp.in_keyevent.set(if imp.im_composing.get() {
                IMKeyEvent::Composing
            } else {
                IMKeyEvent::NotComposing
            });
            let handled = filter();
            let in_keyevent = imp.in_keyevent.replace(IMKeyEvent::False);

            // Pass the event through the input method which returns true if
            // handled. Confusingly, not all events handled by the input
            // method result in this returning true so we have to maintain
            // some additional state about whether we were composing or not
            // to determine if we should proceed with key encoding.
            //
            // Cases where the input method does not mark the event as
            // handled:
            //
            // - If we change the input method via keypress while we have
            //   preedit text, the input method will commit the pending text
            //   but will not mark it as handled. We use the `Composing`
            //   state to detect this case.
            //
            // - If we switch input methods (i.e. via ctrl+shift with fcitx),
            //   the input method will handle the key release event but will
            //   not mark it as handled. I don't know any way to detect this
            //   case so it will result in a key event being sent to the key
            //   callback. For Kitty text encoding, this will result in
            //   modifiers being triggered despite being technically
            //   consumed. At the time of writing, both Kitty and Alacritty
            //   have the same behavior. I know of no way to fix this.
            //
            // If the input method handled the event, you would think we would
            // never proceed with key encoding for Ghostty but that is not the
            // case. Input methods will handle basic character encoding like
            // typing "a" and we want to associate that with the key event.
            // So we have to check additional state to determine if we exit.
            if handled {
                // If we are composing then we're in a preedit state and do
                // not want to encode any keys. For example: type a deadkey
                // such as single quote on a US international keyboard layout.
                if imp.im_composing.get() {
                    return true;
                }

                // If we were composing and now we're not, it means that we
                // committed the text. We also don't want to encode a key
                // event for this. Example: enable Japanese input method,
                // press "konn" and then press enter. The final enter should
                // not be encoded and "konn" (in hiragana) should be written
                // as "こん".
                if in_keyevent == IMKeyEvent::Composing {
                    return true;
                }

                // Not composing and our input method buffer is empty. This
                // could mean that the input method reacted to this event by
                // activating an onscreen keyboard or something equivalent.
                // We don't know. But the input method handled it and didn't
                // give us text so we will just assume we should not encode
                // this. This handles a real scenario when ibus starts the
                // emoji input method (super+.).
                if imp.im_buf.borrow().is_empty() {
                    return true;
                }
            }

            // At this point, for the sake of explanation of internal state:
            // it is possible that im_len > 0 and im_composing == false. This
            // means that we received a commit event from the input method
            // that we want associated with the key event. This is common: its
            // how basic character translation for simple inputs like "a"
            // work.
        }

        false
    }

    /// Debug input replay (`GHOSTTY_DEBUG_INPUT_SCRIPT`): feed a synthesized
    /// key event through the same encoding path as [`Surface::key_event`]
    /// (physical key lookup, keyval translation, modifier translation, then
    /// `key_callback`). The only part skipped is the input method filter
    /// (it needs a real `GdkEvent`); text is committed through
    /// [`Surface::debug_commit_text`] instead.
    ///
    /// `keyval`/`keycode`/`state` are what GTK's key controller would pass.
    pub fn debug_key_event(
        &self,
        action: KeyAction,
        keyval: u32,
        keycode: u32,
        state: gdk::ModifierType,
    ) -> bool {
        let display = self.imp().gl_area.display();
        // What `GdkKeyEvent::keyval()/consumed_modifiers()/layout()` would
        // report for this keycode+state (group 0).
        let (translated, consumed) = display.translate_key(keycode, state, 0).map_or(
            (
                {
                    // SAFETY: GdkKey wraps a plain keyval integer.
                    #[allow(unsafe_code)]
                    unsafe {
                        gdk::Key::from_glib(keyval)
                    }
                },
                gdk::ModifierType::empty(),
            ),
            |t| (t.0, t.3),
        );
        let result = self.key_event_encode(
            action,
            &KeySource::Synthetic {
                translated,
                layout: 0,
                consumed,
            },
            translated.into_glib(),
            keycode,
            state,
        );
        self.imp().im_buf.borrow_mut().clear();
        result
    }

    /// Debug input replay: write the renderer's last frame to `path` (PNG).
    pub fn debug_screenshot(&self, path: &str) {
        let Some(core) = self.core() else { return };
        // The renderer reads back from the GL context.
        self.imp().gl_area.make_current();
        self.imp().gl_area.queue_render();
        match core.write_screenshot(path) {
            Ok(()) => log::warn!("debug input: wrote screenshot {path}"),
            Err(err) => log::error!("debug input: screenshot failed: {err}"),
        }
    }

    // ---- mouse handlers shared by the GTK callbacks and the debug replay ----

    fn mouse_down_impl(
        &self,
        button_raw: u32,
        state: gdk::ModifierType,
        x: f64,
        y: f64,
        assume_focus: bool,
    ) {
        let this = self;

        // Bell stops ringing if any mouse button is pressed.
        this.set_bell_ringing(false);

        // Get our surface. If we don't have one, ignore this.
        let imp = this.imp();
        let Some(core_surface) = this.core() else {
            return;
        };

        // If we don't have focus, grab it.
        let had_focus = assume_focus || imp.gl_area.has_focus();
        if !had_focus {
            imp.gl_area.grab_focus();
        }

        // Report the event
        let button = translate_mouse_button(button_raw);

        // If this click is only transitioning split focus, suppress it so
        // it doesn't get forwarded to the terminal as a mouse event.
        if !had_focus && button == MouseButton::Left {
            imp.suppress_left_mouse_release.set(true);
            return;
        }

        if button == MouseButton::Middle && !imp.gtk_enable_primary_paste.get() {
            return;
        }

        let consumed = {
            let mods = gtk_key::translate_mods(state);
            match core_surface.mouse_button_callback(MouseButtonState::Press, button, mods) {
                Ok(c) => c,
                Err(err) => {
                    log::warn!("error in key callback err={err}");
                    false
                }
            }
        };

        // If a right click isn't consumed, mouse_button_callback selects
        // the hovered word and returns false. We can use this to handle
        // the context menu opening under normal scenarios.
        if !consumed && button == MouseButton::Right {
            this.emit_by_name::<()>("menu", &[]);

            let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
            let popover = imp.context_menu.upcast_ref::<gtk::Popover>();
            popover.set_pointing_to(Some(&rect));
            popover.popup();
        }
    }

    fn mouse_up_impl(&self, button_raw: u32, state: gdk::ModifierType, event: Option<&gdk::Event>) {
        let this = self;

        let imp = this.imp();
        let Some(surface) = this.core() else { return };
        let gtk_mods = state;
        let button = translate_mouse_button(button_raw);

        if button == MouseButton::Left && imp.suppress_left_mouse_release.get() {
            imp.suppress_left_mouse_release.set(false);
            return;
        }

        if button == MouseButton::Middle && !imp.gtk_enable_primary_paste.get() {
            return;
        }

        let mods = gtk_key::translate_mods(gtk_mods);
        let consumed = match surface.mouse_button_callback(MouseButtonState::Release, button, mods)
        {
            Ok(c) => c,
            Err(err) => {
                log::warn!("error in key callback err={err}");
                return;
            }
        };

        // Trigger the on-screen keyboard if we have no selection, and
        // that the mouse event hasn't been intercepted by the callback.
        //
        // It's better to do this here rather than within the core
        // callback since we have direct access to the underlying
        // gdk.Event here.
        if !consumed
            && button == MouseButton::Left
            && !surface.has_selection()
            && !this.show_on_screen_keyboard_event(event)
        {
            log::warn!("failed to activate the on-screen keyboard");
        }
    }

    fn mouse_motion_impl(&self, state: gdk::ModifierType, x: f64, y: f64) {
        let this = self;
        let imp = this.imp();

        let (sx, sy) = this.scaled_coordinates(x, y);
        let pos = CursorPos {
            x: sx as f32,
            y: sy as f32,
        };

        // There seem to be at least two cases where GTK issues a mouse
        // motion event without the cursor actually moving:
        // 1. GLArea is resized under the mouse. This has the unfortunate
        //    side effect of causing focus to potentially change when
        //    `focus-follows-mouse` is enabled.
        // 2. The window title is updated. This can cause the mouse to
        //    unhide incorrectly when hide-mouse-when-typing is enabled.
        // To prevent incorrect behavior, we'll only grab focus and
        // continue with callback logic if the cursor has actually moved.
        let cur = imp.cursor_pos.get();
        let is_cursor_still = (cur.x - pos.x).abs() < 1.0 && (cur.y - pos.y).abs() < 1.0;
        if is_cursor_still {
            return;
        }

        // If we don't have focus, and we want it, grab it.
        let config = imp.config.borrow().clone();
        if let Some(config) = config
            && !imp.gl_area.has_focus()
            && config.get().focus_follows_mouse
        {
            imp.gl_area.grab_focus();
        }

        // Our pos changed, update
        imp.cursor_pos.set(pos);

        // Notify the callback
        if let Some(surface) = this.core() {
            let mods = gtk_key::translate_mods(state);
            if let Err(err) = surface.cursor_pos_callback(pos, Some(mods)) {
                log::warn!("error in cursor pos callback err={err}");
            }
        }
    }

    fn scroll_vertical_impl(&self, x: f64, y: f64) -> bool {
        let this = self;
        let imp = this.imp();
        let Some(surface) = this.core() else {
            return false;
        };

        // Multiply precision scrolls by 10 to get a better response from
        // touchpad scrolling
        let precision = imp.precision_scroll.get();
        let multiplier = if precision { 10.0 } else { 1.0 };
        let scroll_mods = ghostty_input::ScrollMods {
            precision,
            ..ghostty_input::ScrollMods::default()
        };

        let (sx, sy) = this.scaled_coordinates(x, y);
        // We invert because we apply natural scrolling to the values.
        // This behavior has existed for years without Linux users
        // complaining but I suspect we'll have to make this configurable
        // in the future or read a system setting.
        match surface.scroll_callback(-sx * multiplier, -sy * multiplier, scroll_mods) {
            Ok(()) => true,
            Err(err) => {
                log::warn!("error in scroll callback err={err}");
                false
            }
        }
    }

    /// Debug input replay: pointer motion (logical widget coordinates).
    pub fn debug_mouse_move(&self, x: f64, y: f64, state: gdk::ModifierType) {
        self.mouse_motion_impl(state, x, y);
    }

    /// Debug input replay: button press/release (1 left, 2 middle, 3 right).
    pub fn debug_mouse_button(
        &self,
        press: bool,
        button: u32,
        x: f64,
        y: f64,
        state: gdk::ModifierType,
    ) {
        if press {
            // The replayed window is usually not the compositor-focused one, so
            // pretend the click did not need to transition focus.
            self.mouse_down_impl(button, state, x, y, true);
        } else {
            self.mouse_up_impl(button, state, None);
        }
    }

    /// Horizontal (surface unit) scroll: accumulate and switch tabs once the
    /// threshold is reached (`ecMouseScrollHorizontal` after its unit check).
    fn scroll_horizontal_impl(&self, x: f64) -> bool {
        let this = self;
        let imp = this.imp();

        imp.pending_horizontal_scroll
            .set(imp.pending_horizontal_scroll.get() + x);

        if imp.pending_horizontal_scroll.get().abs() < 120.0 {
            if let Some(v) = imp.pending_horizontal_scroll_reset.take() {
                v.remove();
            }
            let weak = this.downgrade();
            let id = glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
                if let Some(this) = weak.upgrade() {
                    let imp = this.imp();
                    imp.pending_horizontal_scroll.set(0.0);
                    let _ = imp.pending_horizontal_scroll_reset.take();
                }
                glib::ControlFlow::Break
            });
            imp.pending_horizontal_scroll_reset.set(Some(id));
            return true;
        }

        let _ = this.activate_action(
            if imp.pending_horizontal_scroll.get() < 0.0 {
                "tab.next-page"
            } else {
                "tab.previous-page"
            },
            None,
        );

        if let Some(v) = imp.pending_horizontal_scroll_reset.take() {
            v.remove();
        }

        imp.pending_horizontal_scroll.set(0.0);

        true
    }

    /// Debug input replay: horizontal scroll in surface units (touchpad).
    pub fn debug_hscroll(&self, dx: f64) {
        self.scroll_horizontal_impl(dx);
    }

    /// Debug input replay: wheel scroll (vertical goes through the same
    /// handler as the scroll controller; see `debug_hscroll` for horizontal).
    pub fn debug_scroll(&self, dy: f64) {
        self.scroll_vertical_impl(0.0, dy);
    }

    /// Debug input replay: text of the GDK clipboard and primary selection.
    pub fn debug_clipboard_dump(&self) {
        log::warn!(
            "debug input: context menu visible = {}",
            self.imp().context_menu.is_visible()
        );
        let display = self.display();
        for (name, cb) in [
            ("clipboard", display.clipboard()),
            ("primary", display.primary_clipboard()),
        ] {
            cb.read_text_async(gtk::gio::Cancellable::NONE, move |r| match r {
                Ok(t) => log::warn!("debug input: {name} = {:?}", t.map(|s| s.to_string())),
                Err(e) => log::warn!("debug input: {name} read failed: {e}"),
            });
        }
    }

    /// Debug input replay: commit text as the input method would (the
    /// `commit` signal of the IM context), outside of any key event.
    pub fn debug_commit_text(&self, text: &str) {
        self.imp().im_context.emit_by_name::<()>("commit", &[&text]);
    }

    /// Debug input replay: drive the surface's real input method signal
    /// handlers (the functions the template connects to the IM context's
    /// `preedit-start`/`preedit-changed`/`preedit-end`/`commit` signals) the
    /// way an input method would. `ctx` is the context passed as the signal
    /// emitter (so `preedit_string()` reports a real preedit and attributes).
    pub fn debug_im(&self, ctx: &gtk::IMContext, step: DebugIm) {
        use imp::Surface as H;
        match step {
            DebugIm::PreeditStart => H::im_preedit_start(ctx, self),
            DebugIm::PreeditChanged => H::im_preedit_changed(ctx, self),
            DebugIm::PreeditEnd => H::im_preedit_end(ctx, self),
            DebugIm::Commit(text) => H::im_commit(ctx, &text, self),
        }
    }

    /// Debug input replay: whether the surface believes it is composing.
    pub fn debug_im_composing(&self) -> bool {
        self.imp().im_composing.get()
    }

    /// Debug input replay: a key press+release that goes through
    /// [`Surface::im_filter`] (the real `in_keyevent`/`im_composing`
    /// bookkeeping) with a simulated input method. `ctx` carries the preedit
    /// string. Returns whether the press was consumed by the input method.
    pub fn debug_im_key(
        &self,
        ctx: &gtk::IMContext,
        sim: &DebugImKey,
        keyval: u32,
        keycode: u32,
        state: gdk::ModifierType,
    ) -> bool {
        let consumed = self.im_filter(|| {
            use imp::Surface as H;
            match sim {
                DebugImKey::Preedit => {
                    // A dead key / compose start: preedit-start (once) then
                    // preedit-changed, event handled.
                    if !self.imp().im_composing.get() {
                        H::im_preedit_start(ctx, self);
                    }
                    H::im_preedit_changed(ctx, self);
                    true
                }
                DebugImKey::Commit(text) => {
                    // ibus order: commit THEN preedit-end.
                    H::im_commit(ctx, text, self);
                    H::im_preedit_end(ctx, self);
                    true
                }
                DebugImKey::CommitSimple(text) => {
                    // gtk-im-simple order: preedit-end THEN commit.
                    H::im_preedit_end(ctx, self);
                    H::im_commit(ctx, text, self);
                    true
                }
                DebugImKey::HandledCommit(text) => {
                    // Plain character translation: commit, handled.
                    H::im_commit(ctx, text, self);
                    true
                }
                DebugImKey::Pass => false,
            }
        });
        if !consumed {
            self.debug_key_event(KeyAction::Press, keyval, keycode, state);
        }
        // The release also goes through the input method.
        if !self.im_filter(|| false) {
            self.debug_key_event(KeyAction::Release, keyval, keycode, state);
        }
        consumed
    }

    fn key_event_encode(
        &self,
        action: KeyAction,
        src: &KeySource<'_>,
        keyval: u32,
        keycode: u32,
        gtk_mods: gdk::ModifierType,
    ) -> bool {
        let imp = self.imp();

        // Get the keyvals for this event.
        let keyval_unicode: Option<char> = match src {
            KeySource::Event { key_event, .. } => key_event.keyval().to_unicode(),
            KeySource::Synthetic { translated, .. } => translated.to_unicode(),
        };
        let keyval_unicode_unshifted = match src {
            KeySource::Event { key_event, .. } => {
                gtk_key::keyval_unicode_unshifted(&*imp.gl_area, key_event, keycode)
            }
            KeySource::Synthetic { layout, .. } => {
                gtk_key::keyval_unicode_unshifted_for_layout(&*imp.gl_area, *layout, keycode)
            }
        };

        // We want to get the physical unmapped key to process physical
        // keybinds. (These are keybinds explicitly marked as requesting
        // physical mapping).
        let physical_key: Key = {
            let w3c_key = ghostty_input::keycodes::entries()
                .iter()
                .find(|e| e.native == keycode)
                .map_or(Key::Unidentified, |e| e.key);

            // Consult the pre-remapped XKB keyval/keysym to get the
            // (possibly) remapped key. If the W3C key or the remapped key is
            // eligible for remapping, we use it.
            //
            // See the docs for `should_be_remappable` for why we even have
            // to do this in the first place.
            match gtk_key::key_from_keyval(keyval) {
                Some(remapped)
                    if w3c_key.should_be_remappable() || remapped.should_be_remappable() =>
                {
                    remapped
                }
                // Return the original physical key
                _ => w3c_key,
            }
        };

        // Get our modifier for the event
        let application = app();
        let device = match src {
            KeySource::Event { event, .. } => event.device(),
            KeySource::Synthetic { .. } => imp
                .gl_area
                .display()
                .default_seat()
                .and_then(|s| s.keyboard()),
        };
        let mods: Mods = gtk_key::event_mods_for_device(
            device.as_ref(),
            physical_key,
            gtk_mods,
            action,
            |dev, m| application.winproto().event_mods(dev, m),
        );

        // Get our consumed modifiers
        let consumed = match src {
            KeySource::Event { key_event, .. } => key_event.consumed_modifiers(),
            KeySource::Synthetic { consumed, .. } => *consumed,
        };
        let consumed_mods: Mods = gtk_key::translate_mods(consumed & gdk::MODIFIER_MASK);

        // If we have no UTF-8 text, we try to convert our keyval to
        // a text value. We have to do this because GTK will not process
        // "Ctrl+Shift+1" (on US keyboards) as "Ctrl+!" but instead as "".
        // But the keyval is set correctly so we can at least extract that.
        if imp.im_buf.borrow().is_empty()
            && let Some(ch) = keyval_unicode
        {
            // We don't want to send control characters as IM text.
            // Control characters are handled already by the encoder
            // directly.
            if (ch as u32) >= 0x20 {
                imp.im_buf.borrow_mut().push(ch);
            }
        }

        // Invoke the core Ghostty logic to handle this input.
        let Some(surface) = self.core() else {
            return false;
        };
        let utf8 = imp.im_buf.borrow().clone();
        let effect = match surface.key_callback(KeyEvent {
            action,
            key: physical_key,
            mods,
            consumed_mods,
            composing: imp.im_composing.get(),
            utf8: utf8.as_bytes(),
            unshifted_codepoint: keyval_unicode_unshifted,
        }) {
            Ok(e) => e,
            Err(err) => {
                log::error!("error in key callback err={err}");
                return false;
            }
        };

        match effect {
            InputEffect::Closed => return true,
            InputEffect::Ignored => {}
            InputEffect::Consumed => {
                if action == KeyAction::Press || action == KeyAction::Repeat {
                    // If we were in the composing state then we reset our
                    // context. We do NOT want to reset if we're not in the
                    // composing state because there is other IME state that
                    // we want to preserve, such as quotation mark ordering
                    // for Chinese input.
                    if imp.im_composing.get() {
                        imp.im_context.reset();
                        let _ = surface.preedit_callback(None);
                    }

                    // Bell stops ringing when any key is pressed that is
                    // used by the core in any way.
                    self.set_bell_ringing(false);

                    return true;
                }
            }
        }

        false
    }

    /// Prompt for a manual title change for the surface.
    pub fn prompt_title(&self) {
        let imp = self.imp();
        let initial = imp
            .title_override
            .borrow()
            .clone()
            .or_else(|| imp.title.borrow().clone());
        let dialog = TitleDialog::new(title_dialog::Target::Surface, initial.as_deref());
        let weak = self.downgrade();
        dialog.connect_set(move |_, title| {
            if let Some(this) = weak.upgrade() {
                this.set_title_override(if title.is_empty() { None } else { Some(title) });
            }
        });
        dialog.present(self);
    }

    /// Scale x/y by the GDK device scale.
    fn scaled_coordinates(&self, x: f64, y: f64) -> (f64, f64) {
        let scale_factor = f64::from(self.imp().gl_area.scale_factor());
        (x * scale_factor, y * scale_factor)
    }

    // -----------------------------------------------------------------
    // Libghostty callbacks

    /// Emit `close-request` (Zig: `close`). Does not confirm.
    pub fn close(&self) {
        self.emit_by_name::<()>("close-request", &[]);
    }

    pub fn child_exited(&self, data: &ChildExited) -> bool {
        // Even if we don't support the overlay, we still keep our property
        // up to date for anyone listening.
        self.imp().child_exited.set(true);
        self.notify("child-exited");

        // If we have the noop child exited overlay then we don't do anything
        // for child exited. The false return will force libghostty to show
        // the normal text-based message.
        if SurfaceChildExited::NOOP {
            return false;
        }

        self.imp().child_exited_overlay.set_child_exited(Some(data));
        true
    }

    pub fn get_content_scale(&self) -> ContentScale {
        let gtk_scale: f32 = {
            // Future: detect GTK version 4.12+ and use gdk_surface_get_scale
            // so we can support fractional scaling.
            let scale = self.imp().gl_area.scale_factor();
            if scale <= 0 {
                log::warn!("gtk_widget_get_scale_factor returned a non-positive number: {scale}");
                1.0
            } else {
                scale as f32
            }
        };

        // Also scale using font-specific DPI, which is often exposed to the
        // user via DE accessibility settings (see
        // https://docs.gtk.org/gtk4/class.Settings.html).
        let xft_dpi_scale: f32 = 'xft: {
            // gtk-xft-dpi is font DPI multiplied by 1024.
            let Some(gtk_xft_dpi) = crate::gsettings::get_int(crate::gsettings::Key::GtkXftDpi)
            else {
                log::warn!("gtk-xft-dpi was not set, using default value");
                break 'xft 1.0;
            };

            // Use a value of 1.0 for the XFT DPI scale if the setting is <= 0
            if gtk_xft_dpi <= 0 {
                log::warn!("gtk-xft-dpi has invalid value ({gtk_xft_dpi}), using default");
                break 'xft 1.0;
            }

            // As noted above gtk-xft-dpi is multiplied by 1024, so we divide
            // by 1024, then divide by the default value (96) to derive a
            // scale. Note gtk-xft-dpi can be fractional, so we use floating
            // point math here.
            let xft_dpi = gtk_xft_dpi as f32 / 1024.0;
            xft_dpi / 96.0
        };

        let scale = gtk_scale * xft_dpi_scale;
        ContentScale { x: scale, y: scale }
    }

    pub fn get_size(&self) -> SurfaceSize {
        // By the time this is called, we should be in a widget tree. This
        // should not be called before that. We ensure this by initializing
        // the surface in `gl_resize`. This is VERY important because it
        // avoids the pty having an incorrect initial size.
        self.imp().size.get()
    }

    pub fn get_cursor_pos(&self) -> CursorPos {
        self.imp().cursor_pos.get()
    }

    pub fn default_termio_env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        let application = app();
        // Byte-preserving environment (Zig `internal_os.getEnvMap`).
        let mut env = ghostty_os::env::get_env_map();

        restore_language(&mut env, application.saved_language().as_deref());

        // Don't leak these GTK environment variables to child processes.
        env.remove("GDK_DEBUG");
        env.remove("GDK_DISABLE");
        env.remove("GSK_RENDERER");

        // Remove some environment variables that are set when Ghostty is
        // launched from a `.desktop` file, by D-Bus activation, or systemd.
        env.remove("GIO_LAUNCHED_DESKTOP_FILE");
        env.remove("GIO_LAUNCHED_DESKTOP_FILE_PID");
        env.remove("DBUS_STARTER_ADDRESS");
        env.remove("DBUS_STARTER_BUS_TYPE");
        env.remove("INVOCATION_ID");
        env.remove("JOURNAL_STREAM");
        env.remove("NOTIFY_SOCKET");

        // Unset environment varies set by snaps if we're running in a snap.
        // This allows Ghostty to further launch additional snaps. (Zig gates
        // this on the `snap` build option; running inside a snap implies it.)
        if env.get("SNAP").is_some() {
            ghostty_os::filter_snap_paths(&mut env);
        }

        // This is a hack because it ties ourselves (optionally) to the
        // Window class. The right solution we should do is emit a signal
        // here where the handler can modify our env, but boxing the
        // env is a bit annoying so I'm punting it.
        if let Some(window) = self
            .ancestor(Window::static_type())
            .and_then(|w| w.downcast::<Window>().ok())
        {
            // winproto only ever adds (UTF-8) variables.
            let mut extra: Vec<(String, String)> = Vec::new();
            if let Err(e) = window.winproto().add_subprocess_env(&mut extra) {
                log::warn!("winproto add_subprocess_env failed: {e}");
            }
            for (k, v) in extra {
                env.insert(k, v);
            }
            if window.is_quick_terminal() {
                env.insert("GHOSTTY_QUICK_TERMINAL", "1");
            }
        }

        env.into_iter().collect()
    }

    pub fn clipboard_request(&self, clipboard_type: Clipboard, state: ClipboardRequest) -> bool {
        clipboard::request(self, clipboard_type, state)
    }

    pub fn set_clipboard(
        &self,
        clipboard_type: Clipboard,
        contents: &[(String, Vec<u8>)],
        confirm: bool,
    ) {
        clipboard::set(self, clipboard_type, contents, confirm);
    }

    /// Paste explicit text directly into the surface, regardless of the
    /// actual clipboard contents (Zig: `Clipboard.paste`).
    pub fn paste(&self, text: &str) {
        clipboard::paste(self, text);
    }

    /// [`Surface::paste`] for raw bytes (e.g. dropped file names that are
    /// not valid UTF-8 must reach the pty unchanged).
    pub fn paste_bytes(&self, bytes: &[u8]) {
        clipboard::paste_bytes(self, bytes);
    }

    /// Focus this surface. This properly focuses the input part of
    /// our surface. (Shadows `WidgetExt::grab_focus` on purpose, like Zig.)
    pub fn grab_focus(&self) {
        self.imp().gl_area.grab_focus();
    }

    pub fn send_desktop_notification(&self, title: &str, body: &str) {
        let application = app();

        if self.core().is_none() {
            log::warn!("can't send notification because there is no core surface");
            return;
        }

        let t = if title.is_empty() { "Ghostty" } else { title };

        let notification = gio::Notification::new(t);
        notification.set_body(Some(body));

        let icon = gio::ThemedIcon::new("com.mitchellh.ghostty");
        notification.set_icon(&icon);

        // The "pointer" is our surface id; the app validates it against the
        // core's surface list via a mailbox message.
        notification.set_default_action_and_target_value(
            "app.present-surface",
            Some(&self.id().to_variant()),
        );

        // We set the notification ID to the body content. If the content is
        // the same, this notification may replace a previous notification
        application
            .upcast_ref::<gio::Application>()
            .send_notification(Some(body), &notification);
    }

    // -----------------------------------------------------------------
    // Properties

    /// Returns the title property.
    pub fn get_title(&self) -> Option<String> {
        self.imp().title.borrow().clone()
    }

    /// Returns the effective title: the user-overridden title if set,
    /// otherwise the terminal-set title.
    pub fn get_effective_title(&self) -> Option<String> {
        let imp = self.imp();
        imp.title_override
            .borrow()
            .clone()
            .or_else(|| imp.title.borrow().clone())
    }

    /// Copies the effective title to the clipboard.
    pub fn copy_title_to_clipboard(&self) -> bool {
        let Some(title) = self.get_effective_title() else {
            return false;
        };
        if title.is_empty() {
            return false;
        }
        self.set_clipboard(
            Clipboard::Standard,
            &[("text/plain".to_owned(), title.into_bytes())],
            false,
        );
        true
    }

    /// Set the title for this surface, copies the value. This should always
    /// be the title as set by the terminal program, not any manually set
    /// title. For manually set titles see `set_title_override`.
    pub fn set_title(&self, title: Option<&str>) {
        self.imp().title.replace(title.map(str::to_owned));
        self.notify("title");
    }

    /// Overridden title. This will be generally be shown over the title
    /// unless this is unset (None).
    pub fn set_title_override(&self, title: Option<&str>) {
        self.imp().title_override.replace(title.map(str::to_owned));
        self.notify("title-override");
    }

    pub fn get_title_override(&self) -> Option<String> {
        self.imp().title_override.borrow().clone()
    }

    /// Returns the pwd property.
    pub fn get_pwd(&self) -> Option<ghostty_os::osbytes::EncodedOs> {
        // The typed backing retains bytes; the GObject property renders text.
        self.imp().pwd.borrow().clone()
    }

    /// Set the pwd for this surface, copies the value.
    pub fn set_pwd(&self, pwd: Option<&ghostty_os::osbytes::EncodedOs>) {
        self.imp().pwd.replace(pwd.cloned());
        self.notify("pwd");
    }

    /// Returns the focus state of this surface.
    pub fn get_focused(&self) -> bool {
        self.imp().focused.get()
    }

    /// The surface config object (Zig: `priv.config`).
    pub fn get_config(&self) -> Option<Config> {
        self.imp().config.borrow().clone()
    }

    /// Change the configuration for this surface.
    pub fn set_config(&self, config: &Config) {
        self.imp().config.replace(Some(config.clone()));
        self.notify("config");
    }

    /// Return the default size, if set.
    pub fn get_default_size(&self) -> Option<Size> {
        *self.imp().default_size.borrow()
    }

    /// Set the default size for a window that contains this surface.
    /// This is up to the embedding widget to respect this. Generally, only
    /// the first surface in a window respects this.
    pub fn set_default_size(&self, size: Size) {
        self.imp().default_size.replace(Some(size));
        self.notify("default-size");
    }

    /// Estimate and set the initial window size from config and font metrics.
    /// This can be called before the core surface exists to set up the window
    /// size before presenting. This is an estimate because it does not take
    /// into account any padding that may need to be added to the window.
    pub fn estimate_initial_size(&self) {
        let Some(config_obj) = self.imp().config.borrow().clone() else {
            return;
        };
        let config = config_obj.get();

        // Both dimensions must be configured
        if config.window_height == 0 || config.window_width == 0 {
            return;
        }

        // Get content scale and compute DPI
        let content_scale = self.get_content_scale();
        let x_dpi = content_scale.x * DEFAULT_DPI;
        let y_dpi = content_scale.y * DEFAULT_DPI;

        let font_size = DesiredFontSize {
            points: config.font_size,
            xdpi: x_dpi as u16,
            ydpi: y_dpi as u16,
        };

        // Get font grid for cell metrics
        let Some(cell) = app().core().cell_size(&config, font_size) else {
            return;
        };

        let width = MIN_WINDOW_WIDTH_CELLS.max(config.window_width) * cell.width;
        let height = MIN_WINDOW_HEIGHT_CELLS.max(config.window_height) * cell.height;

        let final_width = (width as f32 / content_scale.x).ceil() as u32;
        let final_height = (height as f32 / content_scale.y).ceil() as u32;

        self.set_default_size(Size {
            width: final_width,
            height: final_height,
        });
    }

    /// Return the min size, if set.
    pub fn get_min_size(&self) -> Option<Size> {
        *self.imp().min_size.borrow()
    }

    /// Set the min size for a window that contains this surface.
    /// This is up to the embedding widget to respect this. Generally, only
    /// the first surface in a window respects this.
    pub fn set_min_size(&self, size: Size) {
        self.imp().min_size.replace(Some(size));
        self.notify("min-size");
    }

    pub fn get_font_size_request(&self) -> Option<DesiredFontSize> {
        self.imp().font_size_request.borrow().map(|v| v.0)
    }

    pub fn get_mouse_shape(&self) -> MouseShape {
        self.imp().mouse_shape.get()
    }

    pub fn set_mouse_shape(&self, shape: MouseShape) {
        self.imp().mouse_shape.set(shape);
        self.notify("mouse-shape");
    }

    pub fn get_mouse_hidden(&self) -> bool {
        self.imp().mouse_hidden.get()
    }

    pub fn set_mouse_hidden(&self, hidden: bool) {
        self.imp().mouse_hidden.set(hidden);
        self.notify("mouse-hidden");
    }

    pub fn set_mouse_hover_url(&self, url: Option<&str>) {
        self.imp().mouse_hover_url.replace(url.map(str::to_owned));
        self.notify("mouse-hover-url");
    }

    pub fn set_bell_ringing(&self, ringing: bool) {
        self.imp().set_bell_ringing_prop(ringing);
    }

    pub fn get_bell_ringing(&self) -> bool {
        self.imp().bell_ringing.get()
    }

    pub fn set_error(&self, v: bool) {
        self.imp().error.set(v);
        self.notify("error");
    }

    pub fn get_zoom(&self) -> bool {
        self.imp().zoom.get()
    }

    pub fn get_is_split(&self) -> bool {
        self.imp().is_split.get()
    }

    pub fn get_child_exited(&self) -> bool {
        self.imp().child_exited.get()
    }

    pub fn set_search_active(&self, active: bool, needle: &str) {
        let imp = self.imp();
        imp.search_overlay.set_property("active", active);

        if !needle.is_empty() {
            imp.search_overlay.set_search_contents(needle);
        }

        if active {
            imp.search_overlay.grab_focus_entry();
        }
    }

    pub fn set_search_total(&self, total: Option<usize>) {
        self.imp().search_overlay.set_search_total(total);
    }

    pub fn set_search_selected(&self, selected: Option<usize>) {
        self.imp().search_overlay.set_search_selected(selected);
    }

    /// Handle bell features that need to happen every time a BEL is received
    /// Currently this is audio and system but this could change in the future.
    fn ring_bell(&self) {
        let imp = self.imp();

        // Emit the signal
        self.emit_by_name::<()>("bell", &[]);

        // Activate actions if they exist
        let _ = self.activate_action("tab.ring-bell", None);
        let _ = self.activate_action("win.ring-bell", None);

        let Some(config_obj) = imp.config.borrow().clone() else {
            return;
        };
        let config = config_obj.get();

        // Do our sound
        let BellFeatures { audio, .. } = config.bell_features;
        if audio {
            let Some(config_path) = config.bell_audio_path.as_ref() else {
                return;
            };
            let (path, required) = match config_path {
                ConfigPath::Optional(p) => (p, false),
                ConfigPath::Required(p) => (p, true),
            };

            let volume = config.bell_audio_volume.clamp(0.0, 1.0);

            // The config path is an encoded OS string: decode it first.
            let Ok(path) = path.to_path() else {
                log::warn!("invalid bell audio path: BadPathName");
                return;
            };
            debug_assert!(path.is_absolute());
            let media_file = gtk::MediaFile::for_filename(&path);

            // If the audio file is marked as required, we'll emit an error if
            // there was a problem playing it. Otherwise there will be silence.
            if required {
                media_file.connect_notify_local(Some("error"), media_file_error);
            }

            // Watch for the "ended" signal so that we can clean up after
            // ourselves: the closure owns a strong ref until playback ends.
            let keep = RefCell::new(Some(media_file.clone()));
            media_file.connect_notify_local(Some("ended"), move |_, _| {
                keep.borrow_mut().take();
            });

            let media_stream = media_file.upcast_ref::<gtk::MediaStream>();
            media_stream.set_volume(volume);
            media_stream.play();
        }
    }

    // -----------------------------------------------------------------
    // Gtk.Scrollable

    pub fn get_hadjustment(&self) -> Option<gtk::Adjustment> {
        self.imp().hadj.borrow().clone()
    }

    pub fn set_hadjustment(&self, adj: Option<&gtk::Adjustment>) {
        self.imp().hadj.replace(adj.cloned());
        self.notify("hadjustment");
    }

    pub fn get_vadjustment(&self) -> Option<gtk::Adjustment> {
        self.imp().vadj.borrow().clone()
    }

    pub fn set_vadjustment(&self, adj: Option<&gtk::Adjustment>) {
        self.imp().vadj.replace(adj.cloned());
        self.notify("vadjustment");
    }

    // -----------------------------------------------------------------
    // Typed signal connectors (Zig: `Surface.signals.*.connect`)

    /// Emitted whenever the bell event is received. Unlike the
    /// `bell-ringing` property, this is emitted every time the event
    /// is received and not just on state changes.
    pub fn connect_bell<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure("bell", false, glib::closure_local!(move |s: &Self| f(s)))
    }

    /// Emitted whenever the surface would like to be closed for any reason.
    ///
    /// The surface view does NOT handle its own close confirmation. If there
    /// is a process alive then the parent widget should handle this request.
    pub fn connect_close_request<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "close-request",
            false,
            glib::closure_local!(move |s: &Self| f(s)),
        )
    }

    /// Emitted whenever the clipboard has been written.
    pub fn connect_clipboard_write<F: Fn(&Self, Clipboard, &str) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "clipboard-write",
            false,
            glib::closure_local!(move |s: &Self, kind: u32, text: &str| {
                f(s, clipboard_from_u32(kind), text);
            }),
        )
    }

    /// Emitted whenever the surface reads the clipboard.
    pub fn connect_clipboard_read<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "clipboard-read",
            false,
            glib::closure_local!(move |s: &Self| f(s)),
        )
    }

    /// Emitted after the core surface is initialized.
    pub fn connect_init<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure("init", false, glib::closure_local!(move |s: &Self| f(s)))
    }

    /// Emitted just prior to the context menu appearing.
    pub fn connect_menu<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure("menu", false, glib::closure_local!(move |s: &Self| f(s)))
    }

    /// Emitted when the focus wants to be brought to the top and focused.
    pub fn connect_present_request<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "present-request",
            false,
            glib::closure_local!(move |s: &Self| f(s)),
        )
    }

    /// Emitted when this surface requests its container to toggle fullscreen.
    pub fn connect_toggle_fullscreen<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "toggle-fullscreen",
            false,
            glib::closure_local!(move |s: &Self| f(s)),
        )
    }

    /// Emitted when this surface requests its container to toggle maximize.
    pub fn connect_toggle_maximize<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "toggle-maximize",
            false,
            glib::closure_local!(move |s: &Self| f(s)),
        )
    }

    // -----------------------------------------------------------------
    // Internal

    fn prop_config(&self) {
        let imp = self.imp();
        let Some(config_obj) = imp.config.borrow().clone() else {
            return;
        };
        let config = config_obj.get();

        // resize-overlay-duration
        imp.resize_overlay
            .set_property("duration", config.resize_overlay_duration.as_milliseconds());

        // resize-overlay-position
        let (halign, valign) = match config.resize_overlay_position {
            ResizeOverlayPosition::Center => (gtk::Align::Center, gtk::Align::Center),
            ResizeOverlayPosition::TopLeft => (gtk::Align::Start, gtk::Align::Start),
            ResizeOverlayPosition::TopRight => (gtk::Align::End, gtk::Align::Start),
            ResizeOverlayPosition::TopCenter => (gtk::Align::Center, gtk::Align::Start),
            ResizeOverlayPosition::BottomLeft => (gtk::Align::Start, gtk::Align::End),
            ResizeOverlayPosition::BottomRight => (gtk::Align::End, gtk::Align::End),
            ResizeOverlayPosition::BottomCenter => (gtk::Align::Center, gtk::Align::End),
        };
        imp.resize_overlay.set_property("overlay-halign", halign);
        imp.resize_overlay.set_property("overlay-valign", valign);
    }

    fn prop_error(&self) {
        let imp = self.imp();
        if imp.error.get() {
            // Ensure we have an opaque background. The window will NOT set
            // this if we have transparency set and we need an opaque
            // background for the error message to be readable.
            self.add_css_class("background");
        } else {
            // Regardless of transparency setting, we remove the background
            // CSS class from this widget. Parent widgets will set it
            // appropriately (see window.zig for example).
            self.remove_css_class("background");
        }

        // We need to set our child property on an idle tick, because the
        // error property can be triggered by signals that are in the middle
        // of widget mapping and changing our child during that time
        // results in a hard gtk crash.
        let pending = imp.idle_rechild.take();
        if pending.is_some() {
            imp.idle_rechild.set(pending);
        } else {
            let weak = self.downgrade();
            let id = glib::idle_add_local(move || {
                if let Some(this) = weak.upgrade() {
                    let imp = this.imp();
                    let _ = imp.idle_rechild.take();
                    if imp.error.get() {
                        this.set_child(Some(&*imp.error_page));
                    } else {
                        this.set_child(Some(&*imp.terminal_page));
                    }
                }
                glib::ControlFlow::Break
            });
            imp.idle_rechild.set(Some(id));
        }
    }

    fn prop_mouse_hover_url(&self) {
        let imp = self.imp();
        let visible = imp
            .mouse_hover_url
            .borrow()
            .as_ref()
            .is_some_and(|v| !v.is_empty());
        imp.url_left.set_visible(visible);
    }

    fn prop_mouse_hidden(&self) {
        // If we're hidden we set it to "none"
        if self.imp().mouse_hidden.get() {
            self.set_cursor_from_name(Some("none"));
            return;
        }

        // If we're not hidden we just trigger the mouse shape
        // prop notification to handle setting the proper mouse shape.
        self.prop_mouse_shape();
    }

    fn prop_mouse_shape(&self) {
        let imp = self.imp();

        // If our mouse should be hidden currently then we don't do anything.
        if imp.mouse_hidden.get() {
            return;
        }

        // Set our new cursor.
        self.set_cursor_from_name(Some(mouse_shape_name(imp.mouse_shape.get())));
    }

    fn prop_vadjustment(&self) {
        let imp = self.imp();

        // Move our value-changed handler to the new adjustment (Zig uses a
        // lazily-created GSignalGroup for this).
        if let Some((old, id)) = imp.vadj_handler.take() {
            old.disconnect(id);
        }
        let Some(adj) = imp.vadj.borrow().clone() else {
            return;
        };
        let weak = self.downgrade();
        let id = adj.connect_value_changed(move |adj| {
            if let Some(this) = weak.upgrade() {
                this.vadj_value_changed(adj);
            }
        });
        imp.vadj_handler.replace(Some((adj, id)));
    }

    fn vadj_value_changed(&self, adj: &gtk::Adjustment) {
        // This will trigger for every single pixel change in the adjustment,
        // but our core surface handles the noise from this so that identical
        // rows are cheap.
        let Some(core_surface) = self.core() else {
            return;
        };
        let row = adj.value().round().max(0.0) as usize;
        if let Err(err) =
            core_surface.perform_binding_action(&ghostty_input::binding::Action::ScrollToRow(row))
        {
            log::error!("error performing scroll_to_row action err={err}");
        }
    }

    fn init_action_map(&self) {
        let group = gio::SimpleActionGroup::new();

        let prompt_title = gio::SimpleAction::new("prompt-title", None);
        let weak = self.downgrade();
        prompt_title.connect_activate(move |_, _| {
            let Some(this) = weak.upgrade() else { return };
            let Some(surface) = this.core() else { return };
            if let Err(err) =
                surface.perform_binding_action(&ghostty_input::binding::Action::PromptSurfaceTitle)
            {
                log::warn!("unable to perform prompt title action err={err}");
            }
        });
        group.add_action(&prompt_title);

        let notify_next = gio::SimpleAction::new_stateful(
            "notify-on-next-command-finish",
            None,
            &false.to_variant(),
        );
        notify_next.connect_activate(|action, _| {
            let state = action.state().unwrap_or_else(|| false.to_variant());
            let Some(value) = state.get::<bool>() else {
                return;
            };
            action.set_state(&(!value).to_variant());
        });
        group.add_action(&notify_next);

        self.insert_action_group("surface", Some(&group));
        self.imp().action_group.replace(Some(group));
    }

    fn resize_overlay_schedule(&self) {
        let imp = self.imp();
        let Some(surface) = self.core() else { return };

        // Only show the resize overlay if its enabled
        let Some(config_obj) = imp.config.borrow().clone() else {
            return;
        };
        match config_obj.get().resize_overlay {
            ResizeOverlayMode::Always | ResizeOverlayMode::AfterFirst => {}
            ResizeOverlayMode::Never => return,
        }

        // If we have resize overlays enabled, setup an idler
        // to show that. We do this in an idle tick because doing it
        // during the resize results in flickering.
        let (columns, rows) = surface.grid_size();
        imp.resize_overlay
            .set_label(Some(format!("{columns} x {rows}").as_str()));
        imp.resize_overlay.schedule();
    }

    /// The first `focused` surface other than ourselves, used to inherit the
    /// working directory like `App.focusedSurface` does in the core.
    fn focused_surface_pwd(&self) -> Option<ghostty_os::osbytes::EncodedOs> {
        Surface::all()
            .into_iter()
            .find(|s| s != self && s.core().is_some() && s.get_focused())
            .and_then(|s| s.get_pwd())
    }

    fn init_surface(&self) -> Result<(), InitError> {
        let imp = self.imp();
        debug_assert!(imp.core_surface.borrow().is_none());
        let gl_area = imp.gl_area.clone();

        // We need to make the context current so we can call GL functions.
        // This is required for all surface operations.
        gl_area.make_current();
        if let Some(err) = gl_area.error() {
            log::warn!("failed to make GL context current: {}", err.message());
            log::warn!("this error is usually due to a driver or gtk bug");
            log::warn!(
                "this is a common cause of this issue: https://gitlab.gnome.org/GNOME/gtk/-/issues/4950"
            );
            return Err(InitError::GlArea);
        }

        let application = app();
        let base: std::sync::Arc<CoreConfig> = imp
            .config
            .borrow()
            .as_ref()
            .map(Config::get)
            .ok_or(InitError::Surface)?;

        // Initialize our surface configuration.
        let mut config = ghostty_apprt::surface::new_config(
            &base,
            imp.context.get(),
            self.focused_surface_pwd(),
        );

        let overrides = imp.overrides.borrow().clone();
        if let Some(c) = overrides.command {
            config.command = Some(c);
        }
        if let Some(wd) = overrides.working_directory {
            let mut wd_val = WorkingDirectory::Path(wd);
            wd_val.finalize();
            config.working_directory = Some(wd_val);
        }

        // Properties that can impact surface init
        if let Some(size) = *imp.font_size_request.borrow() {
            config.font_size = size.0.points;
        }
        if let Some(pwd) = self.get_pwd() {
            let mut wd_val = WorkingDirectory::Path(pwd);
            wd_val.finalize();
            config.working_directory = Some(wd_val);
        }

        // Initialize the surface
        let core = application
            .core()
            .new_surface(self.id(), config, application.rt(), self.rt())
            .map_err(|err| {
                log::warn!("failed to initialize surface err={err}");
                InitError::Surface
            })?;

        // Store it!
        imp.core_surface.replace(Some(core));

        // Emit the signal that we initialized the surface.
        self.emit_by_name::<()>("init", &[]);
        Ok(())
    }
}

#[derive(Debug)]
enum InitError {
    GlArea,
    Surface,
}

fn clipboard_from_u32(v: u32) -> Clipboard {
    match v {
        0 => Clipboard::Standard,
        1 => Clipboard::Selection,
        _ => Clipboard::Primary,
    }
}

fn media_file_error(media_file: &gtk::MediaFile, _: &glib::ParamSpec) {
    let path = media_file
        .file()
        .and_then(|f| f.path())
        .map_or_else(|| "<<unknown>>".to_owned(), |p| p.display().to_string());

    let media_stream = media_file.upcast_ref::<gtk::MediaStream>();
    let Some(err) = media_stream.error() else {
        return;
    };
    log::warn!(
        "error playing bell from {}: {:?} {} {}",
        path,
        err.domain(),
        err.code(),
        err.message()
    );
}

/// Translate a GTK mouse button number to the input mouse button.
fn translate_mouse_button(button: u32) -> MouseButton {
    match button {
        1 => MouseButton::Left,
        2 => MouseButton::Middle,
        3 => MouseButton::Right,
        4 => MouseButton::Four,
        5 => MouseButton::Five,
        6 => MouseButton::Six,
        7 => MouseButton::Seven,
        8 => MouseButton::Eight,
        9 => MouseButton::Nine,
        10 => MouseButton::Ten,
        11 => MouseButton::Eleven,
        _ => MouseButton::Unknown,
    }
}

/// CSS cursor name for the given mouse shape.
fn mouse_shape_name(shape: MouseShape) -> &'static str {
    match shape {
        MouseShape::Default => "default",
        MouseShape::Help => "help",
        MouseShape::Pointer => "pointer",
        MouseShape::ContextMenu => "context-menu",
        MouseShape::Progress => "progress",
        MouseShape::Wait => "wait",
        MouseShape::Cell => "cell",
        MouseShape::Crosshair => "crosshair",
        MouseShape::Text => "text",
        MouseShape::VerticalText => "vertical-text",
        MouseShape::Alias => "alias",
        MouseShape::Copy => "copy",
        MouseShape::NoDrop => "no-drop",
        MouseShape::Move => "move",
        MouseShape::NotAllowed => "not-allowed",
        MouseShape::Grab => "grab",
        MouseShape::Grabbing => "grabbing",
        MouseShape::AllScroll => "all-scroll",
        MouseShape::ColResize => "col-resize",
        MouseShape::RowResize => "row-resize",
        MouseShape::NResize => "n-resize",
        MouseShape::EResize => "e-resize",
        MouseShape::SResize => "s-resize",
        MouseShape::WResize => "w-resize",
        MouseShape::NeResize => "ne-resize",
        MouseShape::NwResize => "nw-resize",
        MouseShape::SeResize => "se-resize",
        MouseShape::SwResize => "sw-resize",
        MouseShape::EwResize => "ew-resize",
        MouseShape::NsResize => "ns-resize",
        MouseShape::NeswResize => "nesw-resize",
        MouseShape::NwseResize => "nwse-resize",
        MouseShape::ZoomIn => "zoom-in",
        MouseShape::ZoomOut => "zoom-out",
    }
}

fn mouse_shape_from_i32(v: i32) -> MouseShape {
    // Keep in sync with the `MouseShape` discriminants (`repr(i32)`).
    const ALL: [MouseShape; 34] = [
        MouseShape::Default,
        MouseShape::ContextMenu,
        MouseShape::Help,
        MouseShape::Pointer,
        MouseShape::Progress,
        MouseShape::Wait,
        MouseShape::Cell,
        MouseShape::Crosshair,
        MouseShape::Text,
        MouseShape::VerticalText,
        MouseShape::Alias,
        MouseShape::Copy,
        MouseShape::Move,
        MouseShape::NoDrop,
        MouseShape::NotAllowed,
        MouseShape::Grab,
        MouseShape::Grabbing,
        MouseShape::AllScroll,
        MouseShape::ColResize,
        MouseShape::RowResize,
        MouseShape::NResize,
        MouseShape::EResize,
        MouseShape::SResize,
        MouseShape::WResize,
        MouseShape::NeResize,
        MouseShape::NwResize,
        MouseShape::SeResize,
        MouseShape::SwResize,
        MouseShape::EwResize,
        MouseShape::NsResize,
        MouseShape::NeswResize,
        MouseShape::NwseResize,
        MouseShape::ZoomIn,
        MouseShape::ZoomOut,
    ];
    usize::try_from(v)
        .ok()
        .and_then(|i| ALL.get(i).copied())
        .unwrap_or_default()
}

/// Compute a fraction [0.0, 1.0] from the supplied progress, which is clamped
/// to [0, 100].
fn compute_fraction(progress: u8) -> f64 {
    f64::from(progress.min(100)) / 100.0
}

/// Where the data for the key encoding path comes from: a real GDK key event
/// or a synthesized one (debug input replay).
enum KeySource<'a> {
    Event {
        event: &'a gdk::Event,
        key_event: &'a gdk::KeyEvent,
    },
    Synthetic {
        translated: gdk::Key,
        layout: u32,
        consumed: gdk::ModifierType,
    },
}

// ---------------------------------------------------------------------
// Apprt Surface (Surface.zig)

/// The object handed to the core as its runtime surface. Holds only a weak
/// reference to the widget.
pub struct RtSurfaceHandle {
    surface: glib::WeakRef<Surface>,
}

impl RtSurfaceHandle {
    /// The GObject surface for this apprt surface.
    pub fn gobj(&self) -> Option<Surface> {
        self.surface.upgrade()
    }
}

impl RtSurface for RtSurfaceHandle {
    fn close(&self, _process_active: bool) {
        if let Some(s) = self.gobj() {
            s.close();
        }
    }

    fn cgroup(&self) -> Option<String> {
        // Zig's `Surface.cgroup()` forwards to a method that doesn't exist
        // (never instantiated); there is no per-surface cgroup path.
        None
    }

    fn get_title(&self) -> Option<ghostty_os::osbytes::EncodedOs> {
        self.gobj()
            .and_then(|s| s.get_title())
            .map(ghostty_os::osbytes::EncodedOs::from)
    }

    fn get_content_scale(&self) -> ContentScale {
        self.gobj()
            .map_or(ContentScale { x: 1.0, y: 1.0 }, |s| s.get_content_scale())
    }

    fn get_size(&self) -> SurfaceSize {
        self.gobj().map_or(
            SurfaceSize {
                width: 0,
                height: 0,
            },
            |s| s.get_size(),
        )
    }

    fn get_cursor_pos(&self) -> CursorPos {
        self.gobj()
            .map_or(CursorPos { x: 0.0, y: 0.0 }, |s| s.get_cursor_pos())
    }

    fn supports_clipboard(&self, clipboard: Clipboard) -> bool {
        match clipboard {
            Clipboard::Standard | Clipboard::Selection | Clipboard::Primary => true,
        }
    }

    fn clipboard_request(&self, clipboard: Clipboard, state: ClipboardRequest) -> bool {
        self.gobj()
            .is_some_and(|s| s.clipboard_request(clipboard, state))
    }

    fn set_clipboard(&self, clipboard: Clipboard, contents: &[(String, Vec<u8>)], confirm: bool) {
        if let Some(s) = self.gobj() {
            s.set_clipboard(clipboard, contents, confirm);
        }
    }

    fn default_termio_env(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        self.gobj()
            .map(|s| s.default_termio_env())
            .unwrap_or_default()
    }

    fn gl_proc_address(&self, name: &str) -> *const std::ffi::c_void {
        crate::gl::proc_address(name)
    }

    fn create_scope_fn(&self) -> Option<ghostty_apprt::runtime_api::CreateScopeFn> {
        // GDBusConnection is thread-safe; the closure runs on the IO thread
        // right after fork.
        let dbus = gtk::gio::Application::default().and_then(|a| a.dbus_connection())?;
        Some(std::sync::Arc::new(move |pid, memory_high, tasks_max| {
            Some(
                crate::cgroup::create_scope(
                    &dbus,
                    pid,
                    crate::cgroup::Options {
                        memory_high,
                        tasks_max,
                    },
                )
                .map_err(|e| e.to_string()),
            )
        }))
    }

    fn redraw_inspector(&self) {
        if let Some(s) = self.gobj() {
            s.redraw_inspector();
        }
    }
}

// ---------------------------------------------------------------------
// Clipboard

/// A namespace for our clipboard-related functions so Surface isn't SO large.
mod clipboard {
    use adw::{prelude::*, subclass::prelude::ObjectSubclassIsExt};

    use super::{
        Clipboard,
        ClipboardConfirmationDialog,
        ClipboardError,
        ClipboardRequest,
        Surface,
        gdk,
        gio,
        glib,
    };

    /// Set the clipboard contents.
    pub(super) fn set(
        surface: &Surface,
        clipboard_type: Clipboard,
        contents: &[(String, Vec<u8>)],
        confirm: bool,
    ) {
        // Grab our plaintext content for use in confirmation dialogs
        // and signals (display text only; the clipboard gets the raw bytes).
        // We always expect one to exist.
        let Some(text) = contents
            .iter()
            .find(|(mime, _)| mime == "text/plain")
            .map(|(_, data)| String::from_utf8_lossy(data).into_owned())
        else {
            return;
        };

        // If no confirmation is necessary, set the clipboard.
        if !confirm {
            let clipboard = get(surface.imp().gl_area.upcast_ref(), clipboard_type);

            let providers: Vec<gdk::ContentProvider> = contents
                .iter()
                .map(|(mime, data)| {
                    let bytes = glib::Bytes::from(data.as_slice());
                    if mime == "text/plain" {
                        // Add an explicit UTF-8 encoding parameter to the
                        // text/plain type. The default charset when there is
                        // none is ASCII, and lots of things look for UTF-8
                        // specifically. The specs are not clear about the
                        // order here, but some clients apparently pick the
                        // first match in the order we set here then garble up
                        // bare 'text/plain' with non-ASCII UTF-8 content, so
                        // offer UTF-8 first.
                        //
                        // Note that under X11, GTK automatically adds the
                        // UTF8_STRING atom when this is present.
                        let text_providers = [
                            gdk::ContentProvider::for_bytes("text/plain;charset=utf-8", &bytes),
                            gdk::ContentProvider::for_bytes("text/plain", &bytes),
                        ];
                        gdk::ContentProvider::new_union(&text_providers)
                    } else {
                        gdk::ContentProvider::for_bytes(mime, &bytes)
                    }
                })
                .collect();

            let all = gdk::ContentProvider::new_union(&providers);
            if let Err(err) = clipboard.set_content(Some(&all)) {
                log::warn!("failed to set clipboard content: {err}");
            }

            surface.emit_by_name::<()>(
                "clipboard-write",
                &[&clipboard_to_u32(clipboard_type), &text],
            );

            return;
        }

        show_clipboard_confirmation(
            surface,
            ClipboardRequest::Osc52Write(clipboard_type),
            &text,
            false,
        );
    }

    fn clipboard_to_u32(c: Clipboard) -> u32 {
        match c {
            Clipboard::Standard => 0,
            Clipboard::Selection => 1,
            Clipboard::Primary => 2,
        }
    }

    /// Request data from the clipboard (read the clipboard). This
    /// completes asynchronously and will call the `complete_clipboard_request`
    /// core surface API when done.
    ///
    /// Returns true if the request was started, false if the clipboard
    /// doesn't contain text (allowing performable keybinds to pass through).
    pub(super) fn request(
        surface: &Surface,
        clipboard_type: Clipboard,
        state: ClipboardRequest,
    ) -> bool {
        // Get our requested clipboard
        let clipboard = get(surface.imp().gl_area.upcast_ref(), clipboard_type);

        // For paste requests, check if clipboard has text format available.
        // This is a synchronous check that allows performable keybinds to
        // pass through when the clipboard contains non-text content (e.g.,
        // images).
        if state == ClipboardRequest::Paste
            && !clipboard.formats().contains_type(glib::Type::STRING)
        {
            log::debug!("clipboard has no text format, not starting paste request");
            return false;
        }

        // Important: we hold a strong ref to the surface in the callback so
        // that we can't free memory while we have an outstanding clipboard
        // read.
        let this = surface.clone();
        clipboard.read_text_async(None::<&gio::Cancellable>, move |res| {
            let text = match res {
                Ok(Some(t)) => t,
                Ok(None) => return,
                Err(err) => {
                    log::warn!("failed to read clipboard err={}", err.message());
                    return;
                }
            };

            let Some(core) = this.core() else { return };
            match core.complete_clipboard_request(state, text.as_bytes(), false) {
                Ok(()) => {}
                Err(ClipboardError::UnsafePaste | ClipboardError::UnauthorizedPaste) => {
                    show_clipboard_confirmation(&this, state, &text, false);
                    return;
                }
                Err(err) => {
                    log::warn!("failed to complete clipboard request err={err}");
                    return;
                }
            }

            this.emit_by_name::<()>("clipboard-read", &[]);
        });

        true
    }

    /// Paste explicit text directly into the surface, regardless of the
    /// actual clipboard contents.
    pub(super) fn paste(surface: &Surface, text: &str) {
        if text.is_empty() {
            return;
        }

        let Some(core) = surface.core() else { return };
        match core.complete_clipboard_request(ClipboardRequest::Paste, text.as_bytes(), false) {
            Ok(()) => {}
            Err(ClipboardError::UnsafePaste | ClipboardError::UnauthorizedPaste) => {
                show_clipboard_confirmation(surface, ClipboardRequest::Paste, text, false);
            }
            Err(err) => log::warn!("failed to complete clipboard request err={err}"),
        }
    }

    /// Paste explicit bytes. Valid UTF-8 takes the normal text path; other
    /// bytes use the byte-preserving paste (the confirmation dialog, which
    /// is text based, carries them with the reversible byte escape).
    pub(super) fn paste_bytes(surface: &Surface, bytes: &[u8]) {
        if let Ok(text) = std::str::from_utf8(bytes) {
            paste(surface, text);
            return;
        }

        let Some(core) = surface.core() else { return };
        match core.complete_paste_bytes(bytes, false) {
            Ok(()) => {}
            Err(ClipboardError::UnsafePaste | ClipboardError::UnauthorizedPaste) => {
                let text = ghostty_os::osbytes::bytes_to_config_string(bytes);
                show_clipboard_confirmation(surface, ClipboardRequest::Paste, &text, true);
            }
            Err(err) => log::warn!("failed to complete clipboard request err={err}"),
        }
    }

    /// Get the specific type of clipboard for a widget.
    fn get(widget: &gtk::Widget, clipboard: Clipboard) -> gdk::Clipboard {
        match clipboard {
            Clipboard::Standard => widget.clipboard(),
            Clipboard::Selection | Clipboard::Primary => widget.primary_clipboard(),
        }
    }

    /// Bytes to paste once a paste confirmation was accepted. Only text that
    /// was byte-escaped for the dialog (`encoded`, from `paste_bytes`) is
    /// decoded; ordinary text is pasted verbatim, even if it happens to
    /// contain the escape character.
    pub(super) fn confirmed_paste_bytes(text: &str, encoded: bool) -> Vec<u8> {
        if encoded {
            ghostty_os::osbytes::config_str_to_bytes(text)
        } else {
            text.as_bytes().to_vec()
        }
    }

    fn show_clipboard_confirmation(
        surface: &Surface,
        req: ClipboardRequest,
        text: &str,
        encoded: bool,
    ) {
        // Build a text buffer for our contents
        let contents_buf = gtk::TextBuffer::new(None);
        contents_buf.insert_at_cursor(text);

        // Confirm
        let dialog = ClipboardConfirmationDialog::new();
        dialog.set_request_value(req);
        dialog.set_can_remember(match req {
            ClipboardRequest::Osc52Read(_) | ClipboardRequest::Osc52Write(_) => true,
            ClipboardRequest::Paste => false,
        });
        dialog.set_clipboard_contents(Some(contents_buf.clone()));

        let weak = surface.downgrade();
        let buf = contents_buf.clone();
        dialog.connect_confirm(move |_, remember| {
            let Some(this) = weak.upgrade() else { return };
            let Some(core) = this.core() else { return };

            // Handle remember
            if remember {
                match req {
                    ClipboardRequest::Osc52Read(_) => core.set_clipboard_read_allowed(true),
                    ClipboardRequest::Osc52Write(_) => core.set_clipboard_write_allowed(true),
                    ClipboardRequest::Paste => {}
                }
            }

            // Get our text
            let text = buf.text(&buf.start_iter(), &buf.end_iter(), false);
            let result = if matches!(req, ClipboardRequest::Paste) {
                // Undo the byte escape only for non-UTF-8 pastes.
                core.complete_paste_bytes(&confirmed_paste_bytes(&text, encoded), true)
            } else {
                core.complete_clipboard_request(req, text.as_bytes(), true)
            };
            if let Err(err) = result {
                log::warn!("failed to complete clipboard request: {err}");
            }
        });

        let weak = surface.downgrade();
        dialog.connect_deny(move |_, remember| {
            let Some(this) = weak.upgrade() else { return };
            let Some(core) = this.core() else { return };

            // Handle remember
            if remember {
                match req {
                    ClipboardRequest::Osc52Read(_) => core.set_clipboard_read_allowed(false),
                    ClipboardRequest::Osc52Write(_) => core.set_clipboard_write_allowed(false),
                    ClipboardRequest::Paste => {
                        unreachable!("paste should not be able to be remembered")
                    }
                }
            }
        });

        dialog.present(Some(surface));
    }
}

// ---------------------------------------------------------------------
// GObject implementation

mod imp {
    use super::*;

    #[derive(gtk::CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::SurfaceProps)]
    #[allow(clippy::struct_field_names)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.2/surface.ui")]
    pub struct Surface {
        /// This property is set to true when the bell is ringing. Note that
        /// this property will only emit a changed signal when there is a
        /// full state change. If a bell is ringing and another bell event
        /// comes through, the change notification will NOT be emitted.
        ///
        /// If you need to know every scenario the bell is triggered,
        /// listen to the `bell` signal instead.
        #[property(get, set = Self::set_bell_ringing_prop, explicit_notify, name = "bell-ringing")]
        pub bell_ringing: Cell<bool>,

        /// The configuration that this surface is using.
        #[property(get, set, nullable)]
        pub config: RefCell<Option<Config>>,

        /// True when the child has exited.
        #[property(get, set, name = "child-exited")]
        pub child_exited: Cell<bool>,

        /// The default size for a window that embeds this surface.
        #[property(get, set, nullable, name = "default-size")]
        pub default_size: RefCell<Option<Size>>,

        /// True if this surface is in an error state. This is currently
        /// a simple boolean with no additional information on WHAT the
        /// error state is, because we don't yet need it or use it. For now,
        /// if this is true, then it means the terminal is non-functional.
        #[property(get, set, name = "error")]
        pub error: Cell<bool>,

        /// The requested font size. This only applies to initialization
        /// and has no effect later.
        #[property(get, set, nullable, name = "font-size-request")]
        pub font_size_request: RefCell<Option<FontSizeRequest>>,

        /// The current focus state of the terminal based on the
        /// focus events.
        #[property(get, set)]
        pub focused: Cell<bool>,

        /// The minimum size for this surface. Embedders enforce this,
        /// not the surface itself.
        #[property(get, set, nullable, name = "min-size")]
        pub min_size: RefCell<Option<Size>>,

        /// Whether the mouse should be hidden or not as requested
        /// externally.
        #[property(get, set, name = "mouse-hidden")]
        pub mouse_hidden: Cell<bool>,

        /// The mouse shape to show for the surface. Exposed as the
        /// `MouseShape` discriminant (Zig registers a GEnum).
        #[property(
            get = Self::mouse_shape_prop,
            set = Self::set_mouse_shape_prop,
            type = i32,
            name = "mouse-shape",
            minimum = 0,
            maximum = 33,
            default = 8
        )]
        pub mouse_shape: Cell<MouseShape>,

        /// The URL that the mouse is currently hovering over.
        #[property(get, set, nullable, name = "mouse-hover-url")]
        pub mouse_hover_url: RefCell<Option<String>>,

        /// The current working directory. This has to be reported
        /// externally, usually by shell integration which then talks to
        /// libghostty which triggers this property.
        ///
        /// If this is set prior to initialization then the surface will
        /// start in this pwd. If it is set after, it has no impact on the
        /// core surface.
        #[property(get = Self::pwd_text, set = Self::set_pwd_text, type = Option<String>, nullable)]
        pub pwd: RefCell<Option<ghostty_os::osbytes::EncodedOs>>,

        /// The title of this surface, if any has been set.
        #[property(get, set, nullable)]
        pub title: RefCell<Option<String>>,

        /// The manually overridden title of this surface from
        /// `prompt_title`.
        #[property(get, set, nullable, name = "title-override")]
        pub title_override: RefCell<Option<String>>,

        /// Whether this surface is "zoomed" or not. A zoomed surface
        /// shows up taking the full bounds of a split view.
        #[property(get, set)]
        pub zoom: Cell<bool>,

        /// True if the current surface is a split, this is used to apply
        /// unfocused-split-* options.
        #[property(get, set, name = "is-split")]
        pub is_split: Cell<bool>,

        // Gtk.Scrollable interface adjustments
        #[property(
            get,
            set = Self::set_hadjustment_prop,
            nullable,
            override_interface = gtk::Scrollable,
            name = "hadjustment"
        )]
        pub hadj: RefCell<Option<gtk::Adjustment>>,
        #[property(
            get,
            set = Self::set_vadjustment_prop,
            nullable,
            override_interface = gtk::Scrollable,
            name = "vadjustment"
        )]
        pub vadj: RefCell<Option<gtk::Adjustment>>,
        #[property(get, set, override_interface = gtk::Scrollable, name = "hscroll-policy")]
        pub hscroll_policy: Cell<gtk::ScrollablePolicy>,
        #[property(get, set, override_interface = gtk::Scrollable, name = "vscroll-policy")]
        pub vscroll_policy: Cell<gtk::ScrollablePolicy>,

        /// Key sequence state (human readable labels).
        #[property(get = Self::key_sequence_prop, type = Vec<String>, name = "key-sequence")]
        pub key_sequence: RefCell<Vec<String>>,

        /// Key table stack.
        #[property(get = Self::key_table_prop, type = Vec<String>, name = "key-table")]
        pub key_tables: RefCell<Vec<String>>,

        /// Read-only mode state (from the core surface).
        #[property(get = Self::readonly_prop, type = bool, name = "readonly")]
        _readonly: Cell<bool>,

        // ---- non-property state ----
        pub id: Cell<SurfaceId>,
        pub rt_surface: RefCell<Option<Rc<RtSurfaceHandle>>>,
        /// The core surface backing this GTK surface. This starts out
        /// None because it can't be initialized until there is an available
        /// GLArea that is realized.
        pub core_surface: RefCell<Option<Rc<dyn CoreSurface>>>,

        /// Cached metrics for libghostty callbacks
        pub size: Cell<SurfaceSize>,
        pub cursor_pos: Cell<CursorPos>,

        /// Various input method state. All related to key input.
        pub in_keyevent: Cell<IMKeyEvent>,
        pub im_composing: Cell<bool>,
        pub im_buf: RefCell<String>,

        /// True when we have a precision scroll in progress
        pub precision_scroll: Cell<bool>,

        // Progress bar
        pub progress_bar_timer: Cell<Option<glib::SourceId>>,

        /// The source that handles setting our child property.
        pub idle_rechild: Cell<Option<glib::SourceId>>,

        /// A weak reference to an inspector window.
        pub inspector: glib::WeakRef<InspectorWindow>,

        pub action_group: RefCell<Option<gio::SimpleActionGroup>>,
        pub vadj_handler: RefCell<Option<(gtk::Adjustment, glib::SignalHandlerId)>>,

        /// The context for this surface (window, tab, or split)
        pub context: Cell<NewSurfaceContext>,

        /// Whether primary paste (middle-click paste) is enabled.
        pub gtk_enable_primary_paste: Cell<bool>,

        /// True when a left mouse down was consumed purely for a focus
        /// change, and the matching left mouse release should also be
        /// suppressed.
        pub suppress_left_mouse_release: Cell<bool>,

        /// How much pending horizontal scroll do we have?
        pub pending_horizontal_scroll: Cell<f64>,

        /// Timer to reset the amount of horizontal scroll if the user
        /// stops scrolling.
        pub pending_horizontal_scroll_reset: Cell<Option<glib::SourceId>>,

        pub overrides: RefCell<SurfaceOverrides>,

        // Template children
        #[template_child]
        pub gl_area: TemplateChild<gtk::GLArea>,
        #[template_child]
        pub url_left: TemplateChild<gtk::Label>,
        #[template_child]
        pub url_right: TemplateChild<gtk::Label>,
        #[template_child]
        pub resize_overlay: TemplateChild<ResizeOverlay>,
        #[template_child]
        pub search_overlay: TemplateChild<SearchOverlay>,
        #[template_child]
        pub key_state_overlay: TemplateChild<KeyStateOverlay>,
        #[template_child]
        pub child_exited_overlay: TemplateChild<SurfaceChildExited>,
        #[template_child]
        pub context_menu: TemplateChild<gtk::PopoverMenu>,
        #[template_child]
        pub drop_target: TemplateChild<gtk::DropTarget>,
        #[template_child]
        pub progress_bar_overlay: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub terminal_page: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub im_context: TemplateChild<gtk::IMMulticontext>,
    }

    impl Default for Surface {
        fn default() -> Self {
            Self {
                bell_ringing: Cell::new(false),
                config: RefCell::new(None),
                child_exited: Cell::new(false),
                default_size: RefCell::new(None),
                error: Cell::new(false),
                font_size_request: RefCell::new(None),
                focused: Cell::new(true),
                min_size: RefCell::new(None),
                mouse_hidden: Cell::new(false),
                mouse_shape: Cell::new(MouseShape::Text),
                mouse_hover_url: RefCell::new(None),
                pwd: RefCell::new(None),
                title: RefCell::new(None),
                title_override: RefCell::new(None),
                zoom: Cell::new(false),
                is_split: Cell::new(false),
                hadj: RefCell::new(None),
                vadj: RefCell::new(None),
                hscroll_policy: Cell::new(gtk::ScrollablePolicy::Natural),
                vscroll_policy: Cell::new(gtk::ScrollablePolicy::Natural),
                key_sequence: RefCell::new(Vec::new()),
                key_tables: RefCell::new(Vec::new()),
                _readonly: Cell::new(false),
                id: Cell::new(0),
                rt_surface: RefCell::new(None),
                core_surface: RefCell::new(None),
                size: Cell::new(SurfaceSize {
                    width: 0,
                    height: 0,
                }),
                cursor_pos: Cell::new(CursorPos { x: 0.0, y: 0.0 }),
                in_keyevent: Cell::new(IMKeyEvent::False),
                im_composing: Cell::new(false),
                im_buf: RefCell::new(String::new()),
                precision_scroll: Cell::new(false),
                progress_bar_timer: Cell::new(None),
                idle_rechild: Cell::new(None),
                inspector: glib::WeakRef::new(),
                action_group: RefCell::new(None),
                vadj_handler: RefCell::new(None),
                context: Cell::new(NewSurfaceContext::Window),
                gtk_enable_primary_paste: Cell::new(true),
                suppress_left_mouse_release: Cell::new(false),
                pending_horizontal_scroll: Cell::new(0.0),
                pending_horizontal_scroll_reset: Cell::new(None),
                overrides: RefCell::new(SurfaceOverrides::default()),
                gl_area: TemplateChild::default(),
                url_left: TemplateChild::default(),
                url_right: TemplateChild::default(),
                resize_overlay: TemplateChild::default(),
                search_overlay: TemplateChild::default(),
                key_state_overlay: TemplateChild::default(),
                child_exited_overlay: TemplateChild::default(),
                context_menu: TemplateChild::default(),
                drop_target: TemplateChild::default(),
                progress_bar_overlay: TemplateChild::default(),
                error_page: TemplateChild::default(),
                terminal_page: TemplateChild::default(),
                im_context: TemplateChild::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Surface {
        const NAME: &'static str = "GhosttySurface";
        type Type = super::Surface;
        type ParentType = adw::Bin;
        type Interfaces = (gtk::Scrollable,);

        fn class_init(klass: &mut Self::Class) {
            ResizeOverlay::ensure_type();
            SearchOverlay::ensure_type();
            KeyStateOverlay::ensure_type();
            SurfaceChildExited::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl Surface {
        fn pwd_text(&self) -> Option<String> {
            self.pwd
                .borrow()
                .as_ref()
                .map(ghostty_os::osbytes::EncodedOs::to_text_lossy)
        }

        fn set_pwd_text(&self, value: Option<String>) {
            self.pwd
                .replace(value.map(ghostty_os::osbytes::EncodedOs::from));
        }

        fn mouse_shape_prop(&self) -> i32 {
            self.mouse_shape.get() as i32
        }

        fn set_mouse_shape_prop(&self, v: i32) {
            self.mouse_shape.set(mouse_shape_from_i32(v));
        }

        fn key_sequence_prop(&self) -> Vec<String> {
            self.key_sequence.borrow().clone()
        }

        fn key_table_prop(&self) -> Vec<String> {
            self.key_tables.borrow().clone()
        }

        fn readonly_prop(&self) -> bool {
            self.obj().get_readonly()
        }

        pub(super) fn set_bell_ringing_prop(&self, ringing: bool) {
            let obj = self.obj();

            // Prevent duplicate change notifications if the signals we emit
            // in this function cause this state to change again.
            let _guard = obj.freeze_notify();

            // Logic around bell reaction happens on every event even if
            // we're already in the ringing state.
            if ringing {
                obj.ring_bell();
            }

            // Property change only happens on actual state change
            if self.bell_ringing.get() != ringing {
                self.bell_ringing.set(ringing);
                obj.notify("bell-ringing");
            }
        }

        fn set_hadjustment_prop(&self, adj: Option<gtk::Adjustment>) {
            // Notification is emitted by the property machinery.
            self.hadj.replace(adj);
        }

        fn set_vadjustment_prop(&self, adj: Option<gtk::Adjustment>) {
            self.vadj.replace(adj);
        }
    }

    impl ObjectImpl for Surface {
        fn properties() -> &'static [glib::ParamSpec] {
            Self::derived_properties()
        }

        fn set_property(&self, id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            self.derived_set_property(id, value, pspec);
        }

        fn property(&self, id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            self.derived_property(id, pspec)
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("bell").build(),
                    Signal::builder("close-request").build(),
                    Signal::builder("clipboard-write")
                        .param_types([u32::static_type(), String::static_type()])
                        .build(),
                    Signal::builder("clipboard-read").build(),
                    Signal::builder("init").build(),
                    Signal::builder("menu").build(),
                    Signal::builder("present-request").build(),
                    Signal::builder("toggle-fullscreen").build(),
                    Signal::builder("toggle-maximize").build(),
                ]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            // Register ourselves so the core's `Target::Surface(id)` can be
            // mapped back to this widget.
            let id = NEXT_ID.with(|n| {
                let id = n.get();
                n.set(id + 1);
                id
            });
            self.id.set(id);
            SURFACES.with(|m| m.borrow_mut().insert(id, obj.downgrade()));

            // Initialize our actions
            obj.init_action_map();

            // If our configuration is null then we get the configuration
            // from the application.
            if self.config.borrow().is_none() {
                self.config.replace(Some(app().get_config()));
            }

            // Read GTK primary paste setting
            self.gtk_enable_primary_paste.set(
                crate::gsettings::get_bool(crate::gsettings::Key::GtkEnablePrimaryPaste)
                    .unwrap_or(true),
            );

            // Set up to handle items being dropped on our surface. Files can
            // be dropped from Nautilus and strings can be dropped from many
            // programs. The order of these types matter.
            self.drop_target.set_types(&[
                gdk::FileList::static_type(),
                gio::File::static_type(),
                String::static_type(),
            ]);

            // Setup properties we can't set from our Blueprint file.
            obj.set_cursor_from_name(Some("text"));

            // Initialize our config
            obj.prop_config();
        }

        fn dispose(&self) {
            // The inspector window holds on to our core surface to
            // deactivate the inspector; Zig does that from a weak notify
            // during our dispose. Closing the window here makes the inspector
            // widget deactivate the inspector while the core surface is
            // still alive.
            if let Some(inspector) = self.inspector.upgrade() {
                inspector.destroy();
            }

            if let Some((adj, id)) = self.vadj_handler.take() {
                adj.disconnect(id);
            }
            self.hadj.replace(None);
            self.vadj.replace(None);

            if let Some(timer) = self.progress_bar_timer.take() {
                timer.remove();
            }
            if let Some(src) = self.idle_rechild.take() {
                src.remove();
            }
            if let Some(src) = self.pending_horizontal_scroll_reset.take() {
                src.remove();
            }

            // This works around a GTK double-free bug where if you bind
            // to a top-level template child, it frees twice if the widget is
            // also the root child of the template. By unsetting the child
            // here, we avoid the double-free.
            self.obj().set_child(gtk::Widget::NONE);

            self.dispose_template();
        }
    }

    impl Drop for Surface {
        // Zig deinitializes the core surface in `finalize`, NOT `dispose`,
        // because the inspector relies on that with a weakRef.
        fn drop(&mut self) {
            let id = self.id.get();
            let _ = SURFACES.try_with(|m| {
                if let Ok(mut m) = m.try_borrow_mut() {
                    m.remove(&id);
                }
            });
            if let Some(core) = self.core_surface.take() {
                // Remove ourselves from the list of known surfaces in the
                // app. We do this before deinit in case a callback triggers
                // searching for this surface.
                if let Some(a) =
                    gio::Application::default().and_then(|a| a.downcast::<Application>().ok())
                {
                    a.core().delete_surface(id);
                }
                drop(core);
            }
        }
    }

    impl WidgetImpl for Surface {}
    impl BinImpl for Surface {}
    impl ScrollableImpl for Surface {}

    #[gtk::template_callbacks]
    impl Surface {
        // ---- closures (template expressions) ----

        /// Callback used to determine whether border should be shown around
        /// the surface.
        #[template_callback(function)]
        fn should_border_be_shown(config: Option<Config>, bell_ringing: bool) -> bool {
            // If the bell isn't ringing exit early because when the surface
            // is first created there's a race between this code being run
            // and the config being set on the surface. That way we don't
            // overwhelm people with the warning that we issue if the config
            // isn't set and overwhelm ourselves with large numbers of bug
            // reports.
            if !bell_ringing {
                return false;
            }

            let Some(config) = config else {
                log::warn!(
                    "config unavailable for computing whether border should be shown, likely bug"
                );
                return false;
            };

            config.get().bell_features.border
        }

        /// Callback used to determine whether unfocused-split-fill /
        /// unfocused-split-opacity should be applied to the surface
        #[template_callback(function)]
        fn should_unfocused_split_be_shown(
            search_active: bool,
            focused: bool,
            is_split: bool,
        ) -> bool {
            !search_active && !focused && is_split
        }

        // ---- property notifications ----

        #[template_callback]
        fn notify_config(
            _emitter: &super::Surface,
            _pspec: &glib::ParamSpec,
            this: &super::Surface,
        ) {
            this.prop_config();
        }

        #[template_callback]
        fn notify_error(
            _emitter: &super::Surface,
            _pspec: &glib::ParamSpec,
            this: &super::Surface,
        ) {
            this.prop_error();
        }

        #[template_callback]
        fn notify_mouse_hover_url(
            _emitter: &super::Surface,
            _pspec: &glib::ParamSpec,
            this: &super::Surface,
        ) {
            this.prop_mouse_hover_url();
        }

        #[template_callback]
        fn notify_mouse_hidden(
            _emitter: &super::Surface,
            _pspec: &glib::ParamSpec,
            this: &super::Surface,
        ) {
            this.prop_mouse_hidden();
        }

        #[template_callback]
        fn notify_mouse_shape(
            _emitter: &super::Surface,
            _pspec: &glib::ParamSpec,
            this: &super::Surface,
        ) {
            this.prop_mouse_shape();
        }

        #[template_callback]
        fn notify_vadjustment(
            _emitter: &super::Surface,
            _pspec: &glib::ParamSpec,
            this: &super::Surface,
        ) {
            this.prop_vadjustment();
        }

        // ---- child widgets ----

        #[template_callback]
        fn child_exited_close(_overlay: &SurfaceChildExited, this: &super::Surface) {
            // This closes the surface with no confirmation.
            this.close();
        }

        #[template_callback]
        fn context_menu_closed(_popover: &gtk::PopoverMenu, this: &super::Surface) {
            // When the context menu closes, it moves focus back to the tab
            // bar if there are tabs. That's not correct. We need to grab it
            // on the surface.
            this.grab_focus();
        }

        #[template_callback]
        fn search_stop(_overlay: &SearchOverlay, this: &super::Surface) {
            let Some(surface) = this.core() else { return };
            if let Err(err) =
                surface.perform_binding_action(&ghostty_input::binding::Action::EndSearch)
            {
                log::warn!("unable to perform end_search action err={err}");
            }
            this.imp().gl_area.grab_focus();
        }

        #[template_callback]
        fn search_changed(_overlay: &SearchOverlay, needle: Option<String>, this: &super::Surface) {
            let Some(surface) = this.core() else { return };
            if let Err(err) = surface.perform_binding_action(
                // Binding payloads are config strings: the needle is plain
                // UI text, so escape it instead of decoding it.
                &ghostty_input::binding::Action::Search(
                    ghostty_os::osbytes::EncodedOs::from_plain_text(&needle.unwrap_or_default()),
                ),
            ) {
                log::warn!("unable to perform search action err={err}");
            }
        }

        #[template_callback]
        fn search_next_match(_overlay: &SearchOverlay, this: &super::Surface) {
            let Some(surface) = this.core() else { return };
            if let Err(err) =
                surface.perform_binding_action(&ghostty_input::binding::Action::NavigateSearch(
                    ghostty_input::binding::action::NavigateSearch::Next,
                ))
            {
                log::warn!("unable to perform navigate_search action err={err}");
            }
        }

        #[template_callback]
        fn search_previous_match(_overlay: &SearchOverlay, this: &super::Surface) {
            let Some(surface) = this.core() else { return };
            if let Err(err) =
                surface.perform_binding_action(&ghostty_input::binding::Action::NavigateSearch(
                    ghostty_input::binding::action::NavigateSearch::Previous,
                ))
            {
                log::warn!("unable to perform navigate_search action err={err}");
            }
        }

        #[template_callback]
        fn url_mouse_enter(
            _ec: &gtk::EventControllerMotion,
            _x: f64,
            _y: f64,
            this: &super::Surface,
        ) {
            this.imp().url_right.set_visible(true);
        }

        #[template_callback]
        fn url_mouse_leave(_ec: &gtk::EventControllerMotion, this: &super::Surface) {
            this.imp().url_right.set_visible(false);
        }

        // ---- drag & drop ----

        #[template_callback]
        #[allow(clippy::needless_pass_by_value)]
        fn drop(
            _dt: &gtk::DropTarget,
            value: glib::Value,
            _x: f64,
            _y: f64,
            this: &super::Surface,
        ) -> bool {
            if let Ok(list) = value.get::<gdk::FileList>() {
                let mut escaped = Vec::new();
                {
                    let mut w = ghostty_os::ShellEscapeWriter::new(&mut escaped);
                    for file in list.files() {
                        let Some(path) = file.path() else { continue };
                        // Raw path bytes: non-UTF-8 names must survive.
                        let path = ghostty_os::osbytes::os_bytes(path.as_os_str());
                        if let Err(err) = w.write_all(&path).and_then(|()| w.write_all(b"\n")) {
                            log::error!("unable to write path to buffer: {err}");
                        }
                    }
                }
                this.paste_bytes(&escaped);
                return true;
            }

            if let Ok(file) = value.get::<gio::File>() {
                let Some(path) = file.path() else {
                    return false;
                };
                let mut escaped = Vec::new();
                {
                    let mut w = ghostty_os::ShellEscapeWriter::new(&mut escaped);
                    if let Err(err) = w
                        .write_all(&ghostty_os::osbytes::os_bytes(path.as_os_str()))
                        .and_then(|()| w.write_all(b"\n"))
                    {
                        log::error!("unable to write path to buffer: {err}");
                        return false;
                    }
                }
                // NOTE: faithful to Zig, which builds the escaped string for
                // a single dropped `GFile` but never pastes it.
                return true;
            }

            if value.type_().is_a(glib::Type::STRING) {
                if let Ok(Some(string)) = value.get::<Option<String>>() {
                    this.paste(&string);
                }
                return true;
            }

            true
        }

        // ---- keyboard ----

        #[template_callback]
        fn key_pressed(
            ec_key: &gtk::EventControllerKey,
            keyval: u32,
            keycode: u32,
            state: gdk::ModifierType,
            this: &super::Surface,
        ) -> bool {
            this.key_event(KeyAction::Press, ec_key, keyval, keycode, state)
        }

        #[template_callback]
        fn key_released(
            ec_key: &gtk::EventControllerKey,
            keyval: u32,
            keycode: u32,
            state: gdk::ModifierType,
            this: &super::Surface,
        ) {
            let _ = this.key_event(KeyAction::Release, ec_key, keyval, keycode, state);
        }

        #[template_callback]
        fn focus_enter(_ec: &gtk::EventControllerFocus, this: &super::Surface) {
            let imp = this.imp();
            imp.focused.set(true);
            imp.im_context.focus_in();
            idle_focus(this);
            this.notify("focused");

            // Bell stops ringing as soon as we gain focus
            this.set_bell_ringing(false);
        }

        #[template_callback]
        fn focus_leave(_ec: &gtk::EventControllerFocus, this: &super::Surface) {
            let imp = this.imp();
            imp.focused.set(false);
            imp.im_context.focus_out();
            idle_focus(this);
            this.notify("focused");
        }

        // ---- mouse ----

        #[template_callback]
        fn mouse_down(
            gesture: &gtk::GestureClick,
            _n_press: i32,
            x: f64,
            y: f64,
            this: &super::Surface,
        ) {
            let Some(event) = gesture.current_event() else {
                return;
            };
            this.mouse_down_impl(
                gesture.current_button(),
                event.modifier_state(),
                x,
                y,
                false,
            );
        }

        #[template_callback]
        fn mouse_up(
            gesture: &gtk::GestureClick,
            _n_press: i32,
            _x: f64,
            _y: f64,
            this: &super::Surface,
        ) {
            let Some(event) = gesture.current_event() else {
                return;
            };
            this.mouse_up_impl(
                gesture.current_button(),
                event.modifier_state(),
                Some(&event),
            );
        }

        #[template_callback]
        fn mouse_motion(ec: &gtk::EventControllerMotion, x: f64, y: f64, this: &super::Surface) {
            let Some(event) = ec.current_event() else {
                return;
            };
            this.mouse_motion_impl(event.modifier_state(), x, y);
        }

        #[template_callback]
        fn mouse_leave(ec_motion: &gtk::EventControllerMotion, this: &super::Surface) {
            let Some(event) = ec_motion.current_event() else {
                return;
            };

            // Get our modifiers
            if let Some(surface) = this.core() {
                // If we have a core surface then we can send the cursor pos
                // callback with an invalid position to indicate the mouse
                // left.
                let mods = gtk_key::translate_mods(event.modifier_state());
                if let Err(err) =
                    surface.cursor_pos_callback(CursorPos { x: -1.0, y: -1.0 }, Some(mods))
                {
                    log::warn!("error in cursor pos callback err={err}");
                }
            }
        }

        #[template_callback]
        fn scroll_vertical_begin(_ec: &gtk::EventControllerScroll, this: &super::Surface) {
            this.imp().precision_scroll.set(true);
        }

        #[template_callback]
        fn scroll_vertical_end(_ec: &gtk::EventControllerScroll, this: &super::Surface) {
            this.imp().precision_scroll.set(false);
        }

        #[template_callback]
        fn scroll_vertical(
            _ec: &gtk::EventControllerScroll,
            x: f64,
            y: f64,
            this: &super::Surface,
        ) -> bool {
            this.scroll_vertical_impl(x, y)
        }

        #[template_callback]
        fn scroll_horizontal(
            ec: &gtk::EventControllerScroll,
            x: f64,
            _y: f64,
            this: &super::Surface,
        ) -> bool {
            match ec.unit() {
                gdk::ScrollUnit::Surface => {}
                _ => return false,
            }

            this.scroll_horizontal_impl(x)
        }

        // ---- input method ----

        #[template_callback]
        pub(super) fn im_preedit_start(_ctx: &gtk::IMContext, this: &super::Surface) {
            // Start our composing state for the input method and reset our
            // input buffer to empty.
            let imp = this.imp();
            imp.im_composing.set(true);
            imp.im_buf.borrow_mut().clear();
        }

        #[template_callback]
        pub(super) fn im_preedit_changed(ctx: &gtk::IMContext, this: &super::Surface) {
            let imp = this.imp();

            // Any preedit change should mark that we're composing. Its
            // possible this is false using fcitx5-hangul and typing
            // "dkssud<space>" ("안녕"). The second "s" results in a "commit"
            // for "안" which sets composing to false, but then immediately
            // sends a preedit change for the next symbol. With composing set
            // to false we won't commit this text. Therefore, we must ensure
            // it is set here.
            imp.im_composing.set(true);

            // We can't set our preedit on our surface unless we're realized.
            // We do this now because we want to still keep our input method
            // state coherent.
            let Some(surface) = this.core() else { return };

            // Get our pre-edit string that we'll use to show the user.
            let (text, _, _) = ctx.preedit_string();

            // Update our preedit state in Ghostty core
            if let Err(err) = surface.preedit_callback(Some(text.as_bytes())) {
                log::warn!("error in preedit callback err={err}");
            }
        }

        #[template_callback]
        pub(super) fn im_preedit_end(_ctx: &gtk::IMContext, this: &super::Surface) {
            // End our composing state for GTK, allowing us to commit the text.
            let imp = this.imp();
            imp.im_composing.set(false);

            // End our preedit state in Ghostty core
            let Some(surface) = this.core() else { return };
            if let Err(err) = surface.preedit_callback(None) {
                log::warn!("error in preedit callback err={err}");
            }
        }

        #[template_callback]
        pub(super) fn im_commit(_ctx: &gtk::IMContext, str: &str, this: &super::Surface) {
            let imp = this.imp();

            // We need to handle commit specially if we're in a key event.
            // Specifically, GTK will send us a commit event for basic key
            // encodings like "a" (on a US layout keyboard). We don't want
            // to treat this as IME committed text because we want to
            // associate it with a key event (i.e. "a" key press).
            match imp.in_keyevent.get() {
                // If we're not in a key event then this commit is from some
                // other source (i.e. on-screen keyboard, tablet, etc.) and we
                // want to commit the text to the core surface.
                //
                // If we're in a composing state and in a key event then this
                // key event is resulting in a commit of multiple keypresses
                // and we don't want to encode it alongside the keypress.
                IMKeyEvent::False | IMKeyEvent::Composing => {}

                // If we're not composing then this commit is just a normal
                // key encoding and we want our key event to handle it so
                // that Ghostty can be aware of the key event alongside the
                // text.
                IMKeyEvent::NotComposing => {
                    if str.len() > 128 {
                        log::warn!("not enough buffer space for input method commit");
                        return;
                    }

                    // Copy our committed text to the buffer
                    imp.im_buf.replace(str.to_owned());
                    return;
                }
            }

            // If we reach this point from above it means we're composing OR
            // not in a keypress. In either case, we want to commit the text
            // given to us because that's what GTK is asking us to do. If
            // we're not in a keypress it means that this commit came via a
            // non-keyboard event (i.e. on-screen keyboard, tablet of some
            // kind, etc.).

            // Committing ends composing state
            imp.im_composing.set(false);

            // We can't set our preedit on our surface unless we're realized.
            // We do this now because we want to still keep our input method
            // state coherent.
            if let Some(surface) = this.core() {
                // End our preedit state. Well-behaved input methods do this
                // for us by triggering a preedit-end event but some do not
                // (ibus 1.5.29).
                if let Err(err) = surface.preedit_callback(None) {
                    log::warn!("error in preedit callback err={err}");
                }

                // Send the text to the core surface, associated with no key
                // (an invalid key, which should produce no PTY encoding).
                if let Err(err) = surface.key_callback(KeyEvent {
                    action: KeyAction::Press,
                    key: Key::Unidentified,
                    mods: Mods::default(),
                    consumed_mods: Mods::default(),
                    composing: false,
                    utf8: str.as_bytes(),
                    ..KeyEvent::default()
                }) {
                    log::warn!("error in key callback err={err}");
                }
            }
        }

        // ---- GLArea ----

        #[template_callback]
        fn gl_realize(_area: &gtk::GLArea, this: &super::Surface) {
            log::debug!("realize");

            // Make the GL area current so we can detect any OpenGL errors.
            // If we have errors here we can't render and we switch to the
            // error state.
            let imp = this.imp();
            imp.gl_area.make_current();
            if let Some(err) = imp.gl_area.error() {
                log::warn!("failed to make GL context current: {}", err.message());
                log::warn!("this error is almost always due to a library, driver, or GTK issue");
                log::warn!(
                    "this is a common cause of this issue: https://ghostty.org/docs/help/gtk-opengl-context"
                );
                this.set_error(true);
                return;
            }

            // If we already have an initialized surface then we notify it.
            // If we don't, we'll initialize it on the first resize so we have
            // our proper initial dimensions.
            if let Some(core) = this.core() {
                match core.renderer_realized() {
                    Ok(()) => this.redraw(),
                    Err(err) => log::warn!("core displayRealized failed err={err}"),
                }
            }

            // Setup our input method. We do this here because this will
            // create a strong reference back to ourself and we want to be
            // able to release that in unrealize.
            imp.im_context.set_client_widget(Some(this));
        }

        #[template_callback]
        fn gl_unrealize(gl_area: &gtk::GLArea, this: &super::Surface) {
            log::debug!("unrealize");

            // Notify our core surface
            let imp = this.imp();
            if let Some(surface) = this.core() {
                // There is no guarantee that our GLArea context is current
                // when unrealize is emitted, so we need to make it current.
                gl_area.make_current();
                if let Some(err) = gl_area.error() {
                    // I don't know a scenario this can happen, but it means
                    // we probably leaked memory because displayUnrealized
                    // below frees resources that aren't specifically OpenGL
                    // related. I didn't make the OpenGL renderer handle this
                    // scenario because I don't know if its even possible
                    // under valid circumstances, so let's log.
                    log::warn!(
                        "gl_area_make_current failed in unrealize msg={}",
                        err.message()
                    );
                    log::warn!("OpenGL resources and memory likely leaked");
                    return;
                }

                surface.renderer_unrealized();
            }

            // Unset our input method
            imp.im_context.set_client_widget(gtk::Widget::NONE);
        }

        #[template_callback]
        fn gl_render(_area: &gtk::GLArea, _ctx: &gdk::GLContext, this: &super::Surface) -> bool {
            // If we don't have a surface then we failed to initialize for
            // some reason and there's nothing to draw to the GLArea.
            let Some(surface) = this.core() else {
                return true;
            };

            // Framebuffer 0 means "the renderer queries GL_FRAMEBUFFER_BINDING
            // itself" (GLArea has bound its own FBO at this point).
            if let Err(err) = surface.draw_frame(0) {
                log::warn!("failed to draw frame err={err}");
                return false;
            }

            true
        }

        #[template_callback]
        fn gl_resize(gl_area: &gtk::GLArea, width: i32, height: i32, this: &super::Surface) {
            // Some debug output to help understand what GTK is telling us.
            {
                let scale_factor = gl_area.scale_factor();
                let window_scale_factor = gl_area
                    .root()
                    .and_then(|r| r.surface())
                    .map_or(0, |s| s.scale_factor());
                log::debug!(
                    "gl resize width={width} height={height} scale={scale_factor} window_scale={window_scale_factor}"
                );
            }

            // Store our cached size
            let imp = this.imp();
            let size = SurfaceSize {
                width: width.max(0) as u32,
                height: height.max(0) as u32,
            };
            imp.size.set(size);

            // If our surface is realize, we send callbacks.
            if let Some(surface) = this.core() {
                // We also update the content scale because there is no signal
                // for content scale change and it seems to trigger a resize
                // event.
                if let Err(err) = surface.content_scale_callback(this.get_content_scale()) {
                    log::warn!("error in content scale callback err={err}");
                }

                if let Err(err) = surface.size_callback(size) {
                    log::warn!("error in size callback err={err}");
                }

                // Setup our resize overlay if configured
                this.resize_overlay_schedule();

                return;
            }

            // If we don't have a surface, then we initialize it.
            if let Err(err) = this.init_surface() {
                log::warn!("surface failed to initialize err={err:?}");
            }
        }
    }

    /// The focus callback must be triggered on an idle loop source because
    /// there are actions within libghostty callbacks (such as showing close
    /// confirmation dialogs) that can trigger focus loss and cause a deadlock
    /// because the lock may be held during the callback.
    fn idle_focus(this: &super::Surface) {
        let this = this.clone();
        glib::idle_add_local_once(move || {
            let imp = this.imp();
            let Some(surface) = this.core() else { return };
            if let Err(err) = surface.focus_callback(imp.focused.get()) {
                log::warn!("error in focus callback err={err}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn inherited_lang_keeps_non_utf8_bytes() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let mut env = ghostty_os::EnvMap::new();
        env.insert("LANG", "zz");
        let saved = std::ffi::OsString::from_vec(b"C\xff".to_vec());
        restore_language(&mut env, Some(&saved));
        assert_eq!(env.get("LANG").unwrap().as_bytes(), b"C\xff");
        restore_language(&mut env, None);
        assert!(env.get("LANG").is_none());
    }

    #[test]
    fn confirmed_paste_decodes_only_encoded_text() {
        // Ordinary text containing the escape char + U+00FF stays intact.
        let text = "a\u{fdd0}\u{ff}b";
        assert_eq!(
            clipboard::confirmed_paste_bytes(text, false),
            text.as_bytes()
        );
        // Text escaped by `paste_bytes` is decoded back to the raw bytes.
        let enc = ghostty_os::osbytes::bytes_to_config_string(b"a\xffb");
        assert_eq!(clipboard::confirmed_paste_bytes(&enc, true), b"a\xffb");
    }

    #[test]
    fn compute_fraction_clamps() {
        assert!((compute_fraction(100) - 1.0).abs() < f64::EPSILON);
        assert!((compute_fraction(255) - 1.0).abs() < f64::EPSILON);
        assert!(compute_fraction(0).abs() < f64::EPSILON);
        assert!((compute_fraction(50) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn translate_mouse_button_maps() {
        assert_eq!(translate_mouse_button(1), MouseButton::Left);
        assert_eq!(translate_mouse_button(2), MouseButton::Middle);
        assert_eq!(translate_mouse_button(3), MouseButton::Right);
        assert_eq!(translate_mouse_button(11), MouseButton::Eleven);
        assert_eq!(translate_mouse_button(0), MouseButton::Unknown);
        assert_eq!(translate_mouse_button(12), MouseButton::Unknown);
    }

    #[test]
    fn mouse_shape_roundtrip() {
        for v in 0..34 {
            assert_eq!(mouse_shape_from_i32(v) as i32, v);
        }
        assert_eq!(mouse_shape_from_i32(-1), MouseShape::Default);
        assert_eq!(mouse_shape_from_i32(99), MouseShape::Default);
        assert_eq!(mouse_shape_name(MouseShape::ContextMenu), "context-menu");
    }
}
