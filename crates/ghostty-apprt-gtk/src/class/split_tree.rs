//! Port of apprt/gtk/class/split_tree.zig: `GhosttySplitTree` and the
//! internal `GhosttySplitTreeSplit` widget.
//!
//! The Zig `tree` property is a boxed pointer that is mutated in place for
//! zoom/resize. Here the tree lives in a `RefCell<Option<Tree>>` inside the
//! widget and is exposed as a [`SurfaceTree`] boxed value (a clone) through
//! the `tree` property; in-place mutation goes through private helpers.

#![allow(clippy::wildcard_imports)]

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use ghostty_datastruct::split_tree::{
    Direction, Goto, Handle, Layout, Node, SplitError, SplitTree as DsSplitTree, SplitTreeView,
};
use gtk::gio;
use gtk::glib::{self, clone, subclass::Signal};
use half::f16;

use super::application::Application;
use super::close_confirmation_dialog::{CloseConfirmationDialog, Target as CloseTarget};
use super::surface::{Surface, SurfaceOverrides};
use super::surface_scrolled_window::SurfaceScrolledWindow;

/// The split tree of surfaces (Zig: `Surface.Tree`).
pub type Tree = DsSplitTree<Surface>;

/// Boxed GObject wrapper for [`Tree`] (Zig: `Surface.Tree.getGObjectType`).
#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "GhosttySurfaceTree", nullable)]
pub struct SurfaceTree(pub Tree);

impl SplitTreeView for Surface {}

/// Find the first ancestor of `widget` of type `T` (Zig: `ext.getAncestor`).
fn get_ancestor<T: IsA<gtk::Widget>>(widget: &impl IsA<gtk::Widget>) -> Option<T> {
    widget
        .ancestor(T::static_type())
        .and_then(|w| w.downcast::<T>().ok())
}

