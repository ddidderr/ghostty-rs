//! Port of apprt/gtk/class/command_palette.zig (`GhosttyCommandPalette` and
//! the private `GhosttyCommand` list item wrapper).
//!
//! Property `config`; signal `trigger(action: BindingAction)`. The palette
//! keeps itself alive (a self-reference) from `new()` until its dialog is
//! closed, like the Zig `refSink`/`unref` dance.

use std::{cell::RefCell, cmp::Ordering, sync::OnceLock};

use adw::{prelude::*, subclass::prelude::*};
use ghostty_input::{binding::Action as BindingAction, command::Command as InputCommand};
use gtk::{CompositeTemplate, TemplateChild, gio, glib, glib::subclass::Signal};

use super::{application::Application, config::Config, surface::Surface, window::Window};

/// Boxed wrapper so a binding action can travel through the `trigger`
/// signal (Zig passes `*const input.Binding.Action`).
#[derive(Debug, Clone, glib::Boxed)]
#[boxed_type(name = "GhosttyBindingAction")]
pub struct BoxedAction(pub BindingAction);

// ---------------------------------------------------------------------
// Command (list item)

mod command_imp {
    use adw::prelude::*;
    use gtk::glib::subclass::prelude::*;

    use super::{
        BindingAction,
        Config,
        InputCommand,
        RefCell,
        Surface,
        ghostty_command_title_for_jump,
        glib,
    };

    pub enum CommandData {
        Regular {
            command: InputCommand,
        },
        Jump {
            surface: glib::WeakRef<Surface>,
            title: Option<String>,
            description: Option<String>,
            sort_key: usize,
        },
    }

