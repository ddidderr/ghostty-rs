//! ghostty-apprt-gtk: the Linux GTK4 + libadwaita application runtime
//! (ZIG/src/apprt/gtk/*). See docs/rewrite/notes/ghostty-apprt-gtk.md.

pub mod adw_version;
pub mod app;
pub mod cgroup;
pub mod class;
pub mod css;
pub mod debug_input;
pub mod ext;
#[cfg(debug_assertions)]
mod fake_version;
pub mod flatpak;
pub mod gl;
pub mod gsettings;
pub mod gtk_version;
pub mod ipc;
pub mod key;
pub mod post_fork;
pub mod pre_exec;
pub mod weak_ref;
pub mod winproto;

/// GTK application ID (Zig: `App.application_id`).
pub const APPLICATION_ID: &str = if cfg!(debug_assertions) {
    "com.mitchellh.ghostty-debug"
} else {
    "com.mitchellh.ghostty"
};

/// GTK object path (Zig: `App.object_path`).
pub const OBJECT_PATH: &str = if cfg!(debug_assertions) {
    "/com/mitchellh/ghostty_debug"
} else {
    "/com/mitchellh/ghostty"
};

/// Resource prefix of the bundled gresource.
pub const RESOURCE_PREFIX: &str = "/com/mitchellh/ghostty";

/// Register the compiled gresource bundle (UI files, CSS, icons). Must be
/// called before any template class is instantiated. Idempotent.
pub fn register_resources() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        gtk::gio::resources_register_include!("ghostty.gresource")
            .expect("failed to register ghostty gresource");
    });
}
