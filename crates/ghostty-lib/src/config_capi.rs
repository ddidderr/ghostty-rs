//! `ghostty_config_*` (ZIG/src/config/CApi.zig and c_get.zig).
#![allow(unsafe_code)]

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, c_char, c_int, c_short, c_uint, c_void},
    rc::Rc,
    sync::{Mutex, OnceLock},
};

use ghostty_config::{
    c_value::{CValue, QuickTerminalSizeC},
    config::Config,
};
use ghostty_input::binding::{Action as BindingAction, TriggerC, TriggerCTag};

use crate::{
    main_c::{EMPTY_STRING, string_from_owned},
    types::{
        ghostty_command_s,
        ghostty_config_color_list_s,
        ghostty_config_color_s,
        ghostty_config_command_list_s,
        ghostty_config_palette_s,
        ghostty_config_path_s,
        ghostty_config_quick_terminal_size_s,
        ghostty_config_t,
        ghostty_diagnostic_s,
        ghostty_input_trigger_key_u,
        ghostty_input_trigger_s,
        ghostty_quick_terminal_size_s,
        ghostty_quick_terminal_size_value_u,
        ghostty_string_s,
        quick_terminal_size_tag,
        trigger_tag,
    },
};

/// The C-owned configuration: the config plus the memory that pointers
/// handed out by `ghostty_config_get` / `ghostty_config_get_diagnostic` point
/// into (Zig: the config's arena). Everything stays valid until the config
/// is freed.
pub struct CConfig {
    pub config: Config,
    cache: RefCell<GetCache>,
    diagnostics: RefCell<Vec<CString>>,
}

/// Backing memory of one `ghostty_config_get` result.
#[derive(Default)]
struct Backing {
    strings: Vec<CString>,
    colors: Box<[ghostty_config_color_s]>,
    commands: Box<[ghostty_command_s]>,
}

#[derive(Default)]
struct GetCache {
    /// The latest result per key; reused if the value didn't change so
    /// repeated gets return the same pointers.
    by_key: HashMap<String, (CValue, Rc<Backing>)>,
    /// Results that were replaced; kept alive until the config is freed
    /// since callers may still hold their pointers.
    retired: Vec<Rc<Backing>>,
}

impl CConfig {
    pub fn new(config: Config) -> CConfig {
        CConfig {
            config,
            cache: RefCell::default(),
            diagnostics: RefCell::default(),
        }
    }

    /// Box the config and return the `ghostty_config_t` handle.
    pub fn into_handle(self) -> ghostty_config_t {
        Box::into_raw(Box::new(self)).cast::<c_void>()
    }

    /// The backing memory for `value`, reusing the cached one if equal.
    fn backing(&self, key: &str, value: &CValue) -> Rc<Backing> {
        let mut cache = self.cache.borrow_mut();
        if let Some((old, backing)) = cache.by_key.get(key)
            && old == value
        {
            return Rc::clone(backing);
        }
        let backing = Rc::new(build_backing(value));
        if let Some((_, old)) = cache
            .by_key
            .insert(key.to_owned(), (value.clone(), Rc::clone(&backing)))
        {
            cache.retired.push(old);
        }
        backing
    }
}

/// A C string without interior NULs.
fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// Intern a string for the lifetime of the process (enum tag names).
fn intern(s: &str) -> *const c_char {
    static POOL: OnceLock<Mutex<HashSet<Box<CStr>>>> = OnceLock::new();
    let Ok(mut pool) = POOL.get_or_init(Mutex::default).lock() else {
        return c"".as_ptr();
    };
    let c = cstring(s);
    if let Some(existing) = pool.get(c.as_c_str()) {
        return existing.as_ptr();
    }
    let boxed: Box<CStr> = c.into_boxed_c_str();
    let ptr = boxed.as_ptr();
    pool.insert(boxed);
    ptr
}

fn build_backing(value: &CValue) -> Backing {
    let mut b = Backing::default();
    match value {
        CValue::String(s) => b.strings.push(cstring(s)),
        CValue::Path { path, .. } => b.strings.push(cstring(path)),
        CValue::ColorList(list) => {
            b.colors = list
                .iter()
                .map(|c| ghostty_config_color_s {
                    r: c[0],
                    g: c[1],
                    b: c[2],
                })
                .collect();
        }
        CValue::Commands(cmds) => {
            for c in cmds {
                b.strings.push(cstring(&c.action_key));
                b.strings.push(cstring(&c.action));
                b.strings.push(cstring(&c.title));
                b.strings.push(cstring(&c.description));
            }
            // The CStrings' heap buffers don't move when the Vec grows.
            b.commands = b
                .strings
                .as_chunks::<4>()
                .0
                .iter()
                .map(|s| ghostty_command_s {
                    action_key: s[0].as_ptr(),
                    action: s[1].as_ptr(),
                    title: s[2].as_ptr(),
                    description: s[3].as_ptr(),
                })
                .collect();
        }
        _ => {}
    }
    b
}

