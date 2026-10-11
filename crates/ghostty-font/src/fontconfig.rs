//! Minimal safe wrappers over the fontconfig C API (the subset used by
//! font discovery). Equivalent of `pkg/fontconfig` in the Zig tree.
//!
//! All raw FFI is confined to this module.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_int};
use std::ptr::{self, NonNull};
use std::sync::Arc;

use fontconfig_sys as sys;
use fontconfig_sys::constants as fc;

/// Errors from fontconfig calls (mirrors `fontconfig.Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("fontconfig: no match")]
    NoMatch,
    #[error("fontconfig: type mismatch")]
    TypeMismatch,
    #[error("fontconfig: no id")]
    NoId,
    #[error("fontconfig: out of memory")]
    OutOfMemory,
    #[error("fontconfig failed")]
    FontconfigFailed,
}

fn result_to_error(r: u32) -> Result<(), Error> {
    match r {
        sys::FcResultMatch => Ok(()),
        sys::FcResultNoMatch => Err(Error::NoMatch),
        sys::FcResultTypeMismatch => Err(Error::TypeMismatch),
        sys::FcResultNoId => Err(Error::NoId),
        _ => Err(Error::OutOfMemory),
    }
}

/// Font weight constants (`FC_WEIGHT_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Weight {
    Bold = fc::FC_WEIGHT_BOLD,
}

/// Font slant constants (`FC_SLANT_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Slant {
    Italic = fc::FC_SLANT_ITALIC,
}

/// Font spacing constants (`FC_*` spacing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Spacing {
    Mono = fc::FC_MONO,
}

/// Pattern property names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Property {
    Family,
    Style,
    FullName,
    File,
    Index,
    Charset,
    Lang,
    Size,
    Weight,
    Slant,
    Spacing,
}

impl Property {
    fn cstr(self) -> &'static CStr {
        match self {
            Property::Family => fc::FC_FAMILY,
            Property::Style => fc::FC_STYLE,
            Property::FullName => fc::FC_FULLNAME,
            Property::File => fc::FC_FILE,
            Property::Index => fc::FC_INDEX,
            Property::Charset => fc::FC_CHARSET,
            Property::Lang => fc::FC_LANG,
            Property::Size => fc::FC_SIZE,
            Property::Weight => fc::FC_WEIGHT,
            Property::Slant => fc::FC_SLANT,
            Property::Spacing => fc::FC_SPACING,
        }
    }
}

/// Initialize fontconfig (safe to call multiple times and concurrently).
pub fn init() -> bool {
    // SAFETY: FcInit has no preconditions.
    unsafe { sys::FcInit() != 0 }
}

/// An owned `FcCharSet`.
pub struct CharSet(NonNull<sys::FcCharSet>);

impl CharSet {
    pub fn create() -> CharSet {
        // SAFETY: no preconditions; null (OOM) is handled.
        let p = unsafe { sys::FcCharSetCreate() };
        CharSet(NonNull::new(p).expect("FcCharSetCreate out of memory"))
    }

    pub fn add_char(&mut self, cp: u32) -> bool {
        // SAFETY: self.0 is a valid, uniquely owned charset.
        unsafe { sys::FcCharSetAddChar(self.0.as_ptr(), cp) != 0 }
    }

    pub fn has_char(&self, cp: u32) -> bool {
        // SAFETY: self.0 is a valid charset.
        unsafe { sys::FcCharSetHasChar(self.0.as_ptr(), cp) != 0 }
    }
}

impl Drop for CharSet {
    fn drop(&mut self) {
        // SAFETY: we own the charset and destroy it once.
        unsafe { sys::FcCharSetDestroy(self.0.as_ptr()) }
    }
}

/// An owned `FcLangSet`.
pub struct LangSet(NonNull<sys::FcLangSet>);

impl LangSet {
    pub fn create() -> LangSet {
        // SAFETY: no preconditions.
        let p = unsafe { sys::FcLangSetCreate() };
        LangSet(NonNull::new(p).expect("FcLangSetCreate out of memory"))
    }

    pub fn add_lang(&mut self, lang: &str) -> bool {
        let Ok(c) = CString::new(lang) else {
            return false;
        };
        // SAFETY: valid langset and NUL-terminated string.
        unsafe { sys::FcLangSetAdd(self.0.as_ptr(), c.as_ptr().cast()) != 0 }
    }

    pub fn has_lang(&self, lang: &str) -> bool {
        has_lang_raw(self.0.as_ptr(), lang)
    }
}

impl Drop for LangSet {
    fn drop(&mut self) {
        // SAFETY: we own the langset and destroy it once.
        unsafe { sys::FcLangSetDestroy(self.0.as_ptr()) }
    }
}