glib::wrapper! {
    /// A tree of surfaces rendered as nested `GtkPaned`s.
    pub struct SplitTree(ObjectSubclass<imp::SplitTree>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for SplitTree {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl SplitTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new split in the given direction from the currently active
    /// surface. If the tree is empty this creates a new tree with a new
    /// surface and ignores the direction. `parent` is used to inherit
    /// surface properties regardless of whether it is in this tree.
    pub fn new_split(
        &self,
        direction: Direction,
        parent: Option<&Surface>,
        overrides: SurfaceOverrides,
    ) -> Result<(), SplitError> {
        let imp = self.imp();

        // Create our new surface.
        let surface = Surface::new(overrides);

        // Inherit properly if we were asked to.
        if let Some(p) = parent {
            surface.set_parent(p, ghostty_apprt::surface::NewSurfaceContext::Split);
        }

        // Bind is-split property for new surface
        self.bind_property("is-split", &surface, "is-split")
            .sync_create()
            .build();

        let single_tree = Tree::new(&surface);

        // We want to move our focus to the new surface no matter what.
        // But we need to be careful to restore state if we fail.
        let old_last_focused = imp.last_focused.upgrade();
        imp.last_focused.set(Some(&surface));

        // If we have no tree yet, then this becomes our tree and we're done.
        let Some(old_tree) = self.get_tree() else {
            self.set_tree(Some(&single_tree));
            return Ok(());
        };

        // The handle we create the split relative to. Today this is the
        // active surface but this might be the handle of the given parent.
        let handle = self.get_active_surface_handle().unwrap_or(Handle::ROOT);

        let new_tree = match old_tree.split(handle, direction, f16::from_f32(0.5), &single_tree) {
            Ok(t) => t,
            Err(e) => {
                imp.last_focused.set(old_last_focused.as_ref());
                return Err(e);
            }
        };
        log::debug!(
            "new split at={handle:?} direction={direction:?} old_tree={old_tree} new_tree={new_tree}"
        );

        self.set_tree(Some(&new_tree));
        Ok(())
    }

    /// Resize the active surface's split by `amount` pixels in `direction`.
    pub fn resize(&self, direction: Direction, amount: u16) -> Result<bool, SplitError> {
        // Avoid useless work
        if amount == 0 {
            return Ok(false);
        }

        let Some(old_tree) = self.get_tree() else {
            return Ok(false);
        };
        let Some(active) = self.get_active_surface_handle() else {
            return Ok(false);
        };

        let bin = self.imp().tree_bin.get();
        let width = bin.width();
        let height = bin.height();
        if width == 0 || height == 0 {
            return Ok(false);
        }
        let width_f64 = f64::from(width);
        let height_f64 = f64::from(height);
        let amount_f64 = f64::from(amount);

        let ratio = match direction {
            Direction::Right => amount_f64 / width_f64,
            Direction::Left => -(amount_f64 / width_f64),
            Direction::Down => amount_f64 / height_f64,
            Direction::Up => -(amount_f64 / height_f64),
        };
        let layout = match direction {
            Direction::Left | Direction::Right => Layout::Horizontal,
            Direction::Up | Direction::Down => Layout::Vertical,
        };

        let new_tree = old_tree.resize(active, layout, f16::from_f64(ratio));
        self.set_tree(Some(&new_tree));
        Ok(true)
    }

    /// Move focus from the currently focused surface in the given
    /// direction. Returns true if focus switched to a new surface.
    pub fn goto(&self, to: Goto) -> bool {
        let imp = self.imp();
        let Some(tree) = self.get_tree() else {
            return false;
        };
        let Some(active) = self.get_active_surface_handle() else {
            return false;
        };
        let Some(target) = tree.goto(active, to) else {
            return false;
        };

        // If we aren't changing targets then we did nothing.
        if active == target {
            return false;
        }

        // Get the surface at the target location and grab focus.
        let Node::Leaf(surface) = &tree.nodes()[target.idx()] else {
            return false;
        };
        surface.grab_focus();

        // We also need to setup our last_focused to this because if we
        // trigger a tree change like below, the grab focus above never
        // actually triggers in time to set this and this ensures we
        // grab focus to the right thing.
        imp.last_focused.set(Some(surface));

        if tree.zoomed().is_some() {
            let config_obj = Application::default().get_config();
            let config = config_obj.get();

            {
                let mut guard = imp.tree.borrow_mut();
                if let Some(t) = guard.as_mut() {
                    if config.split_preserve_zoom.navigation {
                        t.zoom(Some(target));
                    } else {
                        t.zoom(None);
                    }
                }
            }

            // When the zoom state changes our tree state changes and we
            // need to send the proper notifications to trigger relayout.
            self.notify("tree");
            self.notify("is-zoomed");
        }

        true
    }

    /// Returns true if this split tree needs confirmation before quitting
    /// based on the various Ghostty configurations.
    pub fn get_needs_confirm_quit(&self) -> bool {
        let Some(tree) = self.get_tree() else {
            return false;
        };
        tree.iterator()
            .any(|e| e.view.core().is_some_and(|c| c.needs_confirm_quit()))
    }

    /// Get the currently active surface. See the "active-surface" property.
    pub fn get_active_surface(&self) -> Option<Surface> {
        let tree = self.get_tree()?;
        let handle = self.get_active_surface_handle()?;
        match &tree.nodes()[handle.idx()] {
            Node::Leaf(s) => Some(s.clone()),
            Node::Split(_) => None,
        }
    }

    fn get_active_surface_handle(&self) -> Option<Handle> {
        let imp = self.imp();
        let guard = imp.tree.borrow();
        let tree = guard.as_ref()?;
        if let Some(e) = tree.iterator().find(|e| e.view.property::<bool>("focused")) {
            return Some(e.handle);
        }

        // If none are currently focused, the most previously focused
        // surface (if it exists) is our active surface. This lets things
        // like apprt actions and bell ringing continue to work in the
        // background.
        let last = imp.last_focused.upgrade()?;
        tree.iterator().find(|e| *e.view == last).map(|e| e.handle)
    }

    /// Returns the last focused surface in the tree.
    pub fn get_last_focused_surface(&self) -> Option<Surface> {
        self.imp().last_focused.upgrade()
    }

    pub fn get_has_surfaces(&self) -> bool {
        self.imp()
            .tree
            .borrow()
            .as_ref()
            .is_some_and(|t| !t.is_empty())
    }

    pub fn get_is_zoomed(&self) -> bool {
        self.imp()
            .tree
            .borrow()
            .as_ref()
            .is_some_and(|t| t.zoomed().is_some())
    }

    pub fn get_is_split(&self) -> bool {
        self.imp()
            .tree
            .borrow()
            .as_ref()
            .is_some_and(Tree::is_split)
    }

    /// Get the tree data model shown in this widget. This returns a clone
    /// (views are reference counted); mutations must go through
    /// [`SplitTree::set_tree`].
    pub fn get_tree(&self) -> Option<Tree> {
        self.imp().tree.borrow().clone()
    }

    /// Set the tree data model shown in this widget (clones the tree).
    /// Empty trees are normalized to `None`.
    pub fn set_tree(&self, tree: Option<&Tree>) {
        let imp = self.imp();

        let tree = tree.filter(|t| !t.is_empty());

        // Emit the signal so that handlers can witness both the before and
        // after values of the tree.
        let old = imp.tree.borrow().clone().map(SurfaceTree);
        let new = tree.cloned().map(SurfaceTree);
        self.emit_by_name::<()>("changed", &[&old, &new]);

        if imp.tree.borrow().is_some() {
            self.disconnect_surface_handlers();
            imp.tree.replace(None);
        }

        if let Some(new_tree) = tree {
            debug_assert!(imp.tree.borrow().is_none());
            imp.tree.replace(Some(new_tree.clone()));
            self.connect_surface_handlers();
        }

        self.notify("tree");
    }

    /// Update a split ratio in place (used by `SplitTreeSplit`).
    fn resize_in_place(&self, handle: Handle, ratio: f16) {
        if let Some(t) = self.imp().tree.borrow_mut().as_mut() {
            t.resize_in_place(handle, ratio);
        }
    }

    fn disconnect_surface_handlers(&self) {
        for (surface, ids) in self.imp().handlers.take() {
            if let Some(s) = surface.upgrade() {
                for id in ids {
                    s.disconnect(id);
                }
            }
        }
    }

    fn connect_surface_handlers(&self) {
        let Some(tree) = self.get_tree() else { return };
        let mut out = Vec::new();
        for entry in tree.iterator() {
            let surface = entry.view;
            let id1 = surface.connect_local(
                "close-request",
                false,
                clone!(
                    #[weak(rename_to = this)]
                    self,
                    #[weak]
                    surface,
                    #[upgrade_or]
                    None,
                    move |_| {
                        this.surface_close_request(&surface);
                        None
                    }
                ),
            );
            let id2 = surface.connect_notify_local(
                Some("focused"),
                clone!(
                    #[weak(rename_to = this)]
                    self,
                    move |surface, _| this.prop_surface_focused(surface)
                ),
            );
            out.push((surface.downgrade(), vec![id1, id2]));
        }
        self.imp().handlers.replace(out);
    }

    //---------------------------------------------------------------
    // Signal handlers

    fn surface_close_request(&self, surface: &Surface) {
        let Some(core) = surface.core() else { return };
        let imp = self.imp();

        // Reset our pending close state
        imp.pending_close.set(None);

        // Find the surface in the tree to verify this is valid and set
        // our pending close handle.
        let handle = {
            let guard = imp.tree.borrow();
            let Some(tree) = guard.as_ref() else { return };
            let Some(e) = tree.iterator().find(|e| e.view == surface) else {
                return;
            };
            e.handle
        };
        imp.pending_close.set(Some(handle));

        // If we don't need to confirm then just close immediately.
        if !core.needs_confirm_quit() {
            self.close_confirmation_close();
            return;
        }

        // Show a confirmation dialog
        let dialog = CloseConfirmationDialog::new(CloseTarget::Surface);
        dialog.connect_close_request(clone!(
            #[weak(rename_to = this)]
            self,
            move |_| this.close_confirmation_close()
        ));
        dialog.present(Some(self.upcast_ref::<gtk::Widget>()));
    }

    fn close_confirmation_close(&self) {
        // Get the handle we're closing
        let imp = self.imp();
        let Some(handle) = imp.pending_close.take() else {
            return;
        };

        // Figure out our next focus target. The next focus target is
        // always the "previous" surface unless we're the leftmost then
        // its the next.
        let Some(old_tree) = self.get_tree() else {
            return;
        };
        let next_focus: Option<Surface> = (|| {
            let next_handle = old_tree
                .goto(handle, Goto::Previous)
                .or_else(|| old_tree.goto(handle, Goto::Next))?;
            if next_handle == handle {
                return None;
            }
            match &old_tree.nodes()[next_handle.idx()] {
                Node::Leaf(s) => Some(s.clone()),
                Node::Split(_) => None,
            }
        })();

        // Remove it from the tree.
        let new_tree = old_tree.remove(handle);
        self.set_tree(Some(&new_tree));

        // Grab focus. We have to set this on the "last focused" because our
        // focus will be set when the tree is redrawn.
        if let Some(v) = next_focus {
            imp.last_focused.set(Some(&v));
        }
    }

    fn prop_surface_focused(&self, surface: &Surface) {
        // We never CLEAR our last_focused because the property is
        // specifically the last focused surface. The weakref clears itself
        // when the surface is destroyed.
        if !surface.property::<bool>("focused") {
            return;
        }
        self.imp().last_focused.set(Some(surface));

        // Our active surface probably changed
        self.notify("active-surface");
    }

    fn prop_tree(&self) {
        let imp = self.imp();

        // No matter what we notify
        let _freeze = self.freeze_notify();
        self.notify("has-surfaces");
        self.notify("is-zoomed");

        // If we were planning a rebuild, always remove that so we can
        // start from a clean slate.
        if let Some(id) = imp.rebuild_source.take() {
            id.remove();
        }

        // If we transitioned to an empty tree, clear immediately instead of
        // waiting for an idle callback.
        if imp.tree.borrow().is_none() {
            imp.tree_bin.set_child(gtk::Widget::NONE);
            return;
        }

        // Build on an idle callback so rapid tree changes are debounced.
        // We keep the existing tree attached until the rebuild runs,
        // which avoids transient empty frames.
        let id = glib::idle_add_local_full(
            glib::Priority::DEFAULT_IDLE,
            clone!(
                #[weak(rename_to = this)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    this.on_rebuild();
                    glib::ControlFlow::Break
                }
            ),
        );
        imp.rebuild_source.replace(Some(id));
    }

    fn on_rebuild(&self) {
        let imp = self.imp();

        // Always mark our rebuild source as null since we're done. (The
        // source is removed automatically by returning Break; dropping the
        // id without removing it is fine.)
        imp.rebuild_source.take();

        // Rebuild our tree
        match self.get_tree() {
            None => imp.tree_bin.set_child(gtk::Widget::NONE),
            Some(tree) => {
                let widget = Self::build_tree(&tree, tree.zoomed().unwrap_or(Handle::ROOT));
                imp.tree_bin.set_child(Some(&widget));
            }
        }

        // Replacing our tree widget hierarchy can reset focus state.
        // If we have a last-focused surface, restore focus to it.
        if let Some(v) = imp.last_focused.upgrade() {
            v.grab_focus();
        }

        // Our split status may have changed
        self.notify("is-split");

        // Our active surface may have changed
        self.notify("active-surface");
    }

    /// Builds the widget tree associated with a surface split tree.
    /// Returned widgets are expected to be attached to a parent by the
    /// caller. Reused widgets are detached from their old parent (the
    /// returned strong reference keeps them alive meanwhile).
    fn build_tree(tree: &Tree, current: Handle) -> gtk::Widget {
        match &tree.nodes()[current.idx()] {
            Node::Leaf(v) => match get_ancestor::<SurfaceScrolledWindow>(v) {
                // The surface isn't in a window already so we don't have to
                // worry about reuse.
                None => glib::Object::builder::<SurfaceScrolledWindow>()
                    .property("surface", v)
                    .build()
                    .upcast(),
                Some(window) => {
                    let w: gtk::Widget = window.upcast();
                    // Our strong ref (`w`) keeps it alive while we detach it
                    // from the old tree and adopt it into the new one.
                    detach_widget(&w);
                    w
                }
            },
            Node::Split(s) => {
                let left = Self::build_tree(tree, s.left);
                let right = Self::build_tree(tree, s.right);
                SplitTreeSplit::new(current, s.layout, &left, &right).upcast()
            }
        }
    }

    //---------------------------------------------------------------
    // Actions

    fn init_action_map(&self) {
        let group = gio::SimpleActionGroup::new();

        let new_split = gio::SimpleAction::new("new-split", Some(glib::VariantTy::STRING));
        new_split.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, param| {
                let Some(dir) = param.and_then(|p| p.str()) else {
                    log::warn!("split-tree.new-split called without a parameter");
                    return;
                };
                let direction = match dir {
                    "left" => Direction::Left,
                    "right" => Direction::Right,
                    "down" => Direction::Down,
                    "up" => Direction::Up,
                    _ => {
                        // Need to be defensive here since actions can be
                        // triggered externally.
                        log::warn!("invalid split direction for split-tree.new-split: {dir}");
                        return;
                    }
                };
                if let Err(e) = this.new_split(
                    direction,
                    this.get_active_surface().as_ref(),
                    SurfaceOverrides::default(),
                ) {
                    log::warn!("new split failed error={e}");
                }
            }
        ));
        group.add_action(&new_split);

        let equalize = gio::SimpleAction::new("equalize", None);
        equalize.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _| {
                let Some(old_tree) = this.get_tree() else {
                    return;
                };
                let new_tree = old_tree.equalize();
                this.set_tree(Some(&new_tree));
            }
        ));
        group.add_action(&equalize);

        let zoom = gio::SimpleAction::new("zoom", None);
        zoom.connect_activate(clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _| this.action_zoom()
        ));
        group.add_action(&zoom);

        self.insert_action_group("split-tree", Some(&group));
    }

    fn action_zoom(&self) {
        let imp = self.imp();
        if imp.tree.borrow().is_none() {
            return;
        }
        if self.get_is_zoomed() {
            if let Some(t) = imp.tree.borrow_mut().as_mut() {
                t.zoom(None);
            }
        } else {
            let Some(active) = self.get_active_surface_handle() else {
                return;
            };
            if let Some(t) = imp.tree.borrow_mut().as_mut() {
                if t.zoomed() == Some(active) {
                    return;
                }
                t.zoom(Some(active));
            }
        }
        self.notify("tree");
    }
}