/// Borrow the config behind a handle.
///
/// # Safety
/// `p` must be null or a handle returned by this library that wasn't freed,
/// and no other reference to it may be live.
pub(crate) unsafe fn cconfig<'a>(p: ghostty_config_t) -> Option<&'a mut CConfig> {
    // SAFETY: per the function contract.
    unsafe { p.cast::<CConfig>().as_mut() }
}

/// Create a new configuration filled with the initial default values.
#[unsafe(no_mangle)]
pub extern "C" fn ghostty_config_new() -> ghostty_config_t {
    CConfig::new(Config::default()).into_handle()
}

/// Free a configuration.
///
/// # Safety
/// `ptr` must be null or a handle from this library, not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_free(ptr: ghostty_config_t) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: the handle was created by `into_handle` (a leaked Box).
    drop(unsafe { Box::from_raw(ptr.cast::<CConfig>()) });
}

/// Deep clone the configuration.
///
/// # Safety
/// `ptr` must be a valid config handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_clone(ptr: ghostty_config_t) -> ghostty_config_t {
    // SAFETY: per the function contract.
    let Some(c) = (unsafe { cconfig(ptr) }) else {
        return std::ptr::null_mut();
    };
    CConfig::new(c.config.clone()).into_handle()
}

/// Load the configuration from the CLI args.
///
/// # Safety
/// `ptr` must be a valid config handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_load_cli_args(ptr: ghostty_config_t) {
    // SAFETY: per the function contract.
    let Some(c) = (unsafe { cconfig(ptr) }) else {
        return;
    };
    if let Err(err) = c.config.load_cli_args() {
        log::error!("error loading config err={err}");
    }
}

/// Load the configuration from the default file locations. This is usually
/// done first. The default file locations are locations such as the home
/// directory.
///
/// # Safety
/// `ptr` must be a valid config handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_load_default_files(ptr: ghostty_config_t) {
    // SAFETY: per the function contract.
    let Some(c) = (unsafe { cconfig(ptr) }) else {
        return;
    };
    if let Err(err) = c.config.load_default_files() {
        log::error!("error loading config err={err}");
    }
}

/// Load the configuration from a specific file path. The path must be null
/// terminated.
///
/// # Safety
/// `ptr` must be a valid config handle, `path` a NUL terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_load_file(ptr: ghostty_config_t, path: *const c_char) {
    // SAFETY: per the function contract.
    let Some(c) = (unsafe { cconfig(ptr) }) else {
        return;
    };
    if path.is_null() {
        return;
    }
    // SAFETY: `path` is NUL terminated.
    // Non-UTF-8 file names are valid on Unix; keep the bytes (reversible
    // escape understood by the config file loader).
    let path =
        ghostty_os::osbytes::bytes_to_config_string(unsafe { CStr::from_ptr(path) }.to_bytes());
    if let Err(err) = c.config.load_file(&path) {
        log::error!("error loading config from file path={path} err={err}");
    }
}

/// Load the configuration from the user-specified configuration file
/// locations in the previously loaded configuration. This will recursively
/// continue to load up to a built-in limit.
///
/// # Safety
/// `ptr` must be a valid config handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_load_recursive_files(ptr: ghostty_config_t) {
    // SAFETY: per the function contract.
    let Some(c) = (unsafe { cconfig(ptr) }) else {
        return;
    };
    if let Err(err) = c.config.load_recursive_files() {
        log::error!("error loading config err={err}");
    }
}

/// Finalize the configuration (fill in computed defaults).
///
/// # Safety
/// `ptr` must be a valid config handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_finalize(ptr: ghostty_config_t) {
    // SAFETY: per the function contract.
    let Some(c) = (unsafe { cconfig(ptr) }) else {
        return;
    };
    if let Err(err) = c.config.finalize() {
        log::error!("error finalizing config err={err}");
    }
}

/// Store `v` at `out` (`T` must be what the C caller expects for the key).
///
/// # Safety
/// `out` must be valid for a write of `T`.
unsafe fn put<T>(out: *mut c_void, v: T) {
    // SAFETY: per the function contract; unaligned writes are tolerated.
    unsafe { out.cast::<T>().write_unaligned(v) };
}

