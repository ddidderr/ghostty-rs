//! JS interop for wasm. A Rust port of the `zig-js` package
//! (`mitchellh/zig-js`, `src/{main,ref,value,Object,extern}.zig`) which
//! Ghostty's Zig wasm code imports as `js`.
//!
//! This speaks exactly zig-js's ABI so the module runs with zig-js's own JS
//! host (`ZigJS` from `zig-js-glue`; vendored under
//! `crates/ghostty-wasm/js/zig-js`):
//!
//! - import module `"zig-js"`, functions `valueGet`, `valueSet`,
//!   `valueObjectCreate`, `valueStringCreate`, `valueStringLen`,
//!   `valueStringCopy`, `valueDeinit`, `valueNew`, `funcApply` (see [`ffi`]);
//! - a JS value crosses the boundary as a 64-bit [`Ref`]: a plain `f64` for
//!   numbers, otherwise a NaN whose low 32 bits are an id into the host's
//!   value table and bits 32-34 a type id (see [`Ref`]);
//! - the same semantics for [`Value`] (low level) and [`Object`] (high level):
//!   `get`/`set` only on object refs, `apply`/`new` only on function refs,
//!   arguments that are not already an [`Object`] are created for the call and
//!   released after it, results are converted per requested type and released
//!   unless the type retains the JS value (`Object`/`Value`).
//!
//! The wasm host also needs `memory` assigned on the `ZigJS` instance (that is
//! what [`runtime`]`.get::<Object>("memory")` reads).
//!
//! Differences from the Zig API are only those forced by Rust: generics and
//! traits ([`Get`], [`IntoValue`]) instead of `comptime T`, `Drop` instead of
//! explicit `deinit()` (use [`Object::into_id`] to hand ownership to the host
//! like Zig's `atlas_debug_canvas` does), and a `&[&dyn IntoValue]` argument
//! list instead of an anonymous tuple.
#![allow(unsafe_code)]

use std::fmt;

/// Raw imports, module `"zig-js"` (`extern.zig`).
mod ffi {
    #[link(wasm_import_module = "zig-js")]
    unsafe extern "C" {
        #[link_name = "valueGet"]
        pub fn value_get(out: *mut u64, id: u32, addr: *const u8, len: usize);
        #[link_name = "valueSet"]
        pub fn value_set(id: u32, addr: *const u8, len: usize, ref_ptr: *const u64);
        #[link_name = "valueObjectCreate"]
        pub fn value_object_create(out: *mut u64);
        #[link_name = "valueStringCreate"]
        pub fn value_string_create(out: *mut u64, addr: *const u8, len: usize);
        #[link_name = "valueStringLen"]
        pub fn value_string_len(id: u32) -> u32;
        #[link_name = "valueStringCopy"]
        pub fn value_string_copy(id: u32, addr: *mut u8, max: usize);
        #[link_name = "valueDeinit"]
        pub fn value_deinit(id: u32);
        #[link_name = "valueNew"]
        pub fn value_new(out: *mut u64, id: u32, args_ptr: *const u64, args_len: usize);
        #[link_name = "funcApply"]
        pub fn func_apply(
            out: *mut u64,
            func: u32,
            this_ptr: *const u64,
            args_ptr: *const u64,
            args_len: usize,
        );
    }
}

