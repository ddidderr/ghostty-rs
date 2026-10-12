//! JS interop for wasm. Stands in for the `zig-js` package
//! (`mitchellh/zig-js`) that Ghostty's Zig wasm code (`font/face/web_canvas`,
//! `font/Atlas`) imports as `js`.
//!
//! The `zig-js` source is not part of the Ghostty tree, so its exact import
//! names cannot be mirrored; this module defines an equivalent handle based
//! protocol in the import module `ghostty_js` and keeps the *Rust-side API*
//! shaped like `zig-js` (`Object`, `get`, `set`, `call`, `new`, string and
//! number arguments, global object lookup). The host (JS) keeps a table of
//! values; wasm only holds `u32` handles. Handle `0` is "invalid" and is
//! what every import returns when the JS operation threw.
//!
//! Imports (module `ghostty_js`):
//!
//! | import | signature |
//! |---|---|
//! | `global` | `() -> handle` of `globalThis` |
//! | `get` | `(obj, name_ptr, name_len) -> handle` |
//! | `set` | `(obj, name_ptr, name_len, value) -> i32` (1 = ok) |
//! | `call` | `(obj, name_ptr, name_len, args_ptr, args_len) -> handle` |
//! | `new` | `(ctor, args_ptr, args_len) -> handle` |
//! | `string` | `(ptr, len) -> handle` (UTF-8 -> JS string) |
//! | `number` | `(f64) -> handle` |
//! | `boolean` | `(i32) -> handle` |
//! | `to_f64` | `(handle) -> f64` |
//! | `string_len` | `(handle) -> usize` (UTF-8 byte length) |
//! | `string_copy` | `(handle, ptr, cap) -> usize` (writes UTF-8) |
//! | `is_nullish` | `(handle) -> i32` |
//! | `bytes` | `(ptr, len) -> handle` (a `Uint8ClampedArray` copy of wasm memory) |
//! | `copy_bytes` | `(array_handle, dst_ptr, len) -> i32` (copies a `Uint8Array`-like into wasm memory) |
//! | `release` | `(handle)` |
#![allow(unsafe_code)]

use std::fmt;

mod ffi {
    #[link(wasm_import_module = "ghostty_js")]
    unsafe extern "C" {
        pub fn global() -> u32;
        pub fn get(obj: u32, name: *const u8, len: usize) -> u32;
        pub fn set(obj: u32, name: *const u8, len: usize, value: u32) -> i32;
        pub fn call(obj: u32, name: *const u8, len: usize, args: *const u32, argc: usize) -> u32;
        pub fn new(ctor: u32, args: *const u32, argc: usize) -> u32;
        pub fn string(ptr: *const u8, len: usize) -> u32;
        pub fn number(v: f64) -> u32;
        pub fn boolean(v: i32) -> u32;
        pub fn to_f64(h: u32) -> f64;
        pub fn string_len(h: u32) -> usize;
        pub fn string_copy(h: u32, ptr: *mut u8, cap: usize) -> usize;
        pub fn is_nullish(h: u32) -> i32;
        pub fn bytes(ptr: *const u8, len: usize) -> u32;
        pub fn copy_bytes(h: u32, dst: *mut u8, len: usize) -> i32;
        pub fn release(h: u32);
    }
}

/// Errors from JS interop (`zig-js` `error.JSError` and friends).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The JS operation threw (or the handle was invalid).
    Js,
    /// The value is `null`/`undefined` where an object was required.
    Nullish,
    /// A returned string was not valid UTF-8.
    Utf8,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Js => f.write_str("javascript error"),
            Error::Nullish => f.write_str("value is null or undefined"),
            Error::Utf8 => f.write_str("invalid utf-8 from javascript"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// A reference to a JS value. Released on drop (zig-js `Object.deinit`).
#[derive(Debug, PartialEq, Eq)]
pub struct Object(u32);

/// An argument for [`Object::call`] / [`Object::new_instance`].
#[derive(Debug, Clone, Copy)]
pub enum Arg<'a> {
    Object(&'a Object),
    Str(&'a str),
    Num(f64),
    Bool(bool),
}

impl<'a> From<&'a Object> for Arg<'a> {
    fn from(v: &'a Object) -> Self {
        Arg::Object(v)
    }
}

impl<'a> From<&'a str> for Arg<'a> {
    fn from(v: &'a str) -> Self {
        Arg::Str(v)
    }
}

impl From<f64> for Arg<'_> {
    fn from(v: f64) -> Self {
        Arg::Num(v)
    }
}

impl From<f32> for Arg<'_> {
    fn from(v: f32) -> Self {
        Arg::Num(f64::from(v))
    }
}

impl From<u32> for Arg<'_> {
    fn from(v: u32) -> Self {
        Arg::Num(f64::from(v))
    }
}

impl From<bool> for Arg<'_> {
    fn from(v: bool) -> Self {
        Arg::Bool(v)
    }
}

fn check(h: u32) -> Result<Object> {
    if h == 0 {
        Err(Error::Js)
    } else {
        Ok(Object(h))
    }
}

/// Converts arguments to handles. Temporaries are released by [`Args::drop`].
struct Args {
    handles: Vec<u32>,
    owned: Vec<u32>,
}

