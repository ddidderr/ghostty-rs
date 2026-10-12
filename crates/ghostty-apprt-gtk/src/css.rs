//! CSS handling for the application (the CSS portions of
//! ZIG/src/apprt/gtk/class/application.zig).
//!
//! The static stylesheets (`style.css`, `style-dark.css`, `style-hc.css`,
//! `style-hc-dark.css`) live in the gresource at
//! `/com/mitchellh/ghostty/` and are loaded automatically by
//! `AdwApplication` when its `resource-base-path` is
//! [`crate::RESOURCE_PREFIX`]; so the application must set that.
//!
//! This module owns the *dynamic* providers: the runtime CSS derived from
//! the configuration and the user's `gtk-custom-css` files.

use std::fmt::Write as _;

use ghostty_config::{config::Config, path::Path, types_b::WindowTheme};
use gtk::{gdk, glib, prelude::*};

/// Maximum size of a `gtk-custom-css` file.
const CSS_FILE_SIZE_LIMIT: u64 = 5 * 1024 * 1024;

/// The set of CSS providers installed on the display.
#[derive(Debug)]
pub struct CssProviders {
    display: gdk::Display,
    /// Styles based on Ghostty configuration values.
    runtime: gtk::CssProvider,
    /// Providers for `gtk-custom-css`.
    custom: Vec<gtk::CssProvider>,
    /// Last applied color scheme (dark?), reapplied to new providers.
    dark: Option<bool>,
}

fn at_least_gtk(major: u32, minor: u32) -> bool {
    (gtk::major_version(), gtk::minor_version()) >= (major, minor)
}

/// Log a CSS parsing error (Zig: `signalCssParsingError`).
fn log_parsing_error(section: &gtk::CssSection, err: &glib::Error) {
    let location = section.to_str();
    log::warn!("css parsing failed at {location}: {err}");
}

fn new_provider() -> gtk::CssProvider {
    let p = gtk::CssProvider::new();
    p.connect_parsing_error(|_, section, err| log_parsing_error(section, err));
    p
}

impl CssProviders {
    /// Create the runtime provider and install it on the display
    /// (`STYLE_PROVIDER_PRIORITY_APPLICATION + 3`).
    pub fn new(display: &gdk::Display) -> Self {
        let runtime = new_provider();
        gtk::style_context_add_provider_for_display(
            display,
            &runtime,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3,
        );
        Self {
            display: display.clone(),
            runtime,
            custom: Vec::new(),
            dark: None,
        }
    }

    /// Reload runtime and custom CSS from the config. Failures are logged
    /// and leave the previous CSS in place.
    pub fn load_all(&mut self, config: &Config) {
        self.load_runtime(config);
        self.load_custom(config);
    }

    /// Load the config-derived runtime CSS (Zig: `loadRuntimeCss`).
    pub fn load_runtime(&self, config: &Config) {
        let contents = runtime_css(config, at_least_gtk(4, 16));
        log::debug!("runtime CSS is {} bytes", contents.len());
        self.runtime
            .load_from_bytes(&glib::Bytes::from_owned(contents.into_bytes()));
    }

    /// Load `gtk-custom-css` files (Zig: `loadCustomCss`).
    pub fn load_custom(&mut self, config: &Config) {
        for p in self.custom.drain(..) {
            gtk::style_context_remove_provider_for_display(&self.display, &p);
        }

        for p in &config.gtk_custom_css.value {
            let (path, optional) = match p {
                Path::Optional(path) => (path, true),
                Path::Required(path) => (path, false),
            };
            let meta = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::NotFound || !optional {
                        log::warn!("error opening gtk-custom-css file {path}: {e}");
                    }
                    continue;
                }
            };
            log::info!("loading gtk-custom-css path={path}");
            if meta.len() > CSS_FILE_SIZE_LIMIT {
                log::warn!(
                    "gtk-custom-css file {path} was larger than {CSS_FILE_SIZE_LIMIT} bytes"
                );
                continue;
            }
            let contents = match std::fs::read(path) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("error reading gtk-custom-css file {path}: {e}");
                    continue;
                }
            };

            let provider = new_provider();
            provider.load_from_bytes(&glib::Bytes::from_owned(contents));
            gtk::style_context_add_provider_for_display(
                &self.display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
            if let Some(dark) = self.dark {
                set_provider_scheme(&provider, dark);
            }
            self.custom.push(provider);
        }
    }

    /// Propagate the system color scheme to the providers so that
    /// `prefers-color-scheme` media queries match (GTK >= 4.20 only).
    pub fn set_color_scheme(&mut self, dark: bool) {
        self.dark = Some(dark);
        set_provider_scheme(&self.runtime, dark);
        for p in &self.custom {
            set_provider_scheme(p, dark);
        }
    }
}

/// Set the `prefers-color-scheme` property on a provider (GTK >= 4.20).
/// Done through the dynamic property API since the crate is built against
/// an older GTK feature level.
fn set_provider_scheme(provider: &gtk::CssProvider, dark: bool) {
    if !at_least_gtk(4, 20) {
        return;
    }
    let Some(pspec) = provider.find_property("prefers-color-scheme") else {
        return;
    };
    let Some(class) = glib::EnumClass::with_type(pspec.value_type()) else {
        return;
    };
    let nick = if dark { "dark" } else { "light" };
    if let Some(v) = class.to_value_by_nick(nick) {
        provider.set_property_from_value("prefers-color-scheme", &v);
    }
}