fn qt_size(v: QuickTerminalSizeC) -> ghostty_quick_terminal_size_s {
    match v {
        QuickTerminalSizeC::None => ghostty_quick_terminal_size_s {
            tag: quick_terminal_size_tag::NONE,
            value: ghostty_quick_terminal_size_value_u { pixels: 0 },
        },
        QuickTerminalSizeC::Percentage(p) => ghostty_quick_terminal_size_s {
            tag: quick_terminal_size_tag::PERCENTAGE,
            value: ghostty_quick_terminal_size_value_u { percentage: p },
        },
        QuickTerminalSizeC::Pixels(p) => ghostty_quick_terminal_size_s {
            tag: quick_terminal_size_tag::PIXELS,
            value: ghostty_quick_terminal_size_value_u { pixels: p },
        },
    }
}

impl CConfig {
    /// `c_get.get`: write the value of `key` to `out`. Returns false if the
    /// key is unknown, its type isn't supported by the C API yet or an
    /// optional value is unset.
    ///
    /// # Safety
    /// `out` must point to storage of the C type of the key (see
    /// `ghostty.h` and `c_get.zig`).
    pub unsafe fn get(&self, key: &str, out: *mut c_void) -> bool {
        let Some(value) = self.config.c_value(key) else {
            return false;
        };
        // SAFETY (all writes below): `out` is valid for the key's C type by
        // the function contract.
        match &value {
            CValue::Unsupported | CValue::Null => return false,
            CValue::NullString => unsafe { put(out, std::ptr::null::<c_char>()) },
            CValue::String(_) => {
                let b = self.backing(key, &value);
                unsafe { put(out, b.strings[0].as_ptr()) };
            }
            CValue::Bool(v) => unsafe { put(out, *v) },
            CValue::Uint(v) => unsafe { put::<c_uint>(out, *v) },
            CValue::Short(v) => unsafe { put::<c_short>(out, *v) },
            CValue::F32(v) => unsafe { put(out, *v) },
            CValue::F64(v) => unsafe { put(out, *v) },
            CValue::Usize(v) => unsafe { put(out, *v) },
            CValue::EnumName(name) => unsafe { put(out, intern(name)) },
            CValue::Packed(bits) => unsafe { put::<c_uint>(out, *bits) },
            CValue::Color { r, g, b } => unsafe {
                put(
                    out,
                    ghostty_config_color_s {
                        r: *r,
                        g: *g,
                        b: *b,
                    },
                );
            },
            CValue::ColorList(list) => {
                let b = self.backing(key, &value);
                unsafe {
                    put(
                        out,
                        ghostty_config_color_list_s {
                            colors: b.colors.as_ptr(),
                            len: list.len(),
                        },
                    );
                }
            }
            CValue::Palette(colors) => {
                let mut p = ghostty_config_palette_s {
                    colors: [ghostty_config_color_s { r: 0, g: 0, b: 0 }; 256],
                };
                for (dst, c) in p.colors.iter_mut().zip(colors.iter()) {
                    *dst = ghostty_config_color_s {
                        r: c[0],
                        g: c[1],
                        b: c[2],
                    };
                }
                unsafe { put(out, p) };
            }
            CValue::Commands(cmds) => {
                let b = self.backing(key, &value);
                unsafe {
                    put(
                        out,
                        ghostty_config_command_list_s {
                            commands: b.commands.as_ptr(),
                            len: cmds.len(),
                        },
                    );
                }
            }
            CValue::QuickTerminalSize { primary, secondary } => unsafe {
                put(
                    out,
                    ghostty_config_quick_terminal_size_s {
                        primary: qt_size(*primary),
                        secondary: qt_size(*secondary),
                    },
                );
            },
            CValue::Path { optional, .. } => {
                let b = self.backing(key, &value);
                unsafe {
                    put(
                        out,
                        ghostty_config_path_s {
                            path: b.strings[0].as_ptr(),
                            optional: *optional,
                        },
                    );
                }
            }
        }
        true
    }
}

/// Get the value of a config key (see `c_get.zig` for the C types).
///
/// # Safety
/// `ptr` must be a valid config handle, `key_str` must point to `len`
/// bytes and `out` to storage of the key's C type.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_get(
    ptr: ghostty_config_t,
    out: *mut c_void,
    key_str: *const c_char,
    len: usize,
) -> bool {
    // SAFETY: per the function contract.
    let Some(c) = (unsafe { cconfig(ptr) }) else {
        return false;
    };
    if out.is_null() || key_str.is_null() {
        return false;
    }
    // SAFETY: `key_str` points to `len` bytes.
    let key = unsafe { std::slice::from_raw_parts(key_str.cast::<u8>(), len) };
    let Ok(key) = std::str::from_utf8(key) else {
        return false;
    };
    // SAFETY: `out` is valid for the key's type per the contract.
    unsafe { c.get(key, out) }
}

