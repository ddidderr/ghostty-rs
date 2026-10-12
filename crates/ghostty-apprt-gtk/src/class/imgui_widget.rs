//! Port of apprt/gtk/class/imgui_widget.zig (`GhosttyImguiWidget`).
//!
//! A widget for embedding a Dear ImGui application: a `GtkGLArea` that owns
//! an ImGui context and a glow based renderer, feeds the GTK input events
//! into ImGui and drives the frames.
//!
//! The Zig widget exposes `setup` and `render` virtual methods that subclasses
//! implement. zig-gobject cannot define virtual methods, and neither can we
//! cheaply in gtk-rs without a lot of class struct boilerplate, so subclasses
//! install the two hooks with [`ImguiWidget::set_hooks`] instead.

use std::{
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};

use adw::{prelude::*, subclass::prelude::*};
use dear_imgui_glow::GlowRenderer;
use ghostty_inspector::imgui::{Context, MouseButton, Ui};
use gtk::{gdk, glib, glib::translate::IntoGlib};

use crate::key;

/// Called once after the ImGui context is created, with the context current
/// (Zig virtual method `setup`).
pub type SetupHook = Box<dyn Fn(&mut Context, f64)>;

/// Called for every frame to draw the UI (Zig virtual method `render`).
pub type RenderHook = Box<dyn Fn(&Ui)>;

