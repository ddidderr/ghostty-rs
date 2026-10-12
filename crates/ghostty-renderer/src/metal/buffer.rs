//! Metal data storage for a certain set of equal types (`metal/buffer.zig`).
use std::marker::PhantomData;

use bytemuck::Pod;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::NSRange;
use objc2_metal::{MTLBuffer, MTLDevice as _, MTLResourceOptions};

use super::{Device, Error};
use crate::{
    graphics_api::{ApiError, GpuBuffer},
    metal_layout::{ResourceOptions, StorageMode},
};

/// Options for initializing a buffer.
#[derive(Clone)]
pub struct Options {
    /// MTLDevice
    pub device: Device,
    pub resource_options: ResourceOptions,
}

impl Options {
    fn mtl_options(&self) -> MTLResourceOptions {
        MTLResourceOptions(self.resource_options.bits() as usize)
    }
}

/// Metal data storage for a certain set of equal types. This is usually
/// used for vertex buffers, etc. This helpful wrapper makes it easy to
/// prealloc, shrink, grow, sync, buffers with Metal.
pub struct Buffer<T: Pod> {
    /// The options this buffer was initialized with.
    opts: Options,

    /// The underlying MTLBuffer object.
    buffer: Retained<ProtocolObject<dyn MTLBuffer>>,

    /// The allocated length of the buffer. Note that this is the number
    /// of `T`s not the size in bytes.
    pub len: usize,

    _marker: PhantomData<fn() -> T>,
}

// SAFETY: an MTLBuffer is safe to use from any thread; CPU writes through
// `contents` are serialized by `&mut self` and by the swap chain, which never
// hands a buffer to the CPU while the GPU is using it.
unsafe impl<T: Pod> Send for Buffer<T> {}
// SAFETY: see above; shared access only reads the handle.
unsafe impl<T: Pod> Sync for Buffer<T> {}

impl<T: Pod> Buffer<T> {
    /// The underlying MTLBuffer.
    pub fn buffer(&self) -> &ProtocolObject<dyn MTLBuffer> {
        &self.buffer
    }

    /// A retained handle to the underlying MTLBuffer (what render pass steps
    /// bind).
    pub fn raw(&self) -> Retained<ProtocolObject<dyn MTLBuffer>> {
        self.buffer.clone()
    }

    /// Initialize a buffer with the given length pre-allocated.
    pub fn init(opts: Options, len: usize) -> Result<Self, Error> {
        let buffer = opts
            .device
            .newBufferWithLength_options(len * size_of::<T>(), opts.mtl_options())
            .ok_or(Error::MetalFailed)?;
        Ok(Buffer {
            buffer,
            opts,
            len,
            _marker: PhantomData,
        })
    }

    /// Init the buffer filled with the given data.
    pub fn init_fill(opts: Options, data: &[T]) -> Result<Self, Error> {
        let bytes: &[u8] = bytemuck::cast_slice(data);
        // newBufferWithBytes rejects a null pointer / zero length, so empty
        // data allocates an (empty) buffer of length 0 via the length path.
        let buffer = if bytes.is_empty() {
            opts.device
                .newBufferWithLength_options(0, opts.mtl_options())
        } else {
            let ptr = std::ptr::NonNull::from(bytes).cast();
            // SAFETY: `ptr` points to `bytes.len()` readable bytes.
            unsafe {
                opts.device
                    .newBufferWithBytes_length_options(ptr, bytes.len(), opts.mtl_options())
            }
        }
        .ok_or(Error::MetalFailed)?;
        Ok(Buffer {
            buffer,
            opts,
            len: data.len(),
            _marker: PhantomData,
        })
    }

    /// Sync new contents to the buffer. The data is expected to be the
    /// complete contents of the buffer. If the amount of data is larger
    /// than the buffer length, the buffer will be reallocated.
    ///
    /// If the amount of data is smaller than the buffer length, the
    /// remaining data in the buffer is left untouched.
    pub fn sync(&mut self, data: &[T]) -> Result<(), Error> {
        self.sync_from_slices(&[data]).map(|_| ())
    }

    /// Like `Buffer::sync` but takes data from an array of slices, rather
    /// than a single slice. Returns the number of items synced.
    pub fn sync_from_slices(&mut self, lists: &[&[T]]) -> Result<usize, Error> {
        let total_len: usize = lists.iter().map(|l| l.len()).sum();

        // If we need more bytes than our buffer has, we need to reallocate.
        let req_bytes = total_len * size_of::<T>();
        let avail_bytes = self.buffer.length();
        if req_bytes > avail_bytes {
            // Allocate a new buffer with enough to hold double what we
            // require (the old buffer is released when it is replaced).
            //
            // NOTE: Zig multiplies this size by `@sizeOf(T)` a second time,
            // over-allocating; we allocate exactly double.
            let size = req_bytes * 2;
            self.buffer = self
                .opts
                .device
                .newBufferWithLength_options(size, self.opts.mtl_options())
                .ok_or(Error::MetalFailed)?;
            self.len = total_len * 2;
        }

        // We can fit within the buffer so we can just replace bytes.
        let dst = self.buffer.contents().as_ptr().cast::<u8>();
        let mut offset = 0;
        for list in lists {
            let src: &[u8] = bytemuck::cast_slice(list);
            // SAFETY: `dst` points to a live buffer of at least `req_bytes`
            // bytes (checked/reallocated above) and `offset + src.len()` never
            // exceeds `req_bytes`; the regions can't overlap because the
            // buffer is GPU memory owned by us.
            unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), dst.add(offset), src.len()) };
            offset += src.len();
        }

        // If we're using the managed resource storage mode, then
        // we need to signal Metal to synchronize the buffer data.
        //
        // Ref: https://developer.apple.com/documentation/metal/synchronizing-a-managed-resource-in-macos?language=objc
        if self.opts.resource_options.storage_mode == StorageMode::Managed {
            self.buffer.didModifyRange(NSRange::new(0, req_bytes));
        }

        Ok(total_len)
    }
}

impl<T: Pod> GpuBuffer<super::Metal, T> for Buffer<T> {
    fn raw(&self) -> Retained<ProtocolObject<dyn MTLBuffer>> {
        Buffer::raw(self)
    }

    fn sync(&mut self, data: &[T]) -> Result<(), ApiError> {
        Buffer::sync(self, data).map_err(ApiError::new)
    }

    fn sync_from_slices(&mut self, lists: &[&[T]]) -> Result<usize, ApiError> {
        Buffer::sync_from_slices(self, lists).map_err(ApiError::new)
    }
}