/// Build the runtime CSS string. Pure function (testable).
/// `gtk_416` selects between the pre/post GTK 4.16 variants.
#[must_use]
pub fn runtime_css(config: &Config, gtk_416: bool) -> String {
    let mut out = String::with_capacity(2048);
    // Standard css first as it can override some user configured styling.
    if gtk_416 {
        runtime_css_416(config, &mut out);
    } else {
        runtime_css_414(config, &mut out);
    }

    let fill = config.unfocused_split_fill.unwrap_or(config.background);
    let _ = write!(
        out,
        "widget.unfocused-split {{\n opacity: {:.2};\n background-color: rgb({},{},{});\n}}\n",
        1.0 - config.unfocused_split_opacity,
        fill.r,
        fill.g,
        fill.b,
    );

    if let Some(c) = config.split_divider_color {
        let _ = write!(
            out,
            ".window .split paned > separator {{\n  color: rgb({r},{g},{b});\n  background: rgb({r},{g},{b});\n}}\n",
            r = c.r,
            g = c.g,
            b = c.b,
        );
    }

    if let Some(family) = &config.window_title_font_family {
        let _ = write!(
            out,
            ".window headerbar {{\n  font-family: \"{family}\";\n}}\n"
        );
    }
    out
}

fn runtime_css_414(config: &Config, out: &mut String) {
    if config.window_theme != WindowTheme::Ghostty {
        return;
    }
    let bg = config
        .window_titlebar_background
        .unwrap_or(config.background);
    let fg = config
        .window_titlebar_foreground
        .unwrap_or(config.foreground);
    let _ = write!(
        out,
        "windowhandle {{\n  background-color: rgb({},{},{});\n  color: rgb({},{},{});\n}}\nwindowhandle:backdrop {{\n background-color: oklab(from rgb({},{},{}) calc(l * 0.9) a b / alpha);\n}}\n",
        bg.r, bg.g, bg.b, fg.r, fg.g, fg.b, bg.r, bg.g, bg.b,
    );
}

fn runtime_css_416(config: &Config, out: &mut String) {
    out.push_str(
        "/*
 * Child Exited Overlay
 */

.child-exited.normal revealer widget {
  background-color: color-mix(
    in srgb,
    var(--success-bg-color),
    transparent 50%
  );
}

.child-exited.abnormal revealer widget {
  background-color: color-mix(
    in srgb,
    var(--error-bg-color),
    transparent 50%
  );
}

/*
 * Surface
 */

.surface progressbar.error trough progress {
  background-color: color-mix(
    in srgb,
    var(--error-bg-color),
    transparent 50%
  );
}

.surface .bell-overlay {
  border-color: color-mix(
    in srgb,
    var(--accent-color),
    transparent 50%
  );
}

/*
 * Splits
 */

.window .split paned > separator {
  background-color: color-mix(
    in srgb,
    var(--window-bg-color),
    transparent 0%
  );
}
",
    );

    if config.window_theme == WindowTheme::Ghostty {
        let bg = config
            .window_titlebar_background
            .unwrap_or(config.background);
        let fg = config
            .window_titlebar_foreground
            .unwrap_or(config.foreground);
        let _ = write!(
            out,
            ":root {{
  --ghostty-fg: rgb({},{},{});
  --ghostty-bg: rgb({},{},{});
  --headerbar-fg-color: var(--ghostty-fg);
  --headerbar-bg-color: var(--ghostty-bg);
  --headerbar-backdrop-color: oklab(from var(--headerbar-bg-color) calc(l * 0.9) a b / alpha);
  --overview-fg-color: var(--ghostty-fg);
  --overview-bg-color: var(--ghostty-bg);
  --popover-fg-color: var(--ghostty-fg);
  --popover-bg-color: var(--ghostty-bg);
  --window-fg-color: var(--ghostty-fg);
  --window-bg-color: var(--ghostty-bg);
}}
windowhandle {{
  background-color: var(--headerbar-bg-color);
  color: var(--headerbar-fg-color);
}}
windowhandle:backdrop {{
 background-color: var(--headerbar-backdrop-color);
}}",
            fg.r, fg.g, fg.b, bg.r, bg.g, bg.b,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_css_has_unfocused_split() {
        let c = Config::default();
        let css = runtime_css(&c, true);
        assert!(css.contains("widget.unfocused-split"));
        assert!(css.contains("opacity: 0.30;"));
        assert!(css.contains(".child-exited.normal"));
        let old = runtime_css(&c, false);
        assert!(!old.contains(".child-exited.normal"));
    }

    #[test]
    fn runtime_css_ghostty_theme() {
        let c = Config {
            window_theme: WindowTheme::Ghostty,
            ..Config::default()
        };
        assert!(runtime_css(&c, true).contains("--ghostty-bg"));
        assert!(runtime_css(&c, false).contains("windowhandle:backdrop"));
    }
}
