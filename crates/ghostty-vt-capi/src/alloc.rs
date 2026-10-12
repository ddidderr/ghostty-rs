//! The C allocator interface (ZIG/src/lib/allocator.zig, `include/ghostty/vt/allocator.h`).
//!
//! Wrapper objects handed out through the C API (parsers, events, encoders)
//! and the SGR parameter copy are allocated with the caller's allocator when
//! one is given and with the Rust global allocator otherwise (Zig: libc
//! `c_allocator` / `wasm_allocator`). Interior Rust collections (e.g. the OSC
//! parser's buffer) always use the global allocator.
#![allow(unsafe_code)]

use std::{alloc::Layout, ffi::c_void, ptr::NonNull};

/// C: `GhosttyAllocatorVtable`
#[repr(C)]
pub struct AllocatorVtable {
    pub alloc: unsafe extern "C" fn(
        ctx: *mut c_void,
        len: usize,
        alignment: u8,
        ret_addr: usize,
    ) -> *mut c_void,
    pub resize: unsafe extern "C" fn(
        ctx: *mut c_void,
        memory: *mut c_void,
        memory_len: usize,
        alignment: u8,
        new_len: usize,
        ret_addr: usize,
    ) -> bool,
    pub remap: unsafe extern "C" fn(
        ctx: *mut c_void,
        memory: *mut c_void,
        memory_len: usize,
        alignment: u8,
        new_len: usize,
        ret_addr: usize,
    ) -> *mut c_void,
    pub free: unsafe extern "C" fn(
        ctx: *mut c_void,
        memory: *mut c_void,
        memory_len: usize,
        alignment: u8,
        ret_addr: usize,
    ),
}

/// C: `GhosttyAllocator`
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Allocator {
    pub ctx: *mut c_void,
    pub vtable: *const AllocatorVtable,
}

/// A possibly-absent C allocator (`None` = default allocator). Stored inside
/// every object so `free` releases memory with the allocator that created it.
#[derive(Clone, Copy)]
pub struct Alloc(Option<Allocator>);

impl Alloc {
    /// From the `const GhosttyAllocator *` parameter of a `*_new` function.
    ///
    /// # Safety
    /// `ptr` must be null or point to a valid [`Allocator`] with a valid vtable.
    pub unsafe fn from_c(ptr: *const Allocator) -> Alloc {
        // SAFETY: caller guarantees validity when non-null; we copy the struct
        // so the original need not outlive this call (the vtable/ctx must).
        Alloc(unsafe { ptr.as_ref() }.copied())
    }

    /// Allocate `layout`; null on failure. Zero-sized layouts return a dangling
    /// aligned pointer without calling the allocator.
    pub fn alloc_raw(self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return layout.align() as *mut u8;
        }
        match self.0 {
            // SAFETY: non-zero size layout.
            None => unsafe { std::alloc::alloc(layout) },
            // SAFETY: `from_c` contract: vtable is valid.
            Some(a) => unsafe {
                ((*a.vtable).alloc)(
                    a.ctx,
                    layout.size(),
                    layout.align().trailing_zeros() as u8,
                    0,
                )
                .cast()
            },
        }
    }

    /// Free memory returned by [`Alloc::alloc_raw`] with the same layout.
    ///
    /// # Safety
    /// `ptr` must have been allocated by this allocator with `layout`.
    pub unsafe fn free_raw(self, ptr: *mut u8, layout: Layout) {
        if layout.size() == 0 {
            return;
        }
        match self.0 {
            // SAFETY: per the function contract.
            None => unsafe { std::alloc::dealloc(ptr, layout) },
            // SAFETY: per the function contract and `from_c`.
            Some(a) => unsafe {
                ((*a.vtable).free)(
                    a.ctx,
                    ptr.cast(),
                    layout.size(),
                    layout.align().trailing_zeros() as u8,
                    0,
                );
            },
        }
    }

    /// Move `value` into freshly allocated memory (Zig `alloc.create`).
    pub fn create<T>(self, value: T) -> Option<NonNull<T>> {
        let p = NonNull::new(self.alloc_raw(Layout::new::<T>()))?.cast::<T>();
        // SAFETY: freshly allocated, correctly sized and aligned.
        unsafe { p.as_ptr().write(value) };
        Some(p)
    }

    /// Drop and free an object made by [`Alloc::create`] (Zig `alloc.destroy`).
    ///
    /// # Safety
    /// `p` must come from `create::<T>` on this allocator and not be used again.
    pub unsafe fn destroy<T>(self, p: NonNull<T>) {
        // SAFETY: per the function contract.
        unsafe {
            std::ptr::drop_in_place(p.as_ptr());
            self.free_raw(p.as_ptr().cast(), Layout::new::<T>());
        }
    }
}