/// Convert the Rust trigger to the C struct.
pub(crate) fn trigger_to_c(t: &TriggerC) -> ghostty_input_trigger_s {
    let (tag, key) = match t.tag {
        TriggerCTag::Physical => (
            trigger_tag::PHYSICAL,
            ghostty_input_trigger_key_u {
                physical: t.key as c_int,
            },
        ),
        TriggerCTag::Unicode => (
            trigger_tag::UNICODE,
            ghostty_input_trigger_key_u { unicode: t.key },
        ),
        TriggerCTag::CatchAll => (
            trigger_tag::CATCH_ALL,
            ghostty_input_trigger_key_u { unicode: 0 },
        ),
    };
    ghostty_input_trigger_s {
        tag,
        key,
        mods: c_int::from(t.mods.int()),
    }
}

/// The default (unset) trigger (`Trigger.C{}`).
pub(crate) fn default_trigger() -> ghostty_input_trigger_s {
    ghostty_input_trigger_s {
        tag: trigger_tag::PHYSICAL,
        key: ghostty_input_trigger_key_u { physical: 0 },
        mods: 0,
    }
}

/// Find the trigger bound to the given binding action string.
///
/// # Safety
/// `ptr` must be a valid config handle, `str` must point to `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_trigger(
    ptr: ghostty_config_t,
    str: *const c_char,
    len: usize,
) -> ghostty_input_trigger_s {
    // SAFETY: per the function contract.
    let Some(c) = (unsafe { cconfig(ptr) }) else {
        return default_trigger();
    };
    if str.is_null() {
        return default_trigger();
    }
    // SAFETY: `str` points to `len` bytes.
    let s = unsafe { std::slice::from_raw_parts(str.cast::<u8>(), len) };
    let Ok(s) = std::str::from_utf8(s) else {
        log::error!("error finding trigger err=InvalidFormat");
        return default_trigger();
    };
    match BindingAction::parse(s) {
        Ok(action) => {
            let trigger = c
                .config
                .keybind
                .set
                .get_trigger(&action)
                .unwrap_or_default();
            trigger_to_c(&trigger.cval())
        }
        Err(err) => {
            log::error!("error finding trigger err={err:?}");
            default_trigger()
        }
    }
}

/// The number of diagnostics.
///
/// # Safety
/// `ptr` must be a valid config handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_diagnostics_count(ptr: ghostty_config_t) -> u32 {
    // SAFETY: per the function contract.
    let Some(c) = (unsafe { cconfig(ptr) }) else {
        return 0;
    };
    u32::try_from(c.config.diagnostics.items().len()).unwrap_or(u32::MAX)
}

/// The diagnostic at the given index (an empty message if out of range).
///
/// # Safety
/// `ptr` must be a valid config handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghostty_config_get_diagnostic(
    ptr: ghostty_config_t,
    idx: u32,
) -> ghostty_diagnostic_s {
    let empty = ghostty_diagnostic_s {
        message: c"".as_ptr(),
    };
    // SAFETY: per the function contract.
    let Some(c) = (unsafe { cconfig(ptr) }) else {
        return empty;
    };
    let items = c.config.diagnostics.items();
    let idx = idx as usize;
    if idx >= items.len() {
        return empty;
    }

    // Precompute (like libghostty's `DiagnosticList.precompute`): the
    // messages are formatted once per diagnostic and live as long as the
    // config.
    let mut msgs = c.diagnostics.borrow_mut();
    while msgs.len() < items.len() {
        let mut text = String::new();
        let _ = items[msgs.len()].format(&mut text);
        msgs.push(cstring(&text));
    }
    ghostty_diagnostic_s {
        message: msgs[idx].as_ptr(),
    }
}