    impl Default for CommandData {
        fn default() -> Self {
            CommandData::Regular {
                command: InputCommand::new(BindingAction::Ignore, "", ""),
            }
        }
    }

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::Command)]
    pub struct Command {
        #[property(get, set, nullable)]
        pub config: RefCell<Option<Config>>,

        pub data: RefCell<CommandData>,

        #[property(name = "action-key", get = Self::prop_get_action_key, type = Option<String>)]
        #[property(name = "action", get = Self::prop_get_action, type = Option<String>)]
        #[property(name = "title", get = Self::prop_get_title, type = Option<String>)]
        #[property(name = "description", get = Self::prop_get_description, type = Option<String>)]
        _props: (),
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Command {
        const NAME: &'static str = "GhosttyCommand";
        type Type = super::Command;
        type ParentType = glib::Object;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Command {
        fn dispose(&self) {
            self.config.replace(None);
            if let CommandData::Jump { surface, .. } = &*self.data.borrow() {
                surface.set(None);
            }
        }
    }

    impl Command {
        fn prop_get_action_key(&self) -> Option<String> {
            match &*self.data.borrow() {
                CommandData::Regular { command } => Some(command.action.to_string()),
                CommandData::Jump { .. } => None,
            }
        }

        fn prop_get_action(&self) -> Option<String> {
            let data = self.data.borrow();
            let CommandData::Regular { command } = &*data else {
                return None;
            };
            let config = self.config.borrow();
            let cfg = config.as_ref()?.get();
            let trigger = cfg.keybind.set.get_trigger(&command.action)?;
            crate::key::accel_from_trigger(trigger)
        }

        fn prop_get_title(&self) -> Option<String> {
            let mut data = self.data.borrow_mut();
            match &mut *data {
                CommandData::Regular { command } => Some(command.title.to_text_lossy()),
                CommandData::Jump { surface, title, .. } => {
                    if let Some(t) = title {
                        return Some(t.clone());
                    }
                    let surface = surface.upgrade()?;
                    *title = Some(ghostty_command_title_for_jump(&surface));
                    title.clone()
                }
            }
        }

        fn prop_get_description(&self) -> Option<String> {
            let mut data = self.data.borrow_mut();
            match &mut *data {
                CommandData::Regular { command } => Some(command.description.to_text_lossy()),
                CommandData::Jump {
                    surface,
                    description,
                    ..
                } => {
                    if let Some(d) = description {
                        return Some(d.clone());
                    }
                    let surface = surface.upgrade()?;
                    let title = surface
                        .get_effective_title()
                        .unwrap_or_else(|| "Untitled".to_string());
                    // Display text only.
                    if let Some(pwd) = surface
                        .get_pwd()
                        .map(|p| String::from_utf8_lossy(&p.to_bytes()).into_owned())
                        && !title.contains(&pwd)
                    {
                        *description = Some(pwd);
                    }
                    description.clone()
                }
            }
        }
    }
}

/// "Focus: {title}" for jump commands.
fn ghostty_command_title_for_jump(surface: &Surface) -> String {
    let t = surface
        .get_effective_title()
        .unwrap_or_else(|| "Untitled".to_string());
    format!("Focus: {t}")
}

glib::wrapper! {
    /// Object wrapping a command so GTK list models can consume it.
    pub struct Command(ObjectSubclass<command_imp::Command>);
}

impl Command {
    /// Create a regular command.
    pub fn new(config: &Config, command: &InputCommand) -> Self {
        let obj: Self = glib::Object::builder().property("config", config).build();
        obj.imp().data.replace(command_imp::CommandData::Regular {
            command: command.clone(),
        });
        obj
    }

    /// Create a new jump command that focuses a specific surface.
    pub fn new_jump(config: &Config, surface: &Surface) -> Self {
        let obj: Self = glib::Object::builder().property("config", config).build();
        obj.imp().data.replace(command_imp::CommandData::Jump {
            surface: surface.downgrade(),
            title: None,
            description: None,
            // TODO: Replace with surface id whenever Ghostty adds one
            sort_key: surface.as_ptr() as usize,
        });
        obj
    }

    /// The action (None for jump commands).
    pub fn get_action(&self) -> Option<BindingAction> {
        match &*self.imp().data.borrow() {
            command_imp::CommandData::Regular { command } => Some(command.action.clone()),
            command_imp::CommandData::Jump { .. } => None,
        }
    }

    pub fn is_jump(&self) -> bool {
        matches!(
            &*self.imp().data.borrow(),
            command_imp::CommandData::Jump { .. }
        )
    }

    /// Strong reference to the jump surface, if it still exists.
    pub fn get_jump_surface(&self) -> Option<Surface> {
        match &*self.imp().data.borrow() {
            command_imp::CommandData::Regular { .. } => None,
            command_imp::CommandData::Jump { surface, .. } => surface.upgrade(),
        }
    }

    fn sort_key(&self) -> Option<usize> {
        match &*self.imp().data.borrow() {
            command_imp::CommandData::Regular { .. } => None,
            command_imp::CommandData::Jump { sort_key, .. } => Some(*sort_key),
        }
    }
}

/// Compare two commands for sorting (`compareCommands`). Sorts
/// alphabetically by title (case-insensitive), with colon normalization so
/// "Foo:" sorts before "Foo Bar:". Uses sort_key as tie-breaker. Returns
/// true if `a` sorts before `b`.
fn compare_commands(a: &Command, b: &Command) -> bool {
    let Some(a_title) = a.property::<Option<String>>("title") else {
        return false;
    };
    let Some(b_title) = b.property::<Option<String>>("title") else {
        return true;
    };
    title_less_than(&a_title, &b_title).unwrap_or_else(|| {
        // Titles are equal - use sort_key as tie-breaker if both are jump
        // commands.
        match (a.sort_key(), b.sort_key()) {
            (Some(ka), Some(kb)) => ka < kb,
            _ => false,
        }
    })
}

/// Returns Some(ordering result) if the titles differ, None if equal.
fn title_less_than(a_title: &str, b_title: &str) -> Option<bool> {
    let a = a_title.as_bytes();
    let b = b_title.as_bytes();
    for i in 0..a.len().min(b.len()) {
        let norm = |c: u8| if c == b':' { b'\t' } else { c }.to_ascii_lowercase();
        let (al, bl) = (norm(a[i]), norm(b[i]));
        if al != bl {
            return Some(al < bl);
        }
    }
    // If one title is a prefix of the other, shorter one comes first
    if a.len() != b.len() {
        return Some(a.len() < b.len());
    }
    None
}

/// Check if an action is supported on GTK.
fn is_action_supported_on_gtk(action: &BindingAction) -> bool {
    !matches!(
        action,
        BindingAction::CloseAllWindows
            | BindingAction::ToggleSecureInput
            | BindingAction::CheckForUpdates
            | BindingAction::Redo
            | BindingAction::Undo
            | BindingAction::ResetWindowSize
            | BindingAction::ToggleWindowFloatOnTop
    )
}

// ---------------------------------------------------------------------
// CommandPalette

mod imp {
    use adw::{prelude::*, subclass::prelude::*};
    use gtk::{gio, glib};

    use super::{
        Application,
        BoxedAction,
        Command,
        CompositeTemplate,
        Config,
        OnceLock,
        Ordering,
        RefCell,
        Signal,
        TemplateChild,
        compare_commands,
        is_action_supported_on_gtk,
    };

    #[derive(Default, glib::Properties, CompositeTemplate)]
    #[properties(wrapper_type = super::CommandPalette)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.5/command-palette.ui")]
    pub struct CommandPalette {
        /// The configuration that this command palette is using.
        #[property(get, set, nullable)]
        pub config: RefCell<Option<Config>>,

        /// Self reference held while the palette is alive (until the dialog
        /// closes), see `new()`.
        pub self_ref: RefCell<Option<super::CommandPalette>>,

        #[template_child]
        pub dialog: TemplateChild<adw::Dialog>,
        #[template_child]
        pub search: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub view: TemplateChild<gtk::ListView>,
        #[template_child]
        pub model: TemplateChild<gtk::SingleSelection>,
        #[template_child]
        pub source: TemplateChild<gio::ListStore>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CommandPalette {
        const NAME: &'static str = "GhosttyCommandPalette";
        type Type = super::CommandPalette;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Command::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CommandPalette {
        fn constructed(&self) {
            self.parent_constructed();
            // Listen for any changes to our config.
            self.obj()
                .connect_config_notify(super::CommandPalette::prop_config);
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("trigger")
                        .param_types([BoxedAction::static_type()])
                        .build(),
                ]
            })
        }

        fn dispose(&self) {
            self.source.remove_all();
            self.config.replace(None);
            self.dispose_template();
        }
    }

    impl WidgetImpl for CommandPalette {}
    impl BinImpl for CommandPalette {}

    impl CommandPalette {
        pub(super) fn collect_jump_commands(config: &Config, out: &mut Vec<Command>) {
            let app = Application::default();
            for surface in app.surfaces() {
                out.push(Command::new_jump(config, &surface));
            }
        }

        pub(super) fn collect_regular_commands(config: &Config, out: &mut Vec<Command>) {
            let cfg = config.get();
            for command in &cfg.command_palette_entry.value {
                // Filter out actions that are not implemented or don't make
                // sense for GTK.
                if !is_action_supported_on_gtk(&command.action) {
                    continue;
                }
                out.push(Command::new(config, command));
            }
        }

        pub(super) fn sort(commands: &mut [Command]) {
            // std.mem.sort is stable; sort_by is stable too.
            commands.sort_by(|a, b| {
                if compare_commands(a, b) {
                    Ordering::Less
                } else if compare_commands(b, a) {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            });
        }
    }

    #[gtk::template_callbacks]
    impl CommandPalette {
        #[template_callback]
        fn closed(_dialog: &adw::Dialog, palette: &super::CommandPalette) {
            // Drop the self reference taken in `new()`.
            palette.imp().self_ref.replace(None);
        }

        #[template_callback]
        fn search_stopped(_entry: &gtk::SearchEntry, palette: &super::CommandPalette) {
            // ESC was pressed - close the palette
            palette.close();
        }

        #[template_callback]
        fn search_activated(_entry: &gtk::SearchEntry, palette: &super::CommandPalette) {
            // If Enter is pressed, activate the selected entry
            let pos = palette.imp().model.selected();
            palette.activated(pos);
        }

        #[template_callback]
        fn row_activated(_view: &gtk::ListView, pos: u32, palette: &super::CommandPalette) {
            palette.activated(pos);
        }
    }
}

glib::wrapper! {
    pub struct CommandPalette(ObjectSubclass<imp::CommandPalette>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl CommandPalette {
    /// Create a new instance of the command palette. The palette keeps
    /// itself alive until its dialog is closed (the caller's reference is
    /// independent).
    pub fn new() -> Self {
        crate::register_resources();
        let obj: Self = glib::Object::new();
        // Sink ourselves so that we aren't floating anymore. We'll drop
        // this reference when the palette is closed or an action is
        // activated.
        obj.imp().self_ref.replace(Some(obj.clone()));
        obj
    }

    fn prop_config(&self) {
        let imp = self.imp();
        let Some(config) = self.config() else {
            log::warn!("command palette does not have a config!");
            return;
        };

        // Clear existing binds
        imp.source.remove_all();

        let mut commands: Vec<Command> = Vec::new();
        imp::CommandPalette::collect_jump_commands(&config, &mut commands);
        imp::CommandPalette::collect_regular_commands(&config, &mut commands);
        imp::CommandPalette::sort(&mut commands);

        for cmd in &commands {
            imp.source.append(cmd);
        }
    }

    fn close(&self) {
        self.imp().dialog.close();
    }

    /// Show or hide the command palette dialog. If the dialog is shown it
    /// will be modal over the given window.
    pub fn toggle(&self, window: &Window) {
        let imp = self.imp();

        // If the dialog has been shown, close it.
        if imp.dialog.is_realized() {
            self.close();
            return;
        }

        // Show the dialog
        imp.dialog.present(Some(window));

        // Focus on the search bar when opening the dialog
        imp.search.grab_focus();
    }

    /// Send the `trigger` signal for the item at `pos` of the filtered model.
    fn activated(&self, pos: u32) {
        let imp = self.imp();

        // Use the model and not the source here to use the list of
        // *visible* results
        let object = imp.model.upcast_ref::<gio::ListModel>().item(pos);

        // Close before running the action in order to avoid being replaced
        // by another dialog (such as the change title dialog). If that
        // occurs then the command palette dialog won't be counted as having
        // closed properly and cannot receive focus when reopened.
        self.close();

        let Some(cmd) = object.and_then(|o| o.downcast::<Command>().ok()) else {
            return;
        };

        // Handle jump commands differently
        if cmd.is_jump() {
            if let Some(surface) = cmd.get_jump_surface() {
                surface.present();
            }
            return;
        }

        // Regular command - emit trigger signal
        let Some(action) = cmd.get_action() else {
            return;
        };
        self.emit_by_name::<()>("trigger", &[&BoxedAction(action)]);
    }

    /// Connect to the `trigger` signal, emitted when a command from the
    /// palette is activated.
    pub fn connect_trigger<F: Fn(&Self, &BindingAction) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "trigger",
            false,
            glib::closure_local!(move |obj: &Self, action: BoxedAction| f(obj, &action.0)),
        )
    }
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_ordering_colon_normalization() {
        // "Foo:" sorts before "Foo Bar:"
        assert_eq!(title_less_than("Foo:", "Foo Bar:"), Some(true));
        assert_eq!(title_less_than("Foo Bar:", "Foo:"), Some(false));
    }

    #[test]
    fn title_ordering_case_insensitive_and_prefix() {
        assert_eq!(title_less_than("abc", "ABD"), Some(true));
        assert_eq!(title_less_than("abc", "ABC"), None);
        assert_eq!(title_less_than("ab", "abc"), Some(true));
    }

    #[test]
    fn unsupported_actions_filtered() {
        assert!(!is_action_supported_on_gtk(&BindingAction::Undo));
        assert!(!is_action_supported_on_gtk(&BindingAction::CloseAllWindows));
        assert!(is_action_supported_on_gtk(&BindingAction::NewTab));
    }
}
