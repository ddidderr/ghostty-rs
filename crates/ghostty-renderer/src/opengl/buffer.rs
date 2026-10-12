//! OpenGL data storage for a certain set of equal types (`opengl/buffer.zig`).
use std::marker::PhantomData;

use bytemuck::Pod;

use super::gl::{self, BufferTarget, BufferUsage, Gl, RawBuffer};

/// Options for initializing a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub target: BufferTarget,
    pub usage: BufferUsage,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            target: BufferTarget::Array,
            usage: BufferUsage::DynamicDraw,
        }
    }
}

/// OpenGL data storage for a certain set of equal types. This is usually
/// used for vertex buffers, etc. This helpful wrapper makes it easy to
/// prealloc, shrink, grow, sync, buffers with OpenGL.
pub struct Buffer<T: Pod> {
    gl: Gl,

    /// Underlying GL buffer object.
    handle: RawBuffer,

    /// Options this buffer was allocated with.
    pub opts: Options,

    /// Current allocated length of the data store.
    /// Note this is the number of `T`s, not the size in bytes.
    pub len: usize,

    _marker: PhantomData<fn() -> T>,
}

impl<T: Pod> Buffer<T> {
    /// The raw GL buffer handle (for passing to render pass steps).
    pub fn raw(&self) -> RawBuffer {
        self.handle
    }

    /// Initialize a buffer with the given length pre-allocated.
    pub fn init(gl: &Gl, opts: Options, len: usize) -> gl::Result<Self> {
        let buffer = gl.create_buffer()?;
        let this = Buffer {
            gl: gl.clone(),
            handle: buffer,
            opts,
            len,
            _marker: PhantomData,
        };
        // On error `this` is dropped, which destroys the buffer.
        gl.bind_buffer(opts.target, Some(buffer));
        let r = gl.buffer_data_size(opts.target, len * size_of::<T>(), opts.usage);
        gl.bind_buffer(opts.target, None);
        r?;
        Ok(this)
    }

    /// Init the buffer filled with the given data.
    pub fn init_fill(gl: &Gl, opts: Options, data: &[T]) -> gl::Result<Self> {
        let buffer = gl.create_buffer()?;
        // NOTE: Zig sets `len = data.len * @sizeOf(T)` here, which is a bug
        // (len is documented as a count of `T`s); we store the element count.
        let this = Buffer {
            gl: gl.clone(),
            handle: buffer,
            opts,
            len: data.len(),
            _marker: PhantomData,
        };
        gl.bind_buffer(opts.target, Some(buffer));
        let r = gl.buffer_data(opts.target, bytemuck::cast_slice(data), opts.usage);
        gl.bind_buffer(opts.target, None);
        r?;
        Ok(this)
    }

    /// Sync new contents to the buffer. The data is expected to be the
    /// complete contents of the buffer. If the amount of data is larger
    /// than the buffer length, the buffer will be reallocated.
    ///
    /// If the amount of data is smaller than the buffer length, the
    /// remaining data in the buffer is left untouched.
    pub fn sync(&mut self, data: &[T]) -> gl::Result<()> {
        self.sync_from_slices(&[data]).map(|_| ())
    }

    /// Like `Buffer::sync` but takes data from an array of slices, rather
    /// than a single slice. Returns the number of items synced.
    pub fn sync_from_slices(&mut self, lists: &[&[T]]) -> gl::Result<usize> {
        let target = self.opts.target;
        self.gl.bind_buffer(target, Some(self.handle));
        let r = self.sync_bound(lists);
        self.gl.bind_buffer(target, None);
        r
    }

    fn sync_bound(&mut self, lists: &[&[T]]) -> gl::Result<usize> {
        let target = self.opts.target;
        let total_len: usize = lists.iter().map(|l| l.len()).sum();

        // If we need more space than our buffer has, we need to reallocate.
        if total_len > self.len {
            // Reallocate the buffer to hold double what we require.
            self.len = total_len * 2;
            self.gl
                .buffer_data_size(target, self.len * size_of::<T>(), self.opts.usage)?;
        }

        // We can fit within the buffer so we can just replace bytes.
        let mut offset = 0;
        for list in lists {
            self.gl
                .buffer_sub_data(target, offset, bytemuck::cast_slice(list))?;
            offset += size_of_val(*list);
        }

        Ok(total_len)
    }
}

impl<T: Pod> Drop for Buffer<T> {
    fn drop(&mut self) {
        self.gl.delete_buffer(self.handle);
    }
}