fn has_lang_raw(ls: *const sys::FcLangSet, lang: &str) -> bool {
    let Ok(c) = CString::new(lang) else {
        return false;
    };
    // SAFETY: `ls` is a valid langset (callers guarantee), string is NUL-terminated.
    unsafe { sys::FcLangSetHasLang(ls, c.as_ptr().cast()) == sys::FcLangEqual }
}

/// An owned `FcPattern`.
pub struct Pattern(NonNull<sys::FcPattern>);

// SAFETY: patterns are never mutated after construction in this crate
// (only read through &self), and fontconfig patterns are plain data.
unsafe impl Send for Pattern {}
// SAFETY: see above; all &self methods are read-only.
unsafe impl Sync for Pattern {}

impl Pattern {
    pub fn create() -> Pattern {
        // SAFETY: no preconditions.
        let p = unsafe { sys::FcPatternCreate() };
        Pattern(NonNull::new(p).expect("FcPatternCreate out of memory"))
    }

    fn as_ptr(&self) -> *mut sys::FcPattern {
        self.0.as_ptr()
    }

    pub fn add_string(&mut self, prop: Property, v: &str) -> bool {
        let Ok(c) = CString::new(v) else {
            return false;
        };
        // SAFETY: valid pattern; property and value are NUL-terminated (value is copied).
        unsafe {
            sys::FcPatternAddString(self.as_ptr(), prop.cstr().as_ptr(), c.as_ptr().cast()) != 0
        }
    }

    pub fn add_integer(&mut self, prop: Property, v: i32) -> bool {
        // SAFETY: valid pattern, NUL-terminated property.
        unsafe { sys::FcPatternAddInteger(self.as_ptr(), prop.cstr().as_ptr(), v) != 0 }
    }

    pub fn add_charset(&mut self, prop: Property, cs: &CharSet) -> bool {
        // SAFETY: valid pattern and charset (fontconfig takes its own reference).
        unsafe { sys::FcPatternAddCharSet(self.as_ptr(), prop.cstr().as_ptr(), cs.0.as_ptr()) != 0 }
    }

    /// Get the string value at index `id`.
    pub fn get_string(&self, prop: Property, id: u32) -> Result<String, Error> {
        let mut out: *mut sys::FcChar8 = ptr::null_mut();
        // SAFETY: valid pattern; out is a valid out pointer.
        let r = unsafe {
            sys::FcPatternGetString(
                self.as_ptr(),
                prop.cstr().as_ptr(),
                id as c_int,
                &raw mut out,
            )
        };
        result_to_error(r)?;
        if out.is_null() {
            return Err(Error::NoMatch);
        }
        // SAFETY: fontconfig returned a NUL-terminated string owned by the pattern.
        let s = unsafe { CStr::from_ptr(out.cast()) };
        Ok(s.to_string_lossy().into_owned())
    }

    pub fn get_integer(&self, prop: Property, id: u32) -> Result<i32, Error> {
        let mut out: c_int = 0;
        // SAFETY: valid pattern; out is a valid out pointer.
        let r = unsafe {
            sys::FcPatternGetInteger(
                self.as_ptr(),
                prop.cstr().as_ptr(),
                id as c_int,
                &raw mut out,
            )
        };
        result_to_error(r)?;
        Ok(out)
    }

    /// Returns whether `cp` is in the charset property (index 0).
    pub fn charset_has_char(&self, cp: u32) -> Result<bool, Error> {
        let mut out: *mut sys::FcCharSet = ptr::null_mut();
        // SAFETY: valid pattern; out is a valid out pointer.
        let r = unsafe {
            sys::FcPatternGetCharSet(
                self.as_ptr(),
                Property::Charset.cstr().as_ptr(),
                0,
                &raw mut out,
            )
        };
        result_to_error(r)?;
        if out.is_null() {
            return Err(Error::NoMatch);
        }
        // SAFETY: the charset is owned by the pattern, which outlives this call.
        Ok(unsafe { sys::FcCharSetHasChar(out, cp) != 0 })
    }

    /// Returns whether the langset property (index 0) equals `lang`.
    pub fn langset_has_lang(&self, lang: &str) -> Result<bool, Error> {
        let mut out: *mut sys::FcLangSet = ptr::null_mut();
        // SAFETY: valid pattern; out is a valid out pointer.
        let r = unsafe {
            sys::FcPatternGetLangSet(
                self.as_ptr(),
                Property::Lang.cstr().as_ptr(),
                0,
                &raw mut out,
            )
        };
        result_to_error(r)?;
        if out.is_null() {
            return Err(Error::NoMatch);
        }
        Ok(has_lang_raw(out, lang))
    }

    pub fn default_substitute(&mut self) {
        // SAFETY: valid pattern.
        unsafe { sys::FcDefaultSubstitute(self.as_ptr()) }
    }
}

impl Drop for Pattern {
    fn drop(&mut self) {
        // SAFETY: we own the pattern reference and release it once.
        unsafe { sys::FcPatternDestroy(self.as_ptr()) }
    }
}

