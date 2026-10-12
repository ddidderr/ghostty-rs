//! `Ghostty.SurfaceConfiguration` (`Surface View/SurfaceView.swift`): the
//! configuration for a surface. For any configuration not set, defaults are
//! chosen by libghostty, usually from the Ghostty configuration.
//!
//! Portable (no AppKit): the view pointer and the scale factor, which the
//! Swift version reads from `NSScreen.main`, are parameters of
//! [`SurfaceConfiguration::with_c_value`].
#![allow(unsafe_code)]

use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_void},
};

use crate::ghostty::capi::{
    consts::{GHOSTTY_PLATFORM_MACOS, GHOSTTY_SURFACE_CONTEXT_WINDOW},
    ghostty_env_var_s,
    ghostty_platform_macos_s,
    ghostty_platform_u,
    ghostty_surface_config_new,
    ghostty_surface_config_s,
    ghostty_surface_context_e,
};

/// `Ghostty.SurfaceConfiguration`.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceConfiguration {
    /// Explicit font size to use in points.
    pub font_size: Option<f32>,
    /// Explicit working directory to set.
    pub working_directory: Option<String>,
    /// Explicit command to set.
    pub command: Option<String>,
    /// Environment variables to set for the terminal.
    pub environment_variables: BTreeMap<String, String>,
    /// Extra input to send as stdin.
    pub initial_input: Option<String>,
    /// Wait after the command.
    pub wait_after_command: bool,
    /// Context for surface creation.
    pub context: ghostty_surface_context_e,
}

impl Default for SurfaceConfiguration {
    fn default() -> Self {
        Self {
            font_size: None,
            working_directory: None,
            command: None,
            environment_variables: BTreeMap::new(),
            initial_input: None,
            wait_after_command: false,
            context: GHOSTTY_SURFACE_CONTEXT_WINDOW,
        }
    }
}

/// Read an optional C string as UTF-8 (`String(cString:encoding: .utf8)`).
///
/// # Safety
/// `p` must be null or a valid NUL terminated string.
unsafe fn opt_string(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: caller guarantees validity.
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

/// `withCString` semantics: a Swift string with an embedded NUL is seen by C
/// up to the NUL.
fn to_cstring(s: &str) -> CString {
    let end = s.find('\0').unwrap_or(s.len());
    CString::new(&s.as_bytes()[..end]).unwrap_or_default()
}

impl SurfaceConfiguration {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `init(from: ghostty_surface_config_s)`.
    ///
    /// # Safety
    /// The string and env var pointers of `config` must be valid (as
    /// returned by `ghostty_surface_inherited_config`).
    #[must_use]
    pub unsafe fn from_c(config: &ghostty_surface_config_s) -> Self {
        let mut this = Self {
            font_size: Some(config.font_size),
            ..Self::default()
        };
        // SAFETY: per the function contract.
        unsafe {
            this.working_directory = opt_string(config.working_directory);
            this.command = opt_string(config.command);
            if config.env_var_count > 0 && !config.env_vars.is_null() {
                for i in 0..config.env_var_count {
                    let env = *config.env_vars.add(i);
                    if let (Some(k), Some(v)) = (opt_string(env.key), opt_string(env.value)) {
                        this.environment_variables.insert(k, v);
                    }
                }
            }
        }
        this.context = config.context;
        this
    }

    /// `withCValue(view:_:)`: a C configuration valid only within `body`.
    /// `view` is the address of the surface view (also the userdata);
    /// `scale_factor` is `NSScreen.main!.backingScaleFactor`.
    pub fn with_c_value<T>(
        &self,
        view: *mut c_void,
        scale_factor: f64,
        body: impl FnOnce(&mut ghostty_surface_config_s) -> T,
    ) -> T {
        let mut config = ghostty_surface_config_new();
        config.userdata = view;
        config.platform_tag = GHOSTTY_PLATFORM_MACOS;
        config.platform = ghostty_platform_u {
            macos: ghostty_platform_macos_s { nsview: view },
        };
        config.scale_factor = scale_factor;

        // Zero is our default value that means to inherit the font size.
        config.font_size = self.font_size.unwrap_or(0.0);
        config.wait_after_command = self.wait_after_command;
        config.context = self.context;

        // The CStrings stay alive for the whole call, like `withCString`.
        let working_dir = self.working_directory.as_deref().map(to_cstring);
        let command = self.command.as_deref().map(to_cstring);
        let input = self.initial_input.as_deref().map(to_cstring);
        let ptr = |s: &Option<CString>| s.as_ref().map_or(std::ptr::null(), |s| s.as_ptr());
        config.working_directory = ptr(&working_dir);
        config.command = ptr(&command);
        config.initial_input = ptr(&input);

        let keys: Vec<CString> = self
            .environment_variables
            .keys()
            .map(|k| to_cstring(k))
            .collect();
        let values: Vec<CString> = self
            .environment_variables
            .values()
            .map(|v| to_cstring(v))
            .collect();
        let mut env_vars: Vec<ghostty_env_var_s> = keys
            .iter()
            .zip(&values)
            .map(|(k, v)| ghostty_env_var_s {
                key: k.as_ptr(),
                value: v.as_ptr(),
            })
            .collect();
        config.env_vars = env_vars.as_mut_ptr();
        config.env_var_count = env_vars.len();
        body(&mut config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = SurfaceConfiguration::new();
        assert_eq!(c.font_size, None);
        assert!(!c.wait_after_command);
        assert_eq!(c.context, GHOSTTY_SURFACE_CONTEXT_WINDOW);
    }

    #[test]
    fn c_value_round_trip() {
        let mut c = SurfaceConfiguration::new();
        c.font_size = Some(14.0);
        c.working_directory = Some("/tmp".into());
        c.command = Some("echo hi".into());
        c.initial_input = Some("ls\n".into());
        c.wait_after_command = true;
        c.environment_variables.insert("A".into(), "1".into());
        c.environment_variables.insert("B".into(), "2".into());
        let mut fake = 0u8;
        let view = std::ptr::addr_of_mut!(fake).cast::<c_void>();
        let back = c.with_c_value(view, 2.0, |cfg| {
            assert_eq!(cfg.userdata, view);
            assert_eq!(cfg.platform_tag, GHOSTTY_PLATFORM_MACOS);
            assert!((cfg.scale_factor - 2.0).abs() < f64::EPSILON);
            assert!(cfg.wait_after_command);
            // SAFETY: pointers valid within the closure.
            let mut back = unsafe { SurfaceConfiguration::from_c(cfg) };
            // initial_input / wait_after_command are not read back (as in Swift).
            back.initial_input = c.initial_input.clone();
            back.wait_after_command = true;
            back
        });
        assert_eq!(back, c);
    }

    #[test]
    fn unset_strings_are_null() {
        let c = SurfaceConfiguration::new();
        c.with_c_value(std::ptr::null_mut(), 1.0, |cfg| {
            assert!(cfg.working_directory.is_null());
            assert!(cfg.command.is_null());
            assert!(cfg.initial_input.is_null());
            assert_eq!(cfg.env_var_count, 0);
            assert_eq!(cfg.font_size.to_bits(), 0);
        });
    }
}