glib::wrapper! {
    pub struct ImguiWidget(ObjectSubclass<imp::ImguiWidget>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// Implemented by subclasses of [`ImguiWidget`].
pub trait ImguiWidgetImpl: BinImpl {}

#[allow(unsafe_code)]
// SAFETY: `ImguiWidget` adds no class/instance data that needs to be set up
// for subclasses; we forward to the parent (`adw::Bin`) class/instance
// initialization, which is valid for any `T: ImguiWidgetImpl`.
unsafe impl<T: ImguiWidgetImpl> IsSubclassable<T> for ImguiWidget {
    fn class_init(class: &mut glib::Class<Self>) {
        <adw::Bin as IsSubclassable<T>>::class_init(class.upcast_ref_mut());
    }

    fn instance_init(instance: &mut glib::subclass::InitializingObject<T>) {
        <adw::Bin as IsSubclassable<T>>::instance_init(instance);
    }
}

impl ImguiWidget {
    /// This should be called anytime the underlying data for the UI changes
    /// so that the UI can be refreshed.
    pub fn queue_render(&self) {
        self.imp().gl_area.queue_render();
    }

    /// Install the hooks (Zig: the `setup` and `render` virtual methods).
    pub fn set_hooks(&self, setup: SetupHook, render: RenderHook) {
        let imp = self.imp();
        imp.setup_hook.replace(Some(setup));
        imp.render_hook.replace(Some(render));
    }

    /// Debug aid: write the next rendered frame to a PNG file.
    pub fn debug_screenshot(&self, path: &str) {
        self.debug_render_now(3);
        self.imp().pending_screenshot.replace(Some(path.to_owned()));
        self.debug_render_now(1);
    }

    /// Debug aid: render `frames` frames right now. Compositors only send
    /// frame callbacks to visible windows, so the GLArea of an inspector
    /// window that is not on screen would otherwise lag behind the
    /// synthesized input.
    pub fn debug_render_now(&self, frames: usize) {
        let imp = self.imp();
        if !imp.gl_area.is_realized() {
            return;
        }
        imp.gl_area.make_current();
        imp.gl_area.attach_buffers();
        for _ in 0..frames {
            imp.do_render();
        }
    }

    /// Debug aid: synthesize a left click at the given position (in
    /// logical pixels, like the pointer position GTK reports). Frames are
    /// rendered synchronously between the events.
    pub fn debug_click(&self, x: f64, y: f64) {
        let imp = self.imp();
        let scale = imp.scale_factor();
        let pos = [(x * scale) as f32, (y * scale) as f32];
        imp.with_ctx(|ctx| ctx.io_mut().add_mouse_pos_event(pos));
        self.debug_render_now(3);
        imp.with_ctx(|ctx| {
            ctx.io_mut().add_mouse_button_event(MouseButton::Left, true);
        });
        self.debug_render_now(3);
        imp.with_ctx(|ctx| {
            ctx.io_mut()
                .add_mouse_button_event(MouseButton::Left, false);
        });
        self.debug_render_now(3);
    }

    /// Debug aid: synthesize a mouse wheel scroll.
    pub fn debug_scroll(&self, x: f64, y: f64, dy: f64) {
        let imp = self.imp();
        let scale = imp.scale_factor();
        imp.with_ctx(|ctx| {
            let io = ctx.io_mut();
            io.add_mouse_pos_event([(x * scale) as f32, (y * scale) as f32]);
            io.add_mouse_wheel_event([0.0, dy as f32]);
        });
        self.debug_render_now(3);
    }
}

mod imp {
    #[allow(clippy::wildcard_imports)]
    use super::*;

    /// The state that only exists while the GLArea is realized.
    struct Gui {
        ctx: Context,
        renderer: GlowRenderer,
    }

    #[derive(gtk::CompositeTemplate)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.5/imgui-widget.ui")]
    pub struct ImguiWidget {
        /// GL area where we display the Dear ImGui application.
        #[template_child]
        pub gl_area: TemplateChild<gtk::GLArea>,

        /// GTK input method context
        #[template_child]
        pub im_context: TemplateChild<gtk::IMMulticontext>,

        pub setup_hook: RefCell<Option<SetupHook>>,
        pub render_hook: RefCell<Option<RenderHook>>,

        /// Dear ImGui context and renderer. We create a context per widget
        /// so that we can have multiple active imgui views in the same
        /// application.
        gui: RefCell<Option<Gui>>,

        /// Our previous instant used to calculate delta time for animations.
        instant: Cell<Option<Instant>>,

        /// Tick callback for timed updates.
        tick_callback: RefCell<Option<gtk::TickCallbackId>>,

        /// Last render time for throttling to 30 FPS.
        last_render_time: Cell<Option<Instant>>,

        /// A pending debug screenshot path.
        pub pending_screenshot: RefCell<Option<String>>,

        /// The scale factor the style was last scaled to.
        style_scale: Cell<f32>,
    }

    impl Default for ImguiWidget {
        fn default() -> Self {
            Self {
                gl_area: TemplateChild::default(),
                im_context: TemplateChild::default(),
                setup_hook: RefCell::new(None),
                render_hook: RefCell::new(None),
                gui: RefCell::new(None),
                instant: Cell::new(None),
                tick_callback: RefCell::new(None),
                last_render_time: Cell::new(None),
                pending_screenshot: RefCell::new(None),
                style_scale: Cell::new(1.0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImguiWidget {
        const NAME: &'static str = "GhosttyImguiWidget";
        type Type = super::ImguiWidget;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ImguiWidget {
        fn dispose(&self) {
            self.dispose_template();
        }
    }

    impl WidgetImpl for ImguiWidget {}
    impl BinImpl for ImguiWidget {}

    impl ImguiWidget {
        /// Run `f` with the ImGui context, if there is one.
        pub(super) fn with_ctx(&self, f: impl FnOnce(&mut Context)) -> bool {
            let Ok(mut gui) = self.gui.try_borrow_mut() else {
                return false;
            };
            let Some(gui) = gui.as_mut() else {
                log::warn!("Dear ImGui context not initialized");
                return false;
            };
            f(&mut gui.ctx);
            true
        }

        pub(super) fn scale_factor(&self) -> f64 {
            f64::from(self.gl_area.scale_factor())
        }

        /// Handle key press/release events.
        fn key_event(
            &self,
            press: bool,
            ec_key: &gtk::EventControllerKey,
            keyval: gdk::Key,
            gtk_mods: gdk::ModifierType,
        ) -> bool {
            self.obj().queue_render();

            let handled = self.with_ctx(|ctx| {
                let io = ctx.io_mut();
                let mods = key::translate_mods(gtk_mods);
                io.add_key_event(ghostty_inspector::imgui::Key::LeftShift, mods.shift);
                io.add_key_event(ghostty_inspector::imgui::Key::LeftCtrl, mods.ctrl);
                io.add_key_event(ghostty_inspector::imgui::Key::LeftAlt, mods.alt);
                io.add_key_event(ghostty_inspector::imgui::Key::LeftSuper, mods.super_);

                // If our keyval has a key, then we send that key event
                if let Some(imgui_key) = key::key_from_keyval(keyval.into_glib())
                    .and_then(ghostty_inspector::keys::imgui_key)
                {
                    io.add_key_event(imgui_key, press);
                }
            });
            if !handled {
                return false;
            }

            // Try to process the event as text
            if let Some(event) = ec_key.current_event() {
                self.im_context.filter_keypress(&event);
            }

            true
        }

        /// Translate a GTK mouse button to a Dear ImGui mouse button.
        fn translate_mouse_button(button: u32) -> Option<MouseButton> {
            match button {
                1 => Some(MouseButton::Left),
                2 => Some(MouseButton::Middle),
                3 => Some(MouseButton::Right),
                _ => None,
            }
        }

        fn mouse_button(&self, gesture: &gtk::GestureClick, down: bool) {
            self.obj().queue_render();
            let button = gesture.current_button();
            self.with_ctx(|ctx| {
                if let Some(button) = Self::translate_mouse_button(button) {
                    ctx.io_mut().add_mouse_button_event(button, down);
                }
            });
        }

        /// Draw a frame, optionally rendering it.
        fn frame(&self, gui: &mut Gui, render: bool) -> Result<(), String> {
            // Determine our delta time
            let now = Instant::now();
            let delta = self.instant.get().map_or(1.0 / 60.0, |prev| {
                now.duration_since(prev).as_secs_f32().max(0.00001)
            });
            self.instant.set(Some(now));
            gui.ctx.io_mut().set_delta_time(delta);

            {
                let ui = gui.ctx.frame();
                // Call the hook to draw the UI.
                if let Some(hook) = self.render_hook.borrow().as_ref() {
                    hook(ui);
                }
            }

            if !render {
                gui.ctx.end_frame();
                return Ok(());
            }

            let consumer = gui
                .renderer
                .renderer_consumer()
                .map_err(|e| e.to_string())?;
            let pending = gui.ctx.render(consumer);
            gui.renderer.render(pending).map_err(|e| e.to_string())
        }

        #[allow(unsafe_code)]
        fn clear(gui: &Gui) {
            use glow::HasContext;
            let Some(gl) = gui.renderer.gl_context() else {
                return;
            };
            // SAFETY: the GLArea context is current while rendering.
            unsafe {
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
        }

        #[allow(unsafe_code)]
        fn screenshot(&self, gui: &Gui, path: &str) {
            use glow::HasContext;
            let Some(gl) = gui.renderer.gl_context() else {
                return;
            };
            let w = self.gl_area.width() * self.gl_area.scale_factor();
            let h = self.gl_area.height() * self.gl_area.scale_factor();
            if w <= 0 || h <= 0 {
                return;
            }
            let (w, h) = (w as usize, h as usize);
            let mut pixels = vec![0u8; w * h * 4];
            // SAFETY: the GLArea context is current while rendering and the
            // buffer is large enough for `w * h` RGBA8 pixels.
            unsafe {
                gl.read_pixels(
                    0,
                    0,
                    w as i32,
                    h as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut pixels)),
                );
            }
            // GL's origin is the bottom-left.
            let stride = w * 4;
            let mut flipped = Vec::with_capacity(pixels.len());
            for row in pixels.chunks_exact(stride).rev() {
                flipped.extend_from_slice(row);
            }
            let texture = gdk::MemoryTexture::new(
                w as i32,
                h as i32,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_owned(flipped),
                stride,
            );
            match texture.save_to_png(path) {
                Ok(()) => log::warn!("inspector screenshot written to {path}"),
                Err(err) => log::error!("inspector screenshot failed path={path} err={err}"),
            }
        }
    }

    impl ImguiWidget {
        #[allow(unsafe_code)]
        fn do_realize(&self, area: &gtk::GLArea) {
            area.make_current();
            if let Some(err) = area.error() {
                log::warn!(
                    "GLArea for Dear ImGui widget failed to realize: {}",
                    err.message()
                );
                return;
            }

            let mut ctx = match Context::try_create() {
                Ok(ctx) => ctx,
                Err(err) => {
                    log::warn!("unable to initialize Dear ImGui context: {err}");
                    return;
                }
            };

            // Setup some basic config
            if let Err(err) = ctx.set_platform_name(Some("ghostty_gtk")) {
                log::warn!("unable to set the imgui platform name err={err}");
            }

            // Call the hook to setup the UI. This happens before the
            // renderer is created so the fonts exist when it uploads the
            // atlas.
            if let Some(hook) = self.setup_hook.borrow().as_ref() {
                hook(&mut ctx, self.scale_factor());
            }

            // Realize means that our OpenGL context is ready, so we can now
            // initialize the ImGui OpenGL backend for our context.
            //
            // SAFETY: the GLArea's context is current and `proc_address`
            // resolves GL functions for the current context.
            let gl = unsafe { glow::Context::from_loader_function(crate::gl::proc_address) };
            let renderer = match GlowRenderer::new(gl, &mut ctx) {
                Ok(r) => r,
                Err(err) => {
                    log::warn!("unable to initialize the Dear ImGui OpenGL renderer: {err}");
                    return;
                }
            };

            self.style_scale.set(1.0);
            self.gui.replace(Some(Gui { ctx, renderer }));

            // Add a tick callback to drive timed updates via the frame clock.
            // Redraws are limited to 30 FPS max since our imgui widgets don't
            // usually need higher frame rates than that.
            let obj = self.obj();
            let id = obj.add_tick_callback(|widget, _| {
                let imp = widget.imp();
                let frame_time = Duration::from_secs(1) / 30;
                let should_render = imp
                    .last_render_time
                    .get()
                    .is_none_or(|last| last.elapsed() >= frame_time);
                if should_render {
                    widget.queue_render();
                }
                glib::ControlFlow::Continue
            });
            self.tick_callback.replace(Some(id));
        }

        fn do_unrealize(&self, area: &gtk::GLArea) {
            // Remove the tick callback if it was registered.
            if let Some(id) = self.tick_callback.take() {
                id.remove();
            }

            // Unrealize is not guaranteed to be called with a current GL
            // context, so we make it current for ImGui cleanup.
            area.make_current();
            if let Some(err) = area.error() {
                log::warn!(
                    "GLArea for Dear ImGui widget failed to realize: {}",
                    err.message()
                );
                return;
            }

            if let Some(mut gui) = self.gui.take()
                && let Err(err) = gui.renderer.shutdown(&mut gui.ctx)
            {
                log::warn!("error shutting down the imgui renderer err={err}");
            }
        }

        fn do_resize(&self, width: i32, height: i32, area: &gtk::GLArea) {
            let scale_factor = area.scale_factor();
            self.with_ctx(|ctx| {
                // Our display size is always unscaled. We'll do the scaling
                // in the style instead. This creates crisper looking fonts.
                let io = ctx.io_mut();
                io.set_display_size([width as f32, height as f32]);
                io.set_display_framebuffer_scale([1.0, 1.0]);

                // Scale the style appropriately. Zig builds a fresh default
                // style and scales it on every resize, the style of a
                // Dear ImGui context cannot be rebuilt from Rust so we scale
                // by the change in the factor instead.
                let scale = scale_factor as f32;
                let last = self.style_scale.replace(scale);
                if (scale - last).abs() > f32::EPSILON {
                    ctx.style_mut().scale_all_sizes(scale / last);
                }
            });
        }

        pub(super) fn do_render(&self) -> bool {
            // Update last render time for tick callback throttling.
            self.last_render_time.set(Some(Instant::now()));

            let Ok(mut guard) = self.gui.try_borrow_mut() else {
                return false;
            };
            let Some(gui) = guard.as_mut() else {
                return false;
            };

            // Setup our frame. We render twice because some ImGui behaviors
            // take multiple renders to process. I don't know how to make this
            // more efficient.
            if let Err(err) = self.frame(gui, false) {
                log::warn!("imgui frame failed err={err}");
                return false;
            }

            // OpenGL final render
            Self::clear(gui);
            if let Err(err) = self.frame(gui, true) {
                log::warn!("imgui render failed err={err}");
                return false;
            }

            if let Some(path) = self.pending_screenshot.take() {
                self.screenshot(gui, &path);
            }

            true
        }

        fn do_focus_enter(&self, _ec: &gtk::EventControllerFocus) {
            self.with_ctx(|ctx| ctx.io_mut().add_focus_event(true));
            self.obj().queue_render();
        }

        fn do_focus_leave(&self, _ec: &gtk::EventControllerFocus) {
            self.with_ctx(|ctx| ctx.io_mut().add_focus_event(false));
            self.obj().queue_render();
        }

        fn do_key_pressed(
            &self,
            keyval: gdk::Key,
            _keycode: u32,
            state: gdk::ModifierType,
            ec: &gtk::EventControllerKey,
        ) -> glib::Propagation {
            if self.key_event(true, ec, keyval, state) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }

        fn do_key_released(
            &self,
            keyval: gdk::Key,
            _keycode: u32,
            state: gdk::ModifierType,
            ec: &gtk::EventControllerKey,
        ) {
            self.key_event(false, ec, keyval, state);
        }

        fn do_mouse_pressed(&self, _n: i32, _x: f64, _y: f64, gesture: &gtk::GestureClick) {
            self.mouse_button(gesture, true);
        }

        fn do_mouse_released(&self, _n: i32, _x: f64, _y: f64, gesture: &gtk::GestureClick) {
            self.mouse_button(gesture, false);
        }

        fn do_mouse_motion(&self, x: f64, y: f64, _ec: &gtk::EventControllerMotion) {
            self.obj().queue_render();
            let scale = self.scale_factor();
            self.with_ctx(|ctx| {
                ctx.io_mut()
                    .add_mouse_pos_event([(x * scale) as f32, (y * scale) as f32]);
            });
        }

        fn do_scroll(&self, x: f64, y: f64, _ec: &gtk::EventControllerScroll) -> glib::Propagation {
            self.obj().queue_render();
            let handled = self.with_ctx(|ctx| {
                ctx.io_mut().add_mouse_wheel_event([x as f32, -y as f32]);
            });
            if handled {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }

        fn do_im_commit(&self, text: &str, _im: &gtk::IMMulticontext) {
            self.obj().queue_render();
            self.with_ctx(|ctx| ctx.io_mut().add_input_characters_utf8(text));
        }
    }

    #[gtk::template_callbacks]
    impl ImguiWidget {
        #[template_callback]
        fn realize(area: &gtk::GLArea, this: &super::ImguiWidget) {
            this.imp().do_realize(area);
        }

        #[template_callback]
        fn unrealize(area: &gtk::GLArea, this: &super::ImguiWidget) {
            this.imp().do_unrealize(area);
        }

        #[template_callback]
        fn resize(area: &gtk::GLArea, width: i32, height: i32, this: &super::ImguiWidget) {
            this.imp().do_resize(width, height, area);
        }

        #[template_callback]
        fn render(area: &gtk::GLArea, context: &gdk::GLContext, this: &super::ImguiWidget) -> bool {
            let _ = (area, context);
            this.imp().do_render()
        }

        #[template_callback]
        fn focus_enter(ec: &gtk::EventControllerFocus, this: &super::ImguiWidget) {
            this.imp().do_focus_enter(ec);
        }

        #[template_callback]
        fn focus_leave(ec: &gtk::EventControllerFocus, this: &super::ImguiWidget) {
            this.imp().do_focus_leave(ec);
        }

        #[template_callback]
        fn key_pressed(
            ec: &gtk::EventControllerKey,
            keyval: gdk::Key,
            keycode: u32,
            state: gdk::ModifierType,
            this: &super::ImguiWidget,
        ) -> glib::Propagation {
            this.imp().do_key_pressed(keyval, keycode, state, ec)
        }

        #[template_callback]
        fn key_released(
            ec: &gtk::EventControllerKey,
            keyval: gdk::Key,
            keycode: u32,
            state: gdk::ModifierType,
            this: &super::ImguiWidget,
        ) {
            this.imp().do_key_released(keyval, keycode, state, ec);
        }

        #[template_callback]
        fn mouse_pressed(
            gesture: &gtk::GestureClick,
            n: i32,
            x: f64,
            y: f64,
            this: &super::ImguiWidget,
        ) {
            this.imp().do_mouse_pressed(n, x, y, gesture);
        }

        #[template_callback]
        fn mouse_released(
            gesture: &gtk::GestureClick,
            n: i32,
            x: f64,
            y: f64,
            this: &super::ImguiWidget,
        ) {
            this.imp().do_mouse_released(n, x, y, gesture);
        }

        #[template_callback]
        fn mouse_motion(
            ec: &gtk::EventControllerMotion,
            x: f64,
            y: f64,
            this: &super::ImguiWidget,
        ) {
            this.imp().do_mouse_motion(x, y, ec);
        }

        #[template_callback]
        fn scroll(
            ec: &gtk::EventControllerScroll,
            x: f64,
            y: f64,
            this: &super::ImguiWidget,
        ) -> glib::Propagation {
            this.imp().do_scroll(x, y, ec)
        }

        #[template_callback]
        fn im_commit(im: &gtk::IMMulticontext, text: &str, this: &super::ImguiWidget) {
            this.imp().do_im_commit(text, im);
        }
    }
}