/// The path of the config file to open in an editor (created if needed).
/// Free with `ghostty_string_free`.
#[unsafe(no_mangle)]
pub extern "C" fn ghostty_config_open_path() -> ghostty_string_s {
    match ghostty_config::edit::open_path() {
        Ok(path) => string_from_owned(&path),
        Err(err) => {
            log::error!("error opening config in editor err={err}");
            EMPTY_STRING
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::c_uint;

    use super::*;

    fn get<T: Copy>(cfg: ghostty_config_t, key: &str, init: T) -> (bool, T) {
        let mut out = init;
        // SAFETY: `out` is a valid destination of the key's C type.
        let ok = unsafe {
            ghostty_config_get(cfg, (&raw mut out).cast(), key.as_ptr().cast(), key.len())
        };
        (ok, out)
    }

    fn with_config(f: impl FnOnce(&mut Config)) -> ghostty_config_t {
        let h = ghostty_config_new();
        // SAFETY: just created.
        f(&mut unsafe { cconfig(h) }.unwrap().config);
        h
    }

    #[test]
    fn ghostty_config_get_bool() {
        let h = with_config(|c| c.maximize = true);
        assert_eq!(get(h, "maximize", false), (true, true));
        // SAFETY: valid handle.
        unsafe { ghostty_config_free(h) };
    }

    #[test]
    fn ghostty_config_get_enum() {
        let h = with_config(|c| c.window_theme = ghostty_config::types_b::WindowTheme::Dark);
        let (ok, p) = get::<*const c_char>(h, "window-theme", std::ptr::null());
        assert!(ok);
        // SAFETY: an interned NUL terminated string.
        assert_eq!(unsafe { CStr::from_ptr(p) }.to_str().unwrap(), "dark");
        // SAFETY: valid handle.
        unsafe { ghostty_config_free(h) };
    }

    #[test]
    fn ghostty_config_get_optional_null_returns_false() {
        let h = with_config(|c| c.unfocused_split_fill = None);
        let (ok, _) = get(
            h,
            "unfocused-split-fill",
            ghostty_config_color_s { r: 0, g: 0, b: 0 },
        );
        assert!(!ok);
        // SAFETY: valid handle.
        unsafe { ghostty_config_free(h) };
    }

    #[test]
    fn ghostty_config_get_unknown_key_returns_false() {
        let h = ghostty_config_new();
        assert!(!get(h, "not-a-real-key", false).0);
        // SAFETY: valid handle.
        unsafe { ghostty_config_free(h) };
    }

    #[test]
    fn ghostty_config_get_optional_string_null_returns_true() {
        let h = with_config(|c| c.title = None);
        let (ok, p) = get::<*const c_char>(h, "title", std::ptr::dangling::<c_char>());
        assert!(ok);
        assert!(p.is_null());
        // SAFETY: valid handle.
        unsafe { ghostty_config_free(h) };
    }

    #[test]
    fn ghostty_config_get_float() {
        let h = with_config(|c| c.background_opacity = 0.42);
        let (ok, v) = get(h, "background-opacity", 0.0f64);
        assert!(ok);
        assert!((v - 0.42).abs() < 1e-6);
        // SAFETY: valid handle.
        unsafe { ghostty_config_free(h) };
    }

    #[test]
    fn ghostty_config_get_struct_cval_conversion() {
        let h = with_config(|c| {
            c.background = ghostty_config::types_a::Color {
                r: 12,
                g: 34,
                b: 56,
            };
        });
        let (ok, v) = get(h, "background", ghostty_config_color_s { r: 0, g: 0, b: 0 });
        assert!(ok);
        assert_eq!((v.r, v.g, v.b), (12, 34, 56));
        // SAFETY: valid handle.
        unsafe { ghostty_config_free(h) };
    }

    #[test]
    fn c_get_u8() {
        let h = with_config(|c| c.font_size = 24.0);
        assert_eq!(get(h, "font-size", 0.0f32), (true, 24.0));
        // SAFETY: valid handle.
        unsafe { ghostty_config_free(h) };
    }

    #[test]
    fn c_get_split_preserve_zoom() {
        let h = ghostty_config_new();
        assert_eq!(get(h, "split-preserve-zoom", 9 as c_uint), (true, 0));
        // SAFETY: valid handle.
        unsafe {
            cconfig(h).unwrap().config.split_preserve_zoom.navigation = true;
        }
        assert_eq!(get(h, "split-preserve-zoom", 9 as c_uint), (true, 1));
        // SAFETY: valid handle.
        unsafe { ghostty_config_free(h) };
    }

    #[test]
    fn c_get_pointers_are_stable_for_equal_values() {
        let h = ghostty_config_new();
        let (_, a) = get::<ghostty_config_command_list_s>(
            h,
            "command-palette-entry",
            ghostty_config_command_list_s {
                commands: std::ptr::null(),
                len: 0,
            },
        );
        let (_, b) = get::<ghostty_config_command_list_s>(
            h,
            "command-palette-entry",
            ghostty_config_command_list_s {
                commands: std::ptr::null(),
                len: 0,
            },
        );
        assert!(a.len > 0);
        assert_eq!(a.commands, b.commands);
        // SAFETY: valid handle.
        unsafe { ghostty_config_free(h) };
    }
}
