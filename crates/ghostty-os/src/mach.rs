//! macOS virtual memory tags and a VM-tagged page allocator. Port of
//! `os/mach.zig`.

/// macOS virtual memory tags for use with `mach_vm_map`/`mmap`. These
/// identify memory regions in tools like vmmap and Instruments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmTag(pub u8);

impl VmTag {
    pub const APPLICATION_SPECIFIC_1: VmTag = VmTag(240);
    pub const APPLICATION_SPECIFIC_2: VmTag = VmTag(241);
    pub const APPLICATION_SPECIFIC_3: VmTag = VmTag(242);
    pub const APPLICATION_SPECIFIC_4: VmTag = VmTag(243);
    pub const APPLICATION_SPECIFIC_5: VmTag = VmTag(244);
    pub const APPLICATION_SPECIFIC_6: VmTag = VmTag(245);
    pub const APPLICATION_SPECIFIC_7: VmTag = VmTag(246);
    pub const APPLICATION_SPECIFIC_8: VmTag = VmTag(247);
    pub const APPLICATION_SPECIFIC_9: VmTag = VmTag(248);
    pub const APPLICATION_SPECIFIC_10: VmTag = VmTag(249);
    pub const APPLICATION_SPECIFIC_11: VmTag = VmTag(250);
    pub const APPLICATION_SPECIFIC_12: VmTag = VmTag(251);
    pub const APPLICATION_SPECIFIC_13: VmTag = VmTag(252);
    pub const APPLICATION_SPECIFIC_14: VmTag = VmTag(253);
    pub const APPLICATION_SPECIFIC_15: VmTag = VmTag(254);
    pub const APPLICATION_SPECIFIC_16: VmTag = VmTag(255);

    /// Converts the tag to the format expected by `mach_vm_map`/`mmap`'s fd
    /// argument. Equivalent to the C macro `VM_MAKE_TAG(tag)`.
    pub fn make(self) -> i32 {
        (u32::from(self.0) << 24) as i32
    }
}

/// A page allocator that tags all allocated memory with a [`VmTag`]
/// (Zig: `taggedPageAllocator`, based on the Zig 0.15.2 PageAllocator
/// POSIX implementation).
#[cfg(target_vendor = "apple")]
#[allow(unsafe_code)]
pub mod tagged {
    use super::VmTag;

    #[derive(Debug, Clone, Copy)]
    pub struct TaggedPageAllocator {
        pub tag: VmTag,
    }

    fn page_size() -> usize {
        // SAFETY: sysconf has no preconditions.
        unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
    }

    fn align_forward(n: usize, align: usize) -> usize {
        (n + align - 1) & !(align - 1)
    }

    impl TaggedPageAllocator {
        pub fn new(tag: VmTag) -> Self {
            Self { tag }
        }

        /// Map `n` bytes aligned to `alignment` (a power of two). Returns
        /// null on failure.
        ///
        /// # Safety
        /// `n` must be non-zero; release the result with [`Self::unmap`]
        /// using the same `n`.
        pub unsafe fn map(&self, n: usize, alignment: usize) -> *mut u8 {
            let page = page_size();
            if n >= usize::MAX - page {
                return std::ptr::null_mut();
            }
            let aligned_len = align_forward(n, page);
            let max_drop_len = alignment - alignment.min(page);
            let overalloc_len = if max_drop_len <= aligned_len - n {
                aligned_len
            } else {
                align_forward(aligned_len + max_drop_len, page)
            };
            // SAFETY: anonymous private mapping; the tag is smuggled via the
            // fd argument as `mach_vm_map` expects.
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    overalloc_len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    self.tag.make(),
                    0,
                )
            };
            if ptr == libc::MAP_FAILED {
                return std::ptr::null_mut();
            }
            let base = ptr.cast::<u8>();
            let result = align_forward(base as usize, alignment) as *mut u8;
            // Unmap the extra bytes that were only requested in order to
            // guarantee alignment; they could be at the beginning, end or both.
            let drop_len = result as usize - base as usize;
            if drop_len != 0 {
                // SAFETY: [base, base+drop_len) lies inside our mapping.
                unsafe { libc::munmap(base.cast(), drop_len) };
            }
            let remaining_len = overalloc_len - drop_len;
            if remaining_len > aligned_len {
                // SAFETY: the tail lies inside our mapping.
                unsafe {
                    libc::munmap(result.add(aligned_len).cast(), remaining_len - aligned_len);
                }
            }
            result
        }

        /// # Safety
        /// `ptr`/`len` must come from [`Self::map`] with the same length.
        pub unsafe fn unmap(&self, ptr: *mut u8, len: usize) {
            let aligned = align_forward(len, page_size());
            // SAFETY: caller contract.
            unsafe { libc::munmap(ptr.cast(), aligned) };
        }

        /// Try to resize in place (Zig `realloc` with no `mremap` on
        /// Darwin): only shrinking within the mapping or staying on the same
        /// page count succeeds. Returns the (unchanged) pointer or null.
        ///
        /// # Safety
        /// `ptr`/`old_len` must come from [`Self::map`].
        pub unsafe fn realloc(&self, ptr: *mut u8, old_len: usize, new_len: usize) -> *mut u8 {
            let page = page_size();
            let new_aligned = align_forward(new_len, page);
            let old_aligned = align_forward(old_len, page);
            if new_aligned == old_aligned {
                return ptr;
            }
            if new_aligned < old_aligned {
                // SAFETY: the released tail lies inside the mapping.
                unsafe {
                    libc::munmap(ptr.add(new_aligned).cast(), old_aligned - new_aligned);
                }
                return ptr;
            }
            std::ptr::null_mut()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn map_unmap_roundtrip() {
            let a = TaggedPageAllocator::new(VmTag::APPLICATION_SPECIFIC_1);
            // SAFETY: test-only use following the documented contract.
            unsafe {
                let p = a.map(10_000, 16384);
                assert!(!p.is_null());
                assert_eq!(p as usize % 16384, 0);
                p.write(1);
                p.add(9_999).write(2);
                let q = a.realloc(p, 10_000, 4096);
                assert_eq!(q, p);
                a.unmap(p, 4096);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_tag_make() {
        assert_eq!(VmTag::APPLICATION_SPECIFIC_1.make(), (240u32 << 24) as i32);
        assert_eq!(VmTag::APPLICATION_SPECIFIC_16.make(), (255u32 << 24) as i32);
    }
}