/// Errors that can occur in this package (`js.Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The wrong type for the receiver or the result type can't be assigned
    /// from the JS type when using the higher-level APIs like [`Object`].
    InvalidType,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InvalidType")
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Possible types of values (`js.Type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    Undefined,
    Null,
    Boolean,
    Number,
    Object,
    String,
    Symbol,
    Function,
}

/// Ref uniquely identifies a JS value (`ref.zig`). A JS value can't be
/// physically copied to the WASM environment so we represent it using a
/// 64-bit number into a table that is maintained on the JS side.
///
/// For a JS number (float64), the ref is literally the float64 (bitcasted).
/// For other types, we utilize the unused bits of the IEEE 754 NaN
/// representation to encode additional data into the ref. Bits 0-31 are used
/// as an ID and bits 32-34 are used as a type ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ref(pub u64);

/// NaN in IEEE-754 is `0b0111_1111_1111_<anything other than all zeroes>`. We
/// always force NaN to have a 1-bit set in the 4th byte from the MSB so that
/// we can use the lower 51 bits. This is the top 29 bits of the ref.
const NAN_HEAD: u64 = 0x7FF8_0000 >> 3;

/// The `type_id` field (bits 32-34).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
enum TypeId {
    Inferred = 0,
    Object = 1,
    String = 2,
    Symbol = 3,
    Function = 4,
}

impl Ref {
    /// Predefined refs.
    pub const NAN: Ref = Ref::predefined(0);
    pub const NULL: Ref = Ref::predefined(1);
    pub const TRUE: Ref = Ref::predefined(2);
    pub const FALSE: Ref = Ref::predefined(3);
    pub const UNDEFINED: Ref = Ref::predefined(4);
    pub const GLOBAL: Ref = Ref::predefined(5);
    pub const RUNTIME: Ref = Ref::predefined(6);

    const fn predefined(id: u32) -> Ref {
        Ref::new(id, TypeId::Inferred)
    }

    const fn new(id: u32, type_id: TypeId) -> Ref {
        Ref((NAN_HEAD << 35) | ((type_id as u64) << 32) | id as u64)
    }

    /// Bits 0-31.
    pub const fn id(self) -> u32 {
        self.0 as u32
    }

    const fn head(self) -> u64 {
        self.0 >> 35
    }

    /// Bits 32-34, `None` for the unused values 5-7.
    const fn type_id(self) -> Option<TypeId> {
        match (self.0 >> 32) & 0b111 {
            0 => Some(TypeId::Inferred),
            1 => Some(TypeId::Object),
            2 => Some(TypeId::String),
            3 => Some(TypeId::Symbol),
            4 => Some(TypeId::Function),
            _ => None,
        }
    }

    /// Returns the type of a ref.
    pub fn type_of(self) -> Type {
        // If we aren't a NaN then we have to be a number
        if self.head() != NAN_HEAD {
            return Type::Number;
        }

        match self.type_id() {
            Some(TypeId::Object) => Type::Object,
            Some(TypeId::String) => Type::String,
            Some(TypeId::Symbol) => Type::Symbol,
            Some(TypeId::Function) => Type::Function,
            Some(TypeId::Inferred) => match self.id() {
                0 => Type::Number,
                1 => Type::Null,
                2 | 3 => Type::Boolean,
                4 => Type::Undefined,
                5 | 6 => Type::Object,
                // Zig: `unreachable`.
                _ => Type::Undefined,
            },
            None => Type::Undefined,
        }
    }

    /// Returns true if the ref should be released.
    ///
    /// We don't need to release raw numbers because they're copied directly
    /// into our environment. And we don't need to release the predefined
    /// values because they're always retained.
    pub fn is_releasable(self) -> bool {
        self.head() == NAN_HEAD && self.type_id() != Some(TypeId::Inferred)
    }

    pub fn to_f64(self) -> f64 {
        debug_assert_eq!(self.type_of(), Type::Number);
        f64::from_bits(self.0)
    }
}

/// Only used with [`Value::string`]/[`IntoValue`] to denote a string type
/// (`js.String`; named `JsString` to not clash with `std::string::String`).
///
/// This is NOT a JS string. This is just a sentinel type so that we can
/// differentiate a byte slice and a "string" when trying to convert a Rust
/// value into a JS value.
#[derive(Debug, Clone, Copy)]
pub struct JsString<'a>(&'a [u8]);

impl<'a> JsString<'a> {
    /// `js.String.init`.
    pub fn init(x: &'a (impl AsRef<[u8]> + ?Sized)) -> Self {
        JsString(x.as_ref())
    }
}

/// Shortcut for [`JsString::init`] since this is a common operation
/// (`js.string`).
pub fn string(v: &(impl AsRef<[u8]> + ?Sized)) -> JsString<'_> {
    JsString::init(v)
}

/// Only used to denote an undefined value (`js.Undefined`).
#[derive(Debug, Clone, Copy)]
pub struct Undefined;

/// Denotes creating a new empty object (`js.Object{ .value = undefined }`
/// passed to `Value.init`).
#[derive(Debug, Clone, Copy)]
pub struct NewObject;

/// A value represents a JS value (`js.Value`). This is the low-level
/// "untyped" interface to any generic JS value. It is more ergonomic to use
/// the higher level wrappers such as [`Object`]. It is `Copy` and does not
/// release itself; call [`Value::deinit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Value(pub u64);

impl Value {
    pub const NAN: Value = Value(Ref::NAN.0);
    pub const NULL: Value = Value(Ref::NULL.0);
    pub const TRUE: Value = Value(Ref::TRUE.0);
    pub const FALSE: Value = Value(Ref::FALSE.0);
    pub const UNDEFINED: Value = Value(Ref::UNDEFINED.0);
    pub const GLOBAL: Value = Value(Ref::GLOBAL.0);
    /// Gives access to the "this" value for the ZigJS JS class. This makes it
    /// possible to access the memory for example.
    pub const RUNTIME: Value = Value(Ref::RUNTIME.0);

    /// `Value.init` for floats (any numeric type goes through `f64`).
    pub fn number(x: f64) -> Value {
        if x.is_nan() {
            return Value::NAN;
        }
        Value(x.to_bits())
    }

    /// `Value.init` for bools.
    pub fn boolean(x: bool) -> Value {
        if x { Value::TRUE } else { Value::FALSE }
    }

    /// `Value.init(Object{})`: creates a new empty object.
    pub fn object() -> Value {
        let mut result = 0u64;
        // SAFETY: `result` is a valid out pointer.
        unsafe { ffi::value_object_create(&raw mut result) };
        Value(result)
    }

    /// `Value.init(String)`. The bytes are copied to the JS side.
    pub fn string(s: JsString<'_>) -> Value {
        let mut result = 0u64;
        // SAFETY: `s.0` is a live slice (host copies it); `result` is valid.
        unsafe { ffi::value_string_create(&raw mut result, s.0.as_ptr(), s.0.len()) };
        Value(result)
    }

    fn as_ref(self) -> Ref {
        Ref(self.0)
    }

    /// Deinitializes the value, allowing the JS environment to GC the value.
    pub fn deinit(self) {
        // We avoid releasing values that aren't releasable. This avoids
        // crossing the js/wasm boundary for a bit of performance.
        if self.as_ref().is_releasable() {
            // SAFETY: plain call.
            unsafe { ffi::value_deinit(self.as_ref().id()) };
        }
    }

    /// Get the value of a property of an object.
    pub fn get(self, n: &str) -> Result<Value> {
        if self.type_of() != Type::Object {
            return Err(Error::InvalidType);
        }
        let mut result = 0u64;
        // SAFETY: `n` is live for the call; `result` is a valid out pointer.
        unsafe { ffi::value_get(&raw mut result, self.as_ref().id(), n.as_ptr(), n.len()) };
        Ok(Value(result))
    }

    /// Set the value of a property on an object.
    pub fn set(self, n: &str, v: Value) -> Result<()> {
        if self.type_of() != Type::Object {
            return Err(Error::InvalidType);
        }
        // SAFETY: `n` and `v` are live for the call.
        unsafe { ffi::value_set(self.as_ref().id(), n.as_ptr(), n.len(), &raw const v.0) };
        Ok(())
    }

    /// Call this value as a function.
    pub fn apply(self, this: Value, args: &[Value]) -> Result<Value> {
        if self.type_of() != Type::Function {
            return Err(Error::InvalidType);
        }
        let mut result = 0u64;
        // SAFETY: `Value` is `repr(transparent)` over u64; the slice is live
        // for the call.
        unsafe {
            ffi::func_apply(
                &raw mut result,
                self.as_ref().id(),
                &raw const this.0,
                args.as_ptr().cast::<u64>(),
                args.len(),
            );
        }
        Ok(Value(result))
    }

    /// Call "new" on this value like a constructor.
    pub fn new_instance(self, args: &[Value]) -> Result<Value> {
        if self.type_of() != Type::Function {
            return Err(Error::InvalidType);
        }
        let mut result = 0u64;
        // SAFETY: as in `apply`.
        unsafe {
            ffi::value_new(
                &raw mut result,
                self.as_ref().id(),
                args.as_ptr().cast::<u64>(),
                args.len(),
            );
        }
        Ok(Value(result))
    }

    /// Returns the bool value if this is a boolean.
    pub fn as_bool(self) -> Result<bool> {
        if self.type_of() != Type::Boolean {
            return Err(Error::InvalidType);
        }
        Ok(self == Value::TRUE)
    }

    /// Returns the float value if this is a number.
    pub fn float(self) -> Result<f64> {
        if self.type_of() != Type::Number {
            return Err(Error::InvalidType);
        }
        Ok(f64::from_bits(self.0))
    }

    /// Returns the UTF-8 encoded string value.
    pub fn to_bytes(self) -> Result<Vec<u8>> {
        if self.type_of() != Type::String {
            return Err(Error::InvalidType);
        }

        // Get the length and allocate our buffer
        let id = self.as_ref().id();
        // SAFETY: plain call.
        let len = unsafe { ffi::value_string_len(id) } as usize;
        let mut buf = vec![0u8; len];

        // Copy the string into the buffer
        // SAFETY: `buf` has `len` writable bytes.
        unsafe { ffi::value_string_copy(id, buf.as_mut_ptr(), buf.len()) };
        Ok(buf)
    }

    /// Returns the type of this value.
    pub fn type_of(self) -> Type {
        self.as_ref().type_of()
    }
}

/// Anything `Value.init` accepts. [`IntoValue::owned`] says whether the
/// resulting value is created for the call (and so must be released after it,
/// like Zig's `Object.call` does for every argument that isn't a `js.Object`).
pub trait IntoValue {
    fn to_value(&self) -> Value;

    fn owned(&self) -> bool {
        true
    }
}

macro_rules! into_value_number {
    ($($t:ty),*) => {$(
        impl IntoValue for $t {
            fn to_value(&self) -> Value {
                #[allow(clippy::cast_lossless, clippy::unnecessary_cast)]
                Value::number(*self as f64)
            }
        }
    )*};
}
// Ints, floats and (via `usize`) pointers: all JS numbers are f64.
into_value_number!(f64, f32, u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl IntoValue for bool {
    fn to_value(&self) -> Value {
        Value::boolean(*self)
    }
}
impl IntoValue for Undefined {
    fn to_value(&self) -> Value {
        Value::UNDEFINED
    }
}
impl IntoValue for NewObject {
    fn to_value(&self) -> Value {
        Value::object()
    }
}
impl IntoValue for JsString<'_> {
    fn to_value(&self) -> Value {
        Value::string(*self)
    }
}
impl IntoValue for Value {
    fn to_value(&self) -> Value {
        *self
    }

    // Deviation: Zig releases a plain `js.Value` argument of `call`/`new`
    // but not in `set`; we treat a raw `Value` as caller-owned.
    fn owned(&self) -> bool {
        false
    }
}
impl IntoValue for Object {
    fn to_value(&self) -> Value {
        self.value
    }

    fn owned(&self) -> bool {
        false
    }
}
impl<T: IntoValue + ?Sized> IntoValue for &T {
    fn to_value(&self) -> Value {
        (**self).to_value()
    }

    fn owned(&self) -> bool {
        (**self).owned()
    }
}

/// Result types for [`Object::get`]/[`Object::call`] (zig `Get(T)`).
pub trait Get: Sized {
    /// True if we want to retain the js value.
    const RETAINS: bool = false;

    /// Convert a non-null/undefined value.
    fn from_value(v: Value) -> Result<Self>;

    /// What a null/undefined value converts to (`InvalidType` unless the
    /// type is optional or void).
    fn from_nullish() -> Result<Self> {
        Err(Error::InvalidType)
    }
}

impl Get for () {
    fn from_value(_: Value) -> Result<Self> {
        Err(Error::InvalidType)
    }

    fn from_nullish() -> Result<Self> {
        Ok(())
    }
}
impl Get for Value {
    const RETAINS: bool = true;

    fn from_value(v: Value) -> Result<Self> {
        Ok(v)
    }
}
impl Get for Object {
    const RETAINS: bool = true;

    fn from_value(v: Value) -> Result<Self> {
        match v.type_of() {
            Type::Object | Type::Function => Ok(Object { value: v }),
            _ => Err(Error::InvalidType),
        }
    }
}
impl Get for bool {
    fn from_value(v: Value) -> Result<Self> {
        v.as_bool()
    }
}
impl Get for String {
    /// zig `js.String` => `[]u8` (here lossily decoded UTF-8).
    fn from_value(v: Value) -> Result<Self> {
        Ok(String::from_utf8_lossy(&v.to_bytes()?).into_owned())
    }
}
macro_rules! get_number {
    ($($t:ty),*) => {$(
        impl Get for $t {
            fn from_value(v: Value) -> Result<Self> {
                // Zig: `@floatCast` / `@intFromFloat`.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Ok(v.float()? as $t)
            }
        }
    )*};
}
get_number!(f32, f64, u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

/// An optional result: null or undefined become `None`.
impl<T: Get> Get for Option<T> {
    const RETAINS: bool = T::RETAINS;

    fn from_value(v: Value) -> Result<Self> {
        T::from_value(v).map(Some)
    }

    fn from_nullish() -> Result<Self> {
        Ok(None)
    }
}

/// Zig `Object.convertValue`.
fn convert_value<T: Get>(v: Value) -> Result<T> {
    let r = match v.type_of() {
        Type::Null | Type::Undefined => T::from_nullish(),
        _ => T::from_value(v),
    };
    // Release the value unless the result retains it (and conversion worked;
    // Zig's `errdefer` releases it on error).
    if !T::RETAINS || r.is_err() {
        v.deinit();
    }
    r
}

/// Object is a JS object (`js.Object`). This is a higher-level abstraction
/// over [`Value`] and is the recommended interface. It releases the JS value
/// on drop (Zig: `deinit()`).
#[derive(Debug, PartialEq, Eq)]
pub struct Object {
    pub value: Value,
}

/// The global "this" value as the high-level Object API (`js.global`).
pub fn global() -> Object {
    Object {
        value: Value::GLOBAL,
    }
}

/// The runtime value is the "this" value for the ZigJS class in JS. This is
/// useful to directly accessing the `WebAssembly.Memory` property to
/// implement things like JS-to-WASM memcpy (`js.runtime`).
pub fn runtime() -> Object {
    Object {
        value: Value::RUNTIME,
    }
}

impl Drop for Object {
    fn drop(&mut self) {
        self.value.deinit();
    }
}

impl Object {
    /// Gives up ownership without releasing, returning the host ref id (Zig
    /// code does this by reading `ref.id` and skipping `deinit`). The host
    /// releases it with `ZigJS.deleteValue(id)`.
    pub fn into_id(self) -> u32 {
        let id = Ref(self.value.0).id();
        std::mem::forget(self);
        id
    }

    /// Get a property value of type `T` (`get` / `getAlloc`).
    ///
    /// If the type is `Option`, then null or undefined JS values will return
    /// `None`. If it is not, null or undefined will return an error.
    pub fn get<T: Get>(&self, n: &str) -> Result<T> {
        let v = self.value.get(n)?;
        convert_value(v)
    }

    /// Set a value of a property. The type of `v` must be able to turn into
    /// a value (see [`IntoValue`]).
    pub fn set(&self, n: &str, v: impl IntoValue) -> Result<()> {
        let js_value = v.to_value();
        let r = self.value.set(n, js_value);
        if v.owned() {
            js_value.deinit();
        }
        r
    }

    /// Call a function on a object. This will set the "this" parameter to
    /// the object properly (`call` / `callAlloc`).
    pub fn call<T: Get>(&self, n: &str, args: &[&dyn IntoValue]) -> Result<T> {
        // Build our arguments.
        let js_args = Args::new(args);

        // Invoke
        let f = self.value.get(n)?;
        let r = f.apply(self.value, &js_args.values);
        f.deinit();
        convert_value(r?)
    }

    /// Construct an object from this value.
    pub fn new_instance(&self, args: &[&dyn IntoValue]) -> Result<Object> {
        let js_args = Args::new(args);
        let v = self.value.new_instance(&js_args.values)?;
        Ok(Object { value: v })
    }
}

/// Converted call arguments; the ones created for the call are released when
/// this drops, those already objects are left to the caller.
struct Args<'a> {
    values: Vec<Value>,
    src: &'a [&'a dyn IntoValue],
}

impl<'a> Args<'a> {
    fn new(src: &'a [&'a dyn IntoValue]) -> Self {
        Args {
            values: src.iter().map(|a| IntoValue::to_value(*a)).collect(),
            src,
        }
    }
}

impl Drop for Args<'_> {
    fn drop(&mut self) {
        for (v, a) in self.values.iter().zip(self.src) {
            if a.owned() {
                v.deinit();
            }
        }
    }
}
