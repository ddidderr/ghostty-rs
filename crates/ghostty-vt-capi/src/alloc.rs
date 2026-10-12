//! The C allocator interface (ZIG/src/lib/allocator.zig, `include/ghostty/vt/allocator.h`).
//!
//! Wrapper objects handed out through the C API (parsers, events, encoders)
//! and the SGR parameter copy are allocated with the caller's allocator when
//! one is given and with the Rust global allocator otherwise (Zig: libc
//! `c_allocator` / `wasm_allocator`). Parser-owned byte storage (the OSC
//! payload buffer, [`CByteStorage`]) and the copies handed back to C (the OSC
//! window title, [`CBlock`]) are allocated through the same allocator, so the
//! caller's limits and failures govern them: a failed allocation surfaces as
//! `GHOSTTY_OUT_OF_MEMORY` where Zig can report it (object creation) and
//! invalidates the OSC being parsed where Zig's writer fails.
//!
//! Transient values built while a command is parsed (the payload copies the
//! per-command parsers make before `ghostty_osc_end` returns) use the global
//! allocator; nothing of them outlives that call.
#![allow(unsafe_code)]

use std::{alloc::Layout, ffi::c_void, ptr::NonNull};

use ghostty_vt::osc::{ByteStorage, StorageFull};

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

/// A block of bytes allocated through an [`Alloc`] (Zig: an allocator-owned
/// slice). Freed with the same allocator on drop.
pub struct CBlock {
    alloc: Alloc,
    ptr: NonNull<u8>,
    cap: usize,
}

impl CBlock {
    /// Allocate `cap` bytes (alignment 1); `None` when the allocator fails.
    pub fn new(alloc: Alloc, cap: usize) -> Option<CBlock> {
        let layout = Layout::from_size_align(cap, 1).ok()?;
        let ptr = NonNull::new(alloc.alloc_raw(layout))?;
        if cap > 0 {
            // SAFETY: freshly allocated `cap` bytes; zeroing makes the whole
            // block initialized so slices over it are sound.
            unsafe { ptr.as_ptr().write_bytes(0, cap) };
        }
        Some(CBlock { alloc, ptr, cap })
    }

    /// A copy of `bytes` followed by a NUL terminator.
    pub fn from_bytes_z(alloc: Alloc, bytes: &[u8]) -> Option<CBlock> {
        let mut b = CBlock::new(alloc, bytes.len() + 1)?;
        b.as_mut_slice()[..bytes.len()].copy_from_slice(bytes);
        b.as_mut_slice()[bytes.len()] = 0;
        Some(b)
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr.as_ptr()
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `ptr` is a live, zero-initialized allocation of `cap` bytes.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.cap) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: as above, with unique access through `&mut self`.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.cap) }
    }
}

impl Drop for CBlock {
    fn drop(&mut self) {
        // SAFETY: allocated by `self.alloc` with this layout in `new`.
        unsafe {
            self.alloc.free_raw(
                self.ptr.as_ptr(),
                Layout::from_size_align_unchecked(self.cap, 1),
            );
        }
    }
}

/// OSC payload storage backed by the C allocator. Starts with `base` bytes
/// (the parser's fixed buffer, allocated together with the parser so creation
/// can fail with out-of-memory) and grows by allocate-copy-free.
pub struct CByteStorage {
    block: CBlock,
    len: usize,
    base: usize,
}

// SAFETY: the storage is uniquely owned by its parser. Moving a parser to
// another thread is allowed under the same contract as Zig's: the caller's
// allocator must be usable from the thread that uses the parser.
unsafe impl Send for CByteStorage {}

impl std::fmt::Debug for CByteStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CByteStorage")
            .field("len", &self.len)
            .field("cap", &self.block.capacity())
            .field("base", &self.base)
            .finish()
    }
}

impl CByteStorage {
    /// `None` if the initial `base` bytes cannot be allocated.
    pub fn new(alloc: Alloc, base: usize) -> Option<CByteStorage> {
        Some(CByteStorage {
            block: CBlock::new(alloc, base)?,
            len: 0,
            base,
        })
    }

    fn grow_to(&mut self, needed: usize) -> Result<(), StorageFull> {
        if needed <= self.block.capacity() {
            return Ok(());
        }
        let cap = needed.max(self.block.capacity().saturating_mul(2));
        let mut new = CBlock::new(self.block.alloc, cap).ok_or(StorageFull)?;
        new.as_mut_slice()[..self.len].copy_from_slice(&self.block.as_slice()[..self.len]);
        self.block = new;
        Ok(())
    }
}

impl ByteStorage for CByteStorage {
    fn bytes(&self) -> &[u8] {
        &self.block.as_slice()[..self.len]
    }

    fn try_reserve(&mut self, additional: usize) -> Result<(), StorageFull> {
        self.grow_to(self.len.checked_add(additional).ok_or(StorageFull)?)
    }

    fn try_extend(&mut self, bytes: &[u8]) -> Result<(), StorageFull> {
        self.grow_to(self.len.checked_add(bytes.len()).ok_or(StorageFull)?)?;
        self.block.as_mut_slice()[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        Ok(())
    }

    fn reset(&mut self) {
        self.len = 0;
        // Give back memory acquired by growth (Zig deinits the allocating
        // writer). If the small block cannot be allocated, keep the big one.
        if self.block.capacity() > self.base
            && let Some(b) = CBlock::new(self.block.alloc, self.base)
        {
            self.block = b;
        }
    }

    fn clone_box(&self) -> Box<dyn ByteStorage> {
        Box::new(self.bytes().to_vec())
    }
}
