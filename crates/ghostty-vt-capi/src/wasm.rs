//! Wasm allocation helpers (ZIG/src/lib/allocator/convenience.zig and
//! `wasm_alloc_attribute` in ZIG/src/terminal/c/sgr.zig,
//! `include/ghostty/vt/wasm.h`).
//!
//! JavaScript hosts cannot easily create the pointers the C API wants, so
//! upstream exports these helpers on wasm. They always use the default
//! allocator. They are exported (`no_mangle`) only when compiling for wasm.
#![allow(unsafe_code)]

use std::{alloc::Layout, ffi::c_void};

use crate::sgr::SgrAttribute;

fn alloc_layout(layout: Layout) -> *mut u8 {
    if layout.size() == 0 {
        return layout.align() as *mut u8;
    }
    // SAFETY: non-zero size.
    unsafe { std::alloc::alloc(layout) }
}

/// # Safety
/// `ptr` must be null or come from `alloc_layout` with the same `layout`.
unsafe fn free_layout(ptr: *mut u8, layout: Layout) {
    if !ptr.is_null() && layout.size() != 0 {
        // SAFETY: per the contract.
        unsafe { std::alloc::dealloc(ptr, layout) };
    }
}

/// Allocate one `void *` (e.g. an out-parameter for `*_new`).
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn ghostty_wasm_alloc_opaque() -> *mut *mut c_void {
    alloc_layout(Layout::new::<*mut c_void>()).cast()
}

/// # Safety
/// `ptr` must be null or from `ghostty_wasm_alloc_opaque`.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn ghostty_wasm_free_opaque(ptr: *mut *mut c_void) {
    // SAFETY: per the contract.
    unsafe { free_layout(ptr.cast(), Layout::new::<*mut c_void>()) };
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn ghostty_wasm_alloc_u8_array(len: usize) -> *mut u8 {
    Layout::array::<u8>(len).map_or(std::ptr::null_mut(), alloc_layout)
}

/// # Safety
/// `ptr` must be null or from `ghostty_wasm_alloc_u8_array(len)`.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn ghostty_wasm_free_u8_array(ptr: *mut u8, len: usize) {
    if let Ok(l) = Layout::array::<u8>(len) {
        // SAFETY: per the contract.
        unsafe { free_layout(ptr, l) };
    }
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn ghostty_wasm_alloc_u16_array(len: usize) -> *mut u16 {
    Layout::array::<u16>(len).map_or(std::ptr::null_mut(), |l| alloc_layout(l).cast())
}

/// # Safety
/// `ptr` must be null or from `ghostty_wasm_alloc_u16_array(len)`.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn ghostty_wasm_free_u16_array(ptr: *mut u16, len: usize) {
    if let Ok(l) = Layout::array::<u16>(len) {
        // SAFETY: per the contract.
        unsafe { free_layout(ptr.cast(), l) };
    }
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn ghostty_wasm_alloc_u8() -> *mut u8 {
    alloc_layout(Layout::new::<u8>())
}

/// # Safety
/// `ptr` must be null or from `ghostty_wasm_alloc_u8`.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn ghostty_wasm_free_u8(ptr: *mut u8) {
    // SAFETY: per the contract.
    unsafe { free_layout(ptr, Layout::new::<u8>()) };
}

#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn ghostty_wasm_alloc_usize() -> *mut usize {
    alloc_layout(Layout::new::<usize>()).cast()
}

/// # Safety
/// `ptr` must be null or from `ghostty_wasm_alloc_usize`.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn ghostty_wasm_free_usize(ptr: *mut usize) {
    // SAFETY: per the contract.
    unsafe { free_layout(ptr.cast(), Layout::new::<usize>()) };
}

/// Allocate a `GhosttySgrAttribute` (zero-initialized).
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub extern "C" fn ghostty_wasm_alloc_sgr_attribute() -> *mut SgrAttribute {
    let p = alloc_layout(Layout::new::<SgrAttribute>()).cast::<SgrAttribute>();
    if !p.is_null() {
        // SAFETY: freshly allocated, correctly sized; all-zero is a valid value.
        unsafe { p.write_bytes(0, 1) };
    }
    p
}

/// # Safety
/// `attr` must be null or from `ghostty_wasm_alloc_sgr_attribute`.
#[cfg_attr(target_arch = "wasm32", unsafe(no_mangle))]
pub unsafe extern "C" fn ghostty_wasm_free_sgr_attribute(attr: *mut SgrAttribute) {
    // SAFETY: per the contract.
    unsafe { free_layout(attr.cast(), Layout::new::<SgrAttribute>()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_and_free_round_trip() {
        // SAFETY: pointers come from the matching allocators.
        unsafe {
            let p = ghostty_wasm_alloc_u8_array(16);
            assert!(!p.is_null());
            p.write_bytes(7, 16);
            ghostty_wasm_free_u8_array(p, 16);

            let z = ghostty_wasm_alloc_u8_array(0);
            ghostty_wasm_free_u8_array(z, 0);

            let u = ghostty_wasm_alloc_u16_array(5);
            ghostty_wasm_free_u16_array(u, 5);
            let o = ghostty_wasm_alloc_opaque();
            ghostty_wasm_free_opaque(o);
            let b = ghostty_wasm_alloc_u8();
            ghostty_wasm_free_u8(b);
            let s = ghostty_wasm_alloc_usize();
            ghostty_wasm_free_usize(s);
            let a = ghostty_wasm_alloc_sgr_attribute();
            assert_eq!((*a).tag, 0);
            ghostty_wasm_free_sgr_attribute(a);
        }
    }
}
