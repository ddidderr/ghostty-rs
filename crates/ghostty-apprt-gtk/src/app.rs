//! Port of `apprt/gtk/App.zig`: the apprt `App` handed to the core. It is a
//! thin wrapper around the [`Application`] GObject.
//!
//! ```ignore
//! let app = App::new(core_app);   // loads config, creates GtkApplication
//! app.run()?;                     // main loop (ticks core_app)
//! app.terminate();
//! ```

use std::{rc::Rc, sync::Arc};

use ghostty_apprt::{
    action::{Action, Target},
    ipc,
    runtime_api::{CoreApp, SurfaceId, Waker},
};

use crate::class::application::{Application, RunError};

/// This is detected by the Renderer, in which case it sends a
/// `redraw_surface` message so that we can call `drawFrame` ourselves from
/// the app thread, because GTK's `GLArea` does not support drawing from a
/// different thread.
pub const MUST_DRAW_FROM_APP_THREAD: bool = true;

/// GTK application ID (Zig: `App.application_id`).
pub use crate::APPLICATION_ID as application_id;
/// GTK object path (Zig: `App.object_path`).
pub use crate::OBJECT_PATH as object_path;

/// The [`Waker`] given to core threads: wakes the GLib main context so the
/// app loop runs `CoreApp::tick`.
#[derive(Debug, Default, Clone, Copy)]
pub struct GlibWaker;

impl Waker for GlibWaker {
    fn wake(&self) {
        gtk::glib::MainContext::default().wakeup();
    }
}

/// A new thread-safe waker (create it before the core app, hand it to core).
#[must_use]
pub fn waker() -> Arc<dyn Waker> {
    Arc::new(GlibWaker)
}

/// The apprt App.
pub struct App {
    /// The GObject Application instance.
    pub app: Application,
}

impl App {
    /// `App.init`: creates the GTK application (loads the configuration,
    /// initializes GTK/libadwaita).
    #[must_use]
    pub fn new(core_app: Rc<dyn CoreApp>) -> Self {
        Self {
            app: Application::new(core_app),
        }
    }

    /// `App.run`: run until quit.
    pub fn run(&self) -> Result<(), RunError> {
        self.app.run_loop()
    }

    /// `App.terminate`: force deinitialize the app (other things tend to
    /// still hold references, so this forces the disposal now).
    pub fn terminate(&self) {
        self.app.deinit();
    }

    /// `App.wakeup`: wake up the event loop (thread safe).
    pub fn wakeup(&self) {
        self.app.wakeup();
    }

    /// `App.performAction`.
    pub fn perform_action(&self, target: Target<SurfaceId>, action: Action) -> bool {
        self.app.perform_action(target, action)
    }

    /// `App.performIpc`: send the IPC to a running Ghostty. Static: it's
    /// called from a CLI process without a full-featured apprt App.
    pub fn perform_ipc(target: &ipc::Target, action: &ipc::Action) -> Result<bool, ipc::Error> {
        crate::ipc::perform_ipc(target, action)
    }
}