/// An owned `FcFontSet` as returned by `FcFontSort`.
pub struct FontSet(NonNull<sys::FcFontSet>);

impl FontSet {
    pub fn len(&self) -> usize {
        // SAFETY: valid font set.
        let n = unsafe { (*self.0.as_ptr()).nfont };
        usize::try_from(n).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Raw pattern pointer at `i`, borrowed from the set.
    fn font(&self, i: usize) -> *mut sys::FcPattern {
        assert!(i < self.len());
        // SAFETY: bounds checked; `fonts` has `nfont` entries.
        unsafe { *(*self.0.as_ptr()).fonts.add(i) }
    }
}

impl Drop for FontSet {
    fn drop(&mut self) {
        // SAFETY: we own the set and destroy it once.
        unsafe { sys::FcFontSetDestroy(self.0.as_ptr()) }
    }
}

// SAFETY: the set is only read after construction.
unsafe impl Send for FontSet {}
// SAFETY: see above.
unsafe impl Sync for FontSet {}

/// An owned `FcConfig`.
pub struct Config(NonNull<sys::FcConfig>);

// SAFETY: fontconfig configs are internally reference counted and
// thread safe for the read operations we perform.
unsafe impl Send for Config {}
// SAFETY: see above.
unsafe impl Sync for Config {}

impl Config {
    /// `FcInitLoadConfigAndFonts`.
    pub fn init_load_config_and_fonts() -> Option<Config> {
        // SAFETY: no preconditions.
        NonNull::new(unsafe { sys::FcInitLoadConfigAndFonts() }).map(Config)
    }

    /// `FcConfigSubstituteWithPat(config, pat, NULL, FcMatchPattern)`.
    pub fn substitute_with_pat_pattern(&self, pat: &mut Pattern) -> bool {
        // SAFETY: valid config and pattern; null pattern-for-match is allowed.
        unsafe {
            sys::FcConfigSubstituteWithPat(
                self.0.as_ptr(),
                pat.as_ptr(),
                ptr::null_mut(),
                sys::FcMatchPattern,
            ) != 0
        }
    }

    /// `FcFontSort`. Returns the sorted set when the result is a match.
    pub fn font_sort(&self, pat: &mut Pattern, trim: bool) -> Result<FontSet, Error> {
        let mut result: sys::FcResult = 0;
        // SAFETY: valid config/pattern; null charset out-pointer is allowed.
        let fs = unsafe {
            sys::FcFontSort(
                self.0.as_ptr(),
                pat.as_ptr(),
                c_int::from(trim),
                ptr::null_mut(),
                &raw mut result,
            )
        };
        let fs = NonNull::new(fs).map(FontSet);
        if result != sys::FcResultMatch {
            return Err(Error::FontconfigFailed);
        }
        fs.ok_or(Error::FontconfigFailed)
    }

    /// `FcFontRenderPrepare(config, pat, set[i])`.
    pub fn font_render_prepare(
        &self,
        pat: &mut Pattern,
        set: &FontSet,
        i: usize,
    ) -> Result<Pattern, Error> {
        // SAFETY: valid config/pattern/font pattern; result is a new owned pattern.
        let p = unsafe { sys::FcFontRenderPrepare(self.0.as_ptr(), pat.as_ptr(), set.font(i)) };
        NonNull::new(p).map(Pattern).ok_or(Error::FontconfigFailed)
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        // SAFETY: we own the config reference and release it once.
        unsafe { sys::FcConfigDestroy(self.0.as_ptr()) }
    }
}

/// Shared handle to a config.
pub type SharedConfig = Arc<Config>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn langset_create() {
        let fs = LangSet::create();
        assert!(!fs.has_lang("und-zsye"));
    }

    #[test]
    fn langset_has_lang_exact_match() {
        let mut fs = LangSet::create();
        assert!(fs.add_lang("en-US"));
        assert!(fs.has_lang("en-US"));
        let mut emoji = LangSet::create();
        assert!(emoji.add_lang("und-zsye"));
        assert!(emoji.has_lang("und-zsye"));
        assert!(!emoji.has_lang("en-US"));
    }

    #[test]
    fn charset_add_has() {
        let mut cs = CharSet::create();
        assert!(!cs.has_char('A' as u32));
        assert!(cs.add_char('A' as u32));
        assert!(cs.has_char('A' as u32));
    }

    #[test]
    fn pattern_roundtrip() {
        let mut p = Pattern::create();
        assert!(p.add_string(Property::Family, "Foo"));
        assert!(p.add_integer(Property::Index, 3));
        assert_eq!("Foo", p.get_string(Property::Family, 0).unwrap());
        assert_eq!(3, p.get_integer(Property::Index, 0).unwrap());
        assert_eq!(Err(Error::NoMatch), p.get_string(Property::File, 0));
    }
}
