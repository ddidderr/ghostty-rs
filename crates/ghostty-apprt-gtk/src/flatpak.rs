//! Flatpak helpers (port of `apprt/gtk/flatpak.zig`).

use ghostty_os::{ResourcesDir, osbytes::EncodedOs};
use gtk::glib;

/// Returns the resources directory, consulting the Flatpak runtime data
/// when running inside a Flatpak (host path discovery).
pub fn resources_dir() -> std::io::Result<ResourcesDir> {
    // Only consult Flatpak runtime data for the host case.
    if ghostty_os::is_flatpak() {
        let mut result = ResourcesDir {
            app_path: Some(EncodedOs::from_plain_text("/app/share/ghostty")),
            host_path: None,
        };

        let keyfile = glib::KeyFile::new();
        if keyfile
            .load_from_file("/.flatpak-info", glib::KeyFileFlags::NONE)
            .is_err()
        {
            return Ok(result);
        }
        let Ok(app_dir) = keyfile.string("Instance", "app-path") else {
            return Ok(result);
        };

        result.host_path = Some(EncodedOs::from_path(
            &std::path::Path::new(app_dir.as_str())
                .join("share")
                .join("ghostty"),
        ));
        return Ok(result);
    }

    ghostty_os::resources_dir()
}