/// Detach a widget from its current parent.
///
/// We intentionally use parent-specific child APIs when possible
/// (`GtkPaned.set_start_child/set_end_child`, `AdwBin.set_child`) instead of
/// calling `unparent` directly, since those keep container state and
/// notifications in sync.
fn detach_widget(widget: &gtk::Widget) {
    let Some(parent) = widget.parent() else {
        return;
    };

    // Surface will be in a paned when it is split.
    if let Some(paned) = parent.downcast_ref::<gtk::Paned>() {
        if paned.start_child().as_ref() == Some(widget) {
            paned.set_start_child(gtk::Widget::NONE);
            return;
        }
        if paned.end_child().as_ref() == Some(widget) {
            paned.set_end_child(gtk::Widget::NONE);
            return;
        }
    }

    // Surface will be in a bin when it is not split.
    if let Some(bin) = parent.downcast_ref::<adw::Bin>()
        && bin.child().as_ref() == Some(widget)
    {
        bin.set_child(gtk::Widget::NONE);
        return;
    }

    // Fallback for unexpected parents.
    widget.unparent();
}

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.5/split-tree.ui")]
    pub struct SplitTree {
        /// The tree datastructure containing all of our surface views.
        pub tree: RefCell<Option<Tree>>,

        #[template_child]
        pub tree_bin: TemplateChild<adw::Bin>,

        /// Last focused surface in the tree.
        pub last_focused: glib::WeakRef<Surface>,

        /// The source used to rebuild the tree (also debounces updates).
        pub rebuild_source: RefCell<Option<glib::SourceId>>,

        /// State about a pending surface close for the close dialog.
        pub pending_close: Cell<Option<Handle>>,

        /// Handlers connected on surfaces in the tree.
        pub handlers: RefCell<Vec<(glib::WeakRef<Surface>, Vec<glib::SignalHandlerId>)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SplitTree {
        const NAME: &'static str = "GhosttySplitTree";
        type Type = super::SplitTree;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            Surface::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl SplitTree {
        #[template_callback]
        fn notify_tree(&self, _pspec: glib::ParamSpec) {
            self.obj().prop_tree();
        }
    }

    impl ObjectImpl for SplitTree {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPS: std::sync::OnceLock<Vec<glib::ParamSpec>> = std::sync::OnceLock::new();
            PROPS.get_or_init(|| {
                vec![
                    // The active surface is the surface that should be
                    // receiving all surface-targeted actions.
                    glib::ParamSpecObject::builder::<Surface>("active-surface")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("has-surfaces")
                        .default_value(false)
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("is-zoomed")
                        .default_value(false)
                        .read_only()
                        .build(),
                    glib::ParamSpecBoxed::builder::<SurfaceTree>("tree").build(),
                    glib::ParamSpecBoolean::builder("is-split")
                        .default_value(false)
                        .read_only()
                        .build(),
                ]
            })
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<Signal>> = std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // Emitted whenever the tree property has changed, with
                    // access to the previous and new values.
                    Signal::builder("changed")
                        .param_types([SurfaceTree::static_type(), SurfaceTree::static_type()])
                        .build(),
                ]
            })
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();
            match pspec.name() {
                "active-surface" => obj.get_active_surface().to_value(),
                "has-surfaces" => obj.get_has_surfaces().to_value(),
                "is-zoomed" => obj.get_is_zoomed().to_value(),
                "is-split" => obj.get_is_split().to_value(),
                "tree" => obj.get_tree().map(SurfaceTree).to_value(),
                _ => unreachable!(),
            }
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "tree" => {
                    let t = value.get::<Option<SurfaceTree>>().ok().flatten();
                    self.obj().set_tree(t.as_ref().map(|t| &t.0));
                }
                _ => unreachable!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.init_action_map();
        }

        fn dispose(&self) {
            self.last_focused.set(None);
            if let Some(id) = self.rebuild_source.take() {
                id.remove();
            }
            self.dispose_template();
        }
    }

    impl WidgetImpl for SplitTree {}
    impl BoxImpl for SplitTree {}
}

