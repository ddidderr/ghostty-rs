//! Port of apprt/gtk/class/config.zig (`GhosttyConfig`).
//!
//! Wraps a core config in a GObject so it can be reference counted and bound
//! from blueprints (`diagnostics-buffer`, `has-diagnostics`).

use std::{cell::RefCell, sync::Arc};

use ghostty_config::config::Config as CoreConfig;
use gtk::{glib, glib::subclass::prelude::*};

/// GTK accepts UTF-8, while the diagnostic formatter transports original bytes.
/// Upstream inserts each diagnostic separately, skipping invalid text but still
/// inserting its newline, and skips diagnostics exceeding its 4095-byte buffer.
fn diagnostics_text(config: &CoreConfig) -> String {
    let mut text = String::new();
    for diag in config.diagnostics.items() {
        let bytes = ghostty_os::osbytes::config_str_to_bytes(&diag.to_string());
        if bytes.len() > 4095 {
            log::warn!("diagnostic exceeds GTK buffer capacity");
            continue;
        }
        if !bytes.contains(&0)
            && let Ok(line) = std::str::from_utf8(&bytes)
        {
            text.push_str(line);
        }
        text.push('\n');
    }
    text
}

mod imp {
    use gtk::{
        glib,
        glib::{prelude::*, subclass::prelude::*},
        prelude::TextBufferExt,
    };

    use super::{Arc, CoreConfig, RefCell};

    #[derive(glib::Properties)]
    #[properties(wrapper_type = super::Config)]
    pub struct Config {
        pub config: RefCell<Arc<CoreConfig>>,

        #[property(name = "diagnostics-buffer", get = Self::diagnostics_buffer, type = Option<gtk::TextBuffer>)]
        #[property(name = "has-diagnostics", get = Self::has_diagnostics, type = bool)]
        _props: (),
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                config: RefCell::new(Arc::new(CoreConfig::new_default())),
                _props: (),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Config {
        const NAME: &'static str = "GhosttyConfig";
        type Type = super::Config;
        type ParentType = glib::Object;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Config {}

    impl Config {
        fn has_diagnostics(&self) -> bool {
            !self.config.borrow().diagnostics.empty()
        }

        fn diagnostics_buffer(&self) -> Option<gtk::TextBuffer> {
            let config = self.config.borrow();
            if config.diagnostics.empty() {
                return None;
            }
            let buf = gtk::TextBuffer::new(None);
            buf.set_text(&super::diagnostics_text(&config));
            Some(buf)
        }
    }
}

glib::wrapper! {
    /// `GhosttyConfig`: reference counted wrapper around a core config.
    pub struct Config(ObjectSubclass<imp::Config>);
}

impl Config {
    /// Create a new GhosttyConfig from a loaded configuration. This clones
    /// the given configuration.
    pub fn new(config: &CoreConfig) -> Self {
        let obj: Self = glib::Object::new();
        *obj.imp().config.borrow_mut() = Arc::new(config.clone());
        obj
    }

    /// Create a GhosttyConfig that takes ownership of an existing `Arc`.
    pub fn from_arc(config: Arc<CoreConfig>) -> Self {
        let obj: Self = glib::Object::new();
        *obj.imp().config.borrow_mut() = config;
        obj
    }

    /// Create a GhosttyConfig holding the default configuration.
    pub fn new_default() -> Self {
        Self::new(&CoreConfig::new_default())
    }

    /// Get the wrapped configuration (shared; cheap).
    pub fn get(&self) -> Arc<CoreConfig> {
        self.imp().config.borrow().clone()
    }

    /// Run `f` with mutable access to the config (clones on write if the
    /// `Arc` is shared). Changes aren't propagated to other holders.
    pub fn get_mut<R>(&self, f: impl FnOnce(&mut CoreConfig) -> R) -> R {
        let mut guard = self.imp().config.borrow_mut();
        f(Arc::make_mut(&mut guard))
    }

    /// A deep copy of the wrapped configuration.
    pub fn clone_config(&self) -> CoreConfig {
        (*self.get()).clone()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_display_original_text_and_skip_invalid_utf8() {
        let mut config = CoreConfig::default();
        config
            .load_iter(&mut ghostty_config::cli_args::LineIterator::new(
                b"font-size=\xef\xb7\x90\xc3\xa9\nfont-size=\xff\n".as_slice(),
            ))
            .expect("parse diagnostics");
        let text = diagnostics_text(&config);
        assert!(text.contains("invalid value \"\u{fdd0}é\""));
        assert!(!text.contains("\u{fdd0}\u{fdd0}"));
        assert!(text.ends_with("\n\n"));
    }

    #[test]
    fn ghostty_config() {
        // No display needed; GObject only.
        let obj = Config::new_default();
        assert!(!obj.has_diagnostics());
        assert!(obj.diagnostics_buffer().is_none());
        drop(obj);
    }
}