impl Args {
    fn new(args: &[Arg<'_>]) -> Result<Args> {
        let mut out = Args {
            handles: Vec::with_capacity(args.len()),
            owned: Vec::new(),
        };
        for a in args {
            let h = match a {
                Arg::Object(o) => o.0,
                // SAFETY: pointer/length of a live &str; host copies it.
                Arg::Str(s) => unsafe { ffi::string(s.as_ptr(), s.len()) },
                // SAFETY: plain value call.
                Arg::Num(n) => unsafe { ffi::number(*n) },
                // SAFETY: plain value call.
                Arg::Bool(b) => unsafe { ffi::boolean(i32::from(*b)) },
            };
            if h == 0 {
                return Err(Error::Js);
            }
            if !matches!(a, Arg::Object(_)) {
                out.owned.push(h);
            }
            out.handles.push(h);
        }
        Ok(out)
    }
}

impl Drop for Args {
    fn drop(&mut self) {
        for h in &self.owned {
            // SAFETY: handle was created by us and not yet released.
            unsafe { ffi::release(*h) };
        }
    }
}

/// `js.global`: the `globalThis` object.
pub fn global() -> Result<Object> {
    // SAFETY: plain call into the host.
    check(unsafe { ffi::global() })
}

impl Object {
    /// Creates a JS string value.
    pub fn string(s: &str) -> Result<Object> {
        // SAFETY: pointer/length of a live &str; host copies it.
        check(unsafe { ffi::string(s.as_ptr(), s.len()) })
    }

    /// Creates a `Uint8ClampedArray` holding a copy of `data` (what zig-js
    /// callers built by viewing wasm memory with `new Uint8ClampedArray(
    /// memory.buffer, ptr, len)`; copying also works for shared memory).
    pub fn clamped_array(data: &[u8]) -> Result<Object> {
        // SAFETY: pointer/length of a live slice; host copies it.
        check(unsafe { ffi::bytes(data.as_ptr(), data.len()) })
    }

    /// Gives up ownership of the handle without releasing it (the host is
    /// expected to release it, like zig-js `deleteValue`).
    pub fn into_raw(self) -> u32 {
        let h = self.0;
        std::mem::forget(self);
        h
    }

    /// Gets a property as an object reference (also used for primitives,
    /// which are boxed on the host side).
    pub fn get(&self, name: &str) -> Result<Object> {
        // SAFETY: pointer/length of a live &str; host copies it.
        check(unsafe { ffi::get(self.0, name.as_ptr(), name.len()) })
    }

    /// Gets a property that must be an object (not null/undefined).
    pub fn get_object(&self, name: &str) -> Result<Object> {
        let v = self.get(name)?;
        if v.is_nullish() {
            return Err(Error::Nullish);
        }
        Ok(v)
    }

    /// Gets a numeric property.
    pub fn get_f64(&self, name: &str) -> Result<f64> {
        Ok(self.get(name)?.as_f64())
    }

    /// Gets a numeric property as `f32` (zig `get(f32, ...)`).
    pub fn get_f32(&self, name: &str) -> Result<f32> {
        Ok(self.get_f64(name)? as f32)
    }

    /// Gets a string property.
    pub fn get_string(&self, name: &str) -> Result<String> {
        self.get(name)?.to_string_value()
    }

    /// Sets a property.
    pub fn set(&self, name: &str, value: Arg<'_>) -> Result<()> {
        let args = Args::new(&[value])?;
        // SAFETY: pointer/length of a live &str; handle is valid.
        let ok = unsafe { ffi::set(self.0, name.as_ptr(), name.len(), args.handles[0]) };
        if ok == 1 { Ok(()) } else { Err(Error::Js) }
    }

    /// Calls a method and returns the result (which may be nullish).
    pub fn call(&self, name: &str, args: &[Arg<'_>]) -> Result<Object> {
        let args = Args::new(args)?;
        // SAFETY: name and args pointers are live for the call.
        check(unsafe {
            ffi::call(
                self.0,
                name.as_ptr(),
                name.len(),
                args.handles.as_ptr(),
                args.handles.len(),
            )
        })
    }

    /// `new this(...args)` (zig-js `Object.new`).
    pub fn new_instance(&self, args: &[Arg<'_>]) -> Result<Object> {
        let args = Args::new(args)?;
        // SAFETY: args pointer is live for the call.
        check(unsafe { ffi::new(self.0, args.handles.as_ptr(), args.handles.len()) })
    }

    pub fn is_nullish(&self) -> bool {
        // SAFETY: plain handle call.
        unsafe { ffi::is_nullish(self.0) != 0 }
    }

    /// Numeric value of this object (`Number(v)` on the host).
    pub fn as_f64(&self) -> f64 {
        // SAFETY: plain handle call.
        unsafe { ffi::to_f64(self.0) }
    }

    /// String value of this object (`String(v)` on the host).
    pub fn to_string_value(&self) -> Result<String> {
        // SAFETY: plain handle call.
        let len = unsafe { ffi::string_len(self.0) };
        let mut buf = vec![0u8; len];
        // SAFETY: `buf` has `len` writable bytes.
        let n = unsafe { ffi::string_copy(self.0, buf.as_mut_ptr(), buf.len()) };
        buf.truncate(n);
        String::from_utf8(buf).map_err(|_| Error::Utf8)
    }

    /// Copies this (`Uint8Array`/`Uint8ClampedArray`) object into `dst`.
    pub fn copy_bytes_to(&self, dst: &mut [u8]) -> Result<()> {
        // SAFETY: `dst` is `dst.len()` writable bytes in wasm memory.
        let ok = unsafe { ffi::copy_bytes(self.0, dst.as_mut_ptr(), dst.len()) };
        if ok == 1 { Ok(()) } else { Err(Error::Js) }
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        // SAFETY: we own this handle.
        unsafe { ffi::release(self.0) };
    }
}