glib::wrapper! {
    /// Internal widget representing a split: a `GtkPaned` wrapper handling
    /// ratio (0 to 1) based positioning and writing user adjustments back
    /// to the split tree. Immutable after creation.
    pub struct SplitTreeSplit(ObjectSubclass<split_imp::SplitTreeSplit>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SplitTreeSplit {
    fn new(handle: Handle, layout: Layout, start: &gtk::Widget, end: &gtk::Widget) -> Self {
        let this: Self = glib::Object::new();
        let imp = this.imp();
        imp.handle.set(Some(handle));

        let paned = imp.paned.get();
        paned.set_start_child(Some(start));
        paned.set_end_child(Some(end));
        paned.set_orientation(match layout {
            Layout::Horizontal => gtk::Orientation::Horizontal,
            Layout::Vertical => gtk::Orientation::Vertical,
        });
        this
    }

    fn refresh(&self) {
        let imp = self.imp();
        if imp.idle.borrow().is_none() {
            let id = glib::idle_add_local_full(
                glib::Priority::DEFAULT_IDLE,
                clone!(
                    #[weak(rename_to = this)]
                    self,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        this.on_idle();
                        glib::ControlFlow::Break
                    }
                ),
            );
            imp.idle.replace(Some(id));
        }
    }

    fn on_idle(&self) {
        let imp = self.imp();
        let paned = imp.paned.get();

        // Our idle source is always over
        imp.idle.take();

        // Get our split. We assume that this widget is always a child of a
        // SplitTree, that our handle is valid, and that it is a split node.
        let Some(split_tree) = get_ancestor::<SplitTree>(self) else {
            return;
        };
        let Some(handle) = imp.handle.get() else {
            return;
        };
        let desired_ratio = {
            let guard = split_tree.imp().tree.borrow();
            let Some(tree) = guard.as_ref() else { return };
            match &tree.nodes()[handle.idx()] {
                Node::Split(s) => f64::from(s.ratio),
                Node::Leaf(_) => return,
            }
        };

        // Current, min, and max positions as pixels.
        let pos = paned.position();
        let min: i32 = paned.property("min-position");
        let max: i32 = paned.property("max-position");
        let pos_set: bool = paned.property("position-set");

        // We don't actually use min, but we don't expect it to be non-zero.
        debug_assert_eq!(min, 0);

        // If our max is zero then we can't do any math.
        if max == 0 {
            return;
        }

        let current_ratio = f64::from(pos) / f64::from(max);

        // If our ratio is close enough to our desired ratio, then we ignore
        // the update. This avoids constant split updates for lossy math.
        if (current_ratio - desired_ratio).abs() <= 0.001 {
            return;
        }

        // If we've never set the position, then we set it to the desired.
        if !pos_set {
            let desired_pos = (f64::from(max) * desired_ratio).round() as i32;
            paned.set_position(desired_pos);
            return;
        }

        // If we've set the position, then this is a manual human update and
        // we need to write our update back to the tree.
        split_tree.resize_in_place(handle, f16::from_f64(current_ratio));
    }
}

mod split_imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.5/split-tree-split.ui")]
    pub struct SplitTreeSplit {
        /// The handle of the node in the tree that this split represents.
        pub handle: Cell<Option<Handle>>,

        /// Source to handle repositioning the split when properties change.
        pub idle: RefCell<Option<glib::SourceId>>,

        #[template_child]
        pub paned: TemplateChild<gtk::Paned>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SplitTreeSplit {
        const NAME: &'static str = "GhosttySplitTreeSplit";
        type Type = super::SplitTreeSplit;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl SplitTreeSplit {
        #[template_callback]
        fn notify_max_position(
            _paned: &gtk::Paned,
            _pspec: glib::ParamSpec,
            this: &super::SplitTreeSplit,
        ) {
            // The emitter is the child `Paned`; the template instance is the
            // trailing user-data argument.
            this.refresh();
        }

        #[template_callback]
        fn notify_min_position(
            _paned: &gtk::Paned,
            _pspec: glib::ParamSpec,
            this: &super::SplitTreeSplit,
        ) {
            // The emitter is the child `Paned`; the template instance is the
            // trailing user-data argument.
            this.refresh();
        }

        #[template_callback]
        fn notify_position(
            _paned: &gtk::Paned,
            _pspec: glib::ParamSpec,
            this: &super::SplitTreeSplit,
        ) {
            // The emitter is the child `Paned`; the template instance is the
            // trailing user-data argument.
            this.refresh();
        }
    }

    impl ObjectImpl for SplitTreeSplit {
        fn dispose(&self) {
            if let Some(id) = self.idle.take() {
                id.remove();
            }
            self.dispose_template();
        }
    }

    impl WidgetImpl for SplitTreeSplit {}
    impl BinImpl for SplitTreeSplit {}
}
