//! Thin safe wrapper over `glow` (the Rust equivalent of Ghostty's
//! `pkg/opengl`).
//!
//! This is the only module in the crate that contains `unsafe` code: every
//! `glow::HasContext` method is an `unsafe fn` because GL calls are only valid
//! with a current context on the calling thread. The invariants are
//! documented once here:
//!
//! * A [`Gl`] is only constructed from a context the caller asserts is (or
//!   will be) current on the thread that uses it (`Gl::from_loader`).
//! * All object handles are created from, and only ever passed back to, the
//!   same `Gl`.
//! * Pointer-carrying calls (`buffer_data`, `tex_image_2d`, ...) are only given
//!   Rust slices, so lengths are always valid for the pointed memory.

use std::{ffi::c_void, sync::Arc};

use glow::HasContext;

/// The GL error enum (`pkg/opengl/errors.zig`), plus shader build errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("GL_INVALID_ENUM")]
    InvalidEnum,
    #[error("GL_INVALID_VALUE")]
    InvalidValue,
    #[error("GL_INVALID_OPERATION")]
    InvalidOperation,
    #[error("GL_INVALID_FRAMEBUFFER_OPERATION")]
    InvalidFramebufferOperation,
    #[error("GL_OUT_OF_MEMORY")]
    OutOfMemory,
    #[error("unknown GL error {0:#x}")]
    Unknown(u32),
    /// Shader compilation or program link failure, with the driver info log.
    #[error("shader compile/link failed: {0}")]
    CompileFailed(String),
    /// Creating an object failed (glGen*/glCreate* returned an error).
    #[error("failed to create GL object: {0}")]
    CreateFailed(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Handle to a loaded GL context. Cheap to clone (shared).
#[derive(Clone)]
pub struct Gl {
    ctx: Arc<glow::Context>,
}

impl std::fmt::Debug for Gl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gl").finish_non_exhaustive()
    }
}

// Re-exported raw handle types.
pub use glow::{
    Buffer as RawBuffer,
    Framebuffer as RawFramebuffer,
    Program as RawProgram,
    Renderbuffer as RawRenderbuffer,
    Sampler as RawSampler,
    Texture as RawTexture,
    VertexArray as RawVertexArray,
};

/// Enum for buffer binding targets (`gl.Buffer.Target`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum BufferTarget {
    Array = glow::ARRAY_BUFFER,
    ElementArray = glow::ELEMENT_ARRAY_BUFFER,
    Uniform = glow::UNIFORM_BUFFER,
    Storage = glow::SHADER_STORAGE_BUFFER,
}

/// Buffer usage hints (`gl.Buffer.Usage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum BufferUsage {
    StreamDraw = glow::STREAM_DRAW,
    StreamRead = glow::STREAM_READ,
    StreamCopy = glow::STREAM_COPY,
    StaticDraw = glow::STATIC_DRAW,
    StaticRead = glow::STATIC_READ,
    StaticCopy = glow::STATIC_COPY,
    DynamicDraw = glow::DYNAMIC_DRAW,
    DynamicRead = glow::DYNAMIC_READ,
    DynamicCopy = glow::DYNAMIC_COPY,
}

/// Texture binding targets (`gl.Texture.Target`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TextureTarget {
    D1 = glow::TEXTURE_1D,
    D2 = glow::TEXTURE_2D,
    D3 = glow::TEXTURE_3D,
    D1Array = glow::TEXTURE_1D_ARRAY,
    D2Array = glow::TEXTURE_2D_ARRAY,
    Rectangle = glow::TEXTURE_RECTANGLE,
    CubeMap = glow::TEXTURE_CUBE_MAP,
    Buffer = glow::TEXTURE_BUFFER,
    D2Multisample = glow::TEXTURE_2D_MULTISAMPLE,
    D2MultisampleArray = glow::TEXTURE_2D_MULTISAMPLE_ARRAY,
}

/// Internal format enum for texture images (`gl.Texture.InternalFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum InternalFormat {
    Red = glow::RED,
    Rgb = glow::RGB8,
    Rgba = glow::RGBA8,
    Srgb = glow::SRGB8,
    Srgba = glow::SRGB8_ALPHA8,
    RgbaCompressed = glow::COMPRESSED_RGBA_BPTC_UNORM,
    SrgbaCompressed = glow::COMPRESSED_SRGB_ALPHA_BPTC_UNORM,
}

/// Format for texture images (`gl.Texture.Format`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PixelFormat {
    Red = glow::RED,
    Rgb = glow::RGB,
    Rgba = glow::RGBA,
    Bgra = glow::BGRA,
}

/// Minification filter for textures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MinFilter {
    Nearest = glow::NEAREST as i32,
    Linear = glow::LINEAR as i32,
    NearestMipmapNearest = glow::NEAREST_MIPMAP_NEAREST as i32,
    LinearMipmapNearest = glow::LINEAR_MIPMAP_NEAREST as i32,
    NearestMipmapLinear = glow::NEAREST_MIPMAP_LINEAR as i32,
    LinearMipmapLinear = glow::LINEAR_MIPMAP_LINEAR as i32,
}

/// Magnification filter for textures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum MagFilter {
    Nearest = glow::NEAREST as i32,
    Linear = glow::LINEAR as i32,
}

/// Texture coordinate wrapping mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Wrap {
    ClampToEdge = glow::CLAMP_TO_EDGE as i32,
    ClampToBorder = glow::CLAMP_TO_BORDER as i32,
    MirroredRepeat = glow::MIRRORED_REPEAT as i32,
    Repeat = glow::REPEAT as i32,
}

/// Primitive types we support drawing (same subset as Metal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Primitive {
    Point = glow::POINTS,
    Line = glow::LINES,
    LineStrip = glow::LINE_STRIP,
    Triangle = glow::TRIANGLES,
    TriangleStrip = glow::TRIANGLE_STRIP,
}

/// Framebuffer binding targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FramebufferTarget {
    Framebuffer = glow::FRAMEBUFFER,
    Draw = glow::DRAW_FRAMEBUFFER,
    Read = glow::READ_FRAMEBUFFER,
}

/// Framebuffer attachment points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Attachment {
    Color0 = glow::COLOR_ATTACHMENT0,
    Depth = glow::DEPTH_ATTACHMENT,
    Stencil = glow::STENCIL_ATTACHMENT,
    DepthStencil = glow::DEPTH_STENCIL_ATTACHMENT,
}

/// A previous framebuffer binding, restored by [`Gl::unbind_framebuffer`].
#[derive(Debug, Clone, Copy)]
pub struct FramebufferBinding {
    pub target: FramebufferTarget,
    previous: Option<RawFramebuffer>,
}

/// Component type of a vertex attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttribType {
    U8,
    U16,
    U32,
    I8,
    I16,
    I32,
    F16,
    F32,
    F64,
}

impl Gl {
    /// Load GL function pointers with `loader` (e.g. `eglGetProcAddress`, or
    /// for GTK's GLArea the global loader) and wrap the resulting context.
    ///
    /// `loader` receives GL symbol names and returns the function address or
    /// null. The GL context must be current on this thread and must stay
    /// current (on whichever thread uses this `Gl`) while it is used.
    pub fn from_loader<F>(mut loader: F) -> Gl
    where
        F: FnMut(&str) -> *const c_void,
    {
        // SAFETY: glow documents that the loader must return valid function
        // pointers for the current context (or null); the caller supplies a
        // real GL loader. The returned context is only used with that context
        // current.
        let ctx = unsafe { glow::Context::from_loader_function(|s| loader(s)) };
        Gl { ctx: Arc::new(ctx) }
    }

    /// Like [`Gl::from_loader`] but also installs Ghostty's debug-message
    /// callback (`glDebugMessageCallback`) on the context.
    pub fn from_loader_with_debug<F>(mut loader: F) -> Gl
    where
        F: FnMut(&str) -> *const c_void,
    {
        // SAFETY: see `from_loader`.
        let mut ctx = unsafe { glow::Context::from_loader_function(|s| loader(s)) };
        // SAFETY: the context is current (caller contract); the callback is a
        // plain Rust closure with no captured state.
        unsafe {
            ctx.debug_message_callback(debug_message);
        }
        Gl { ctx: Arc::new(ctx) }
    }

    /// The (major, minor) of the loaded GL context.
    pub fn version(&self) -> (u32, u32) {
        let v = self.ctx.version();
        (v.major, v.minor)
    }

    /// Raw `glow` context, for operations the wrapper doesn't cover.
    pub fn raw(&self) -> &glow::Context {
        &self.ctx
    }

    /// `glGetError`: the error (if any) from the last OpenGL operation.
    pub fn get_error(&self) -> Result<()> {
        // SAFETY: see module docs.
        let e = unsafe { self.ctx.get_error() };
        match e {
            glow::NO_ERROR => Ok(()),
            glow::INVALID_ENUM => Err(Error::InvalidEnum),
            glow::INVALID_VALUE => Err(Error::InvalidValue),
            glow::INVALID_OPERATION => Err(Error::InvalidOperation),
            glow::INVALID_FRAMEBUFFER_OPERATION => Err(Error::InvalidFramebufferOperation),
            glow::OUT_OF_MEMORY => Err(Error::OutOfMemory),
            other => Err(Error::Unknown(other)),
        }
    }

    /// Enable a capability (`glEnable`).
    pub fn enable(&self, cap: u32) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.enable(cap) };
        self.get_error()
    }

    /// Disable a capability (`glDisable`).
    pub fn disable(&self, cap: u32) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.disable(cap) };
        self.get_error()
    }

    pub fn blend_func(&self, src: u32, dst: u32) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.blend_func(src, dst) };
        self.get_error()
    }

    pub fn viewport(&self, x: i32, y: i32, w: i32, h: i32) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.viewport(x, y, w, h) };
        self.get_error()
    }

    /// Returns the current viewport `[x, y, w, h]`.
    pub fn get_viewport(&self) -> [i32; 4] {
        let mut out = [0i32; 4];
        // SAFETY: GL_VIEWPORT writes exactly four ints into the slice.
        unsafe { self.ctx.get_parameter_i32_slice(glow::VIEWPORT, &mut out) };
        out
    }

    pub fn pixel_store_i32(&self, mode: u32, value: i32) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.pixel_store_i32(mode, value) };
        self.get_error()
    }

    pub fn clear_color(&self, r: f32, g: f32, b: f32, a: f32) {
        // SAFETY: see module docs.
        unsafe { self.ctx.clear_color(r, g, b, a) };
    }

    pub fn clear(&self, mask: u32) {
        // SAFETY: see module docs.
        unsafe { self.ctx.clear(mask) };
    }

    pub fn finish(&self) {
        // SAFETY: see module docs.
        unsafe { self.ctx.finish() };
    }

    pub fn flush(&self) {
        // SAFETY: see module docs.
        unsafe { self.ctx.flush() };
    }

    pub fn draw_arrays_instanced(
        &self,
        mode: Primitive,
        first: i32,
        count: i32,
        instances: i32,
    ) -> Result<()> {
        // SAFETY: see module docs; vertex data comes from bound GL buffers.
        unsafe {
            self.ctx
                .draw_arrays_instanced(mode as u32, first, count, instances);
        }
        self.get_error()
    }

    /// `glBlitFramebuffer` with the currently bound read/draw framebuffers.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_framebuffer(
        &self,
        src: (i32, i32, i32, i32),
        dst: (i32, i32, i32, i32),
        mask: u32,
        filter: u32,
    ) {
        // SAFETY: see module docs.
        unsafe {
            self.ctx.blit_framebuffer(
                src.0, src.1, src.2, src.3, dst.0, dst.1, dst.2, dst.3, mask, filter,
            );
        }
    }

    /// Read RGBA8 pixels from the currently bound read framebuffer.
    pub fn read_pixels_rgba8(&self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        let mut out = vec![0u8; (w.max(0) as usize) * (h.max(0) as usize) * 4];
        // SAFETY: `out` is exactly w*h*4 bytes, which is what RGBA/UNSIGNED_BYTE
        // reads at the default pack alignment of 4 (rows are a multiple of 4).
        unsafe {
            self.ctx.read_pixels(
                x,
                y,
                w,
                h,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut out)),
            );
        }
        out
    }

    // ------------------------------------------------------------ buffers

    pub fn create_buffer(&self) -> Result<RawBuffer> {
        // SAFETY: see module docs.
        unsafe { self.ctx.create_buffer() }.map_err(Error::CreateFailed)
    }

    pub fn delete_buffer(&self, b: RawBuffer) {
        // SAFETY: see module docs.
        unsafe { self.ctx.delete_buffer(b) };
    }

    pub fn bind_buffer(&self, target: BufferTarget, b: Option<RawBuffer>) {
        // SAFETY: see module docs.
        unsafe { self.ctx.bind_buffer(target as u32, b) };
    }

    pub fn bind_buffer_base(&self, target: BufferTarget, idx: u32, b: RawBuffer) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.bind_buffer_base(target as u32, idx, Some(b)) };
        self.get_error()
    }

    /// `glBufferData` with a null pointer: allocate `size` bytes.
    pub fn buffer_data_size(
        &self,
        target: BufferTarget,
        size: usize,
        usage: BufferUsage,
    ) -> Result<()> {
        // SAFETY: see module docs.
        unsafe {
            self.ctx
                .buffer_data_size(target as u32, size as i32, usage as u32);
        };
        self.get_error()
    }

    pub fn buffer_data(&self, target: BufferTarget, data: &[u8], usage: BufferUsage) -> Result<()> {
        // SAFETY: slice pointer/len are valid.
        unsafe {
            self.ctx
                .buffer_data_u8_slice(target as u32, data, usage as u32);
        };
        self.get_error()
    }

    pub fn buffer_sub_data(&self, target: BufferTarget, offset: usize, data: &[u8]) -> Result<()> {
        // SAFETY: slice pointer/len are valid.
        unsafe {
            self.ctx
                .buffer_sub_data_u8_slice(target as u32, offset as i32, data);
        }
        self.get_error()
    }

    // ----------------------------------------------------------- programs

    /// Create a program from vertex and fragment shader source: compile both,
    /// attach and link (`Program.createVF`).
    pub fn create_program_vf(&self, vsrc: &str, fsrc: &str) -> Result<RawProgram> {
        let gl = &*self.ctx;
        let compile = |ty: u32, src: &str| -> Result<glow::Shader> {
            // SAFETY: see module docs.
            unsafe {
                let s = gl.create_shader(ty).map_err(Error::CreateFailed)?;
                gl.shader_source(s, src);
                gl.compile_shader(s);
                if gl.get_shader_compile_status(s) {
                    return Ok(s);
                }
                let log = gl.get_shader_info_log(s);
                log::error!("shader compilation failure id={s:?} message={log}");
                gl.delete_shader(s);
                Err(Error::CompileFailed(log))
            }
        };
        let vs = compile(glow::VERTEX_SHADER, vsrc)?;
        let fs = match compile(glow::FRAGMENT_SHADER, fsrc) {
            Ok(fs) => fs,
            Err(e) => {
                // SAFETY: see module docs.
                unsafe { gl.delete_shader(vs) };
                return Err(e);
            }
        };

        // SAFETY: see module docs.
        unsafe {
            let result = (|| {
                let p = gl.create_program().map_err(Error::CreateFailed)?;
                gl.attach_shader(p, vs);
                gl.attach_shader(p, fs);
                gl.link_program(p);
                if gl.get_program_link_status(p) {
                    return Ok(p);
                }
                let log = gl.get_program_info_log(p);
                log::error!("program link failure id={p:?} message={log}");
                gl.delete_program(p);
                Err(Error::CompileFailed(log))
            })();
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            result
        }
    }

    pub fn use_program(&self, p: Option<RawProgram>) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.use_program(p) };
        self.get_error()
    }

    pub fn delete_program(&self, p: RawProgram) {
        // SAFETY: see module docs.
        unsafe { self.ctx.delete_program(p) };
    }

    // ------------------------------------------------------ vertex arrays

    pub fn create_vertex_array(&self) -> Result<RawVertexArray> {
        // SAFETY: see module docs.
        unsafe { self.ctx.create_vertex_array() }.map_err(Error::CreateFailed)
    }

    pub fn delete_vertex_array(&self, v: RawVertexArray) {
        // SAFETY: see module docs.
        unsafe { self.ctx.delete_vertex_array(v) };
    }

    pub fn bind_vertex_array(&self, v: Option<RawVertexArray>) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.bind_vertex_array(v) };
        self.get_error()
    }

    /// Configure attribute `idx` on the currently bound VAO (`VertexArray.Binding`
    /// enableAttribArray + attributeBinding + bindingDivisor + attribute*Format).
    pub fn setup_vertex_attrib(
        &self,
        idx: u32,
        binding: u32,
        divisor: u32,
        size: i32,
        ty: AttribType,
        offset: u32,
    ) -> Result<()> {
        let gl = &*self.ctx;
        // SAFETY: see module docs.
        unsafe {
            gl.enable_vertex_attrib_array(idx);
            self.get_error()?;
            gl.vertex_attrib_binding(idx, binding);
            self.get_error()?;
            gl.vertex_binding_divisor(binding, divisor);
            self.get_error()?;
            match ty {
                AttribType::U8 => {
                    gl.vertex_attrib_format_i32(idx, size, glow::UNSIGNED_BYTE, offset);
                }
                AttribType::U16 => {
                    gl.vertex_attrib_format_i32(idx, size, glow::UNSIGNED_SHORT, offset);
                }
                AttribType::U32 => {
                    gl.vertex_attrib_format_i32(idx, size, glow::UNSIGNED_INT, offset);
                }
                AttribType::I8 => gl.vertex_attrib_format_i32(idx, size, glow::BYTE, offset),
                AttribType::I16 => gl.vertex_attrib_format_i32(idx, size, glow::SHORT, offset),
                AttribType::I32 => gl.vertex_attrib_format_i32(idx, size, glow::INT, offset),
                AttribType::F16 => {
                    gl.vertex_attrib_format_f32(idx, size, glow::HALF_FLOAT, false, offset);
                }
                AttribType::F32 => {
                    gl.vertex_attrib_format_f32(idx, size, glow::FLOAT, false, offset);
                }
                AttribType::F64 => gl.vertex_attrib_format_f64(idx, size, glow::DOUBLE, offset),
            }
        }
        self.get_error()
    }

    /// `glBindVertexBuffer` on the currently bound VAO.
    pub fn bind_vertex_buffer(
        &self,
        idx: u32,
        b: RawBuffer,
        offset: i32,
        stride: i32,
    ) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.bind_vertex_buffer(idx, Some(b), offset, stride) };
        self.get_error()
    }

    // ------------------------------------------------------- framebuffers

    pub fn create_framebuffer(&self) -> Result<RawFramebuffer> {
        // SAFETY: see module docs.
        unsafe { self.ctx.create_framebuffer() }.map_err(Error::CreateFailed)
    }

    pub fn delete_framebuffer(&self, f: RawFramebuffer) {
        // SAFETY: see module docs.
        unsafe { self.ctx.delete_framebuffer(f) };
    }

    /// Bind a framebuffer, remembering what was bound before (the default
    /// framebuffer isn't always zero, so it's queried at runtime).
    pub fn bind_framebuffer(
        &self,
        f: RawFramebuffer,
        target: FramebufferTarget,
    ) -> FramebufferBinding {
        // SAFETY: see module docs.
        unsafe {
            let current = self.ctx.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
            self.ctx.bind_framebuffer(target as u32, Some(f));
            FramebufferBinding {
                target,
                previous: u32::try_from(current)
                    .ok()
                    .and_then(std::num::NonZeroU32::new)
                    .map(glow::NativeFramebuffer),
            }
        }
    }

    /// Restore the framebuffer binding that was current before `bind_framebuffer`.
    pub fn unbind_framebuffer(&self, b: FramebufferBinding) {
        // SAFETY: see module docs.
        unsafe { self.ctx.bind_framebuffer(b.target as u32, b.previous) };
    }

    pub fn framebuffer_texture_2d(
        &self,
        b: &FramebufferBinding,
        attachment: Attachment,
        textarget: TextureTarget,
        texture: RawTexture,
        level: i32,
    ) -> Result<()> {
        // SAFETY: see module docs.
        unsafe {
            self.ctx.framebuffer_texture_2d(
                b.target as u32,
                attachment as u32,
                textarget as u32,
                Some(texture),
                level,
            );
        }
        self.get_error()
    }

    pub fn framebuffer_renderbuffer(
        &self,
        b: &FramebufferBinding,
        attachment: Attachment,
        rb: RawRenderbuffer,
    ) -> Result<()> {
        // SAFETY: see module docs.
        unsafe {
            self.ctx.framebuffer_renderbuffer(
                b.target as u32,
                attachment as u32,
                glow::RENDERBUFFER,
                Some(rb),
            );
        }
        self.get_error()
    }

    /// Returns true if the currently bound framebuffer (at `target`) is complete.
    pub fn framebuffer_complete(&self, target: FramebufferTarget) -> bool {
        // SAFETY: see module docs.
        unsafe { self.ctx.check_framebuffer_status(target as u32) == glow::FRAMEBUFFER_COMPLETE }
    }

    pub fn create_renderbuffer(&self) -> Result<RawRenderbuffer> {
        // SAFETY: see module docs.
        unsafe { self.ctx.create_renderbuffer() }.map_err(Error::CreateFailed)
    }

    pub fn delete_renderbuffer(&self, r: RawRenderbuffer) {
        // SAFETY: see module docs.
        unsafe { self.ctx.delete_renderbuffer(r) };
    }

    pub fn bind_renderbuffer(&self, r: Option<RawRenderbuffer>) {
        // SAFETY: see module docs.
        unsafe { self.ctx.bind_renderbuffer(glow::RENDERBUFFER, r) };
    }

    /// `glRenderbufferStorage` on the currently bound renderbuffer.
    pub fn renderbuffer_storage(&self, format: InternalFormat, w: i32, h: i32) -> Result<()> {
        // SAFETY: see module docs.
        unsafe {
            self.ctx
                .renderbuffer_storage(glow::RENDERBUFFER, format as u32, w, h);
        };
        self.get_error()
    }

    // ----------------------------------------------------------- textures

    pub fn create_texture(&self) -> Result<RawTexture> {
        // SAFETY: see module docs.
        unsafe { self.ctx.create_texture() }.map_err(Error::CreateFailed)
    }

    pub fn delete_texture(&self, t: RawTexture) {
        // SAFETY: see module docs.
        unsafe { self.ctx.delete_texture(t) };
    }

    pub fn bind_texture(&self, target: TextureTarget, t: Option<RawTexture>) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.bind_texture(target as u32, t) };
        self.get_error()
    }

    /// `glActiveTexture(GL_TEXTURE0 + index)`.
    pub fn active_texture(&self, index: u32) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.active_texture(glow::TEXTURE0 + index) };
        self.get_error()
    }

    pub fn tex_parameter_i32(&self, target: TextureTarget, name: u32, value: i32) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.tex_parameter_i32(target as u32, name, value) };
        self.get_error()
    }

    #[allow(clippy::too_many_arguments)]
    /// `glTexImage2D` (level, border 0, `GL_UNSIGNED_BYTE`) on the bound texture.
    pub fn tex_image_2d(
        &self,
        target: TextureTarget,
        level: i32,
        internal: InternalFormat,
        width: i32,
        height: i32,
        format: PixelFormat,
        data: Option<&[u8]>,
    ) -> Result<()> {
        // SAFETY: the slice (if any) is a valid byte slice; as in Ghostty the
        // caller is responsible for it being at least width*height*bpp bytes.
        unsafe {
            self.ctx.tex_image_2d(
                target as u32,
                level,
                internal as i32,
                width,
                height,
                0,
                format as u32,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(data),
            );
        }
        self.get_error()
    }

    #[allow(clippy::too_many_arguments)]
    /// `glTexSubImage2D` (`GL_UNSIGNED_BYTE`) on the bound texture.
    pub fn tex_sub_image_2d(
        &self,
        target: TextureTarget,
        level: i32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: PixelFormat,
        data: &[u8],
    ) -> Result<()> {
        // SAFETY: see `tex_image_2d`.
        unsafe {
            self.ctx.tex_sub_image_2d(
                target as u32,
                level,
                x,
                y,
                width,
                height,
                format as u32,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(data)),
            );
        }
        self.get_error()
    }

    // ----------------------------------------------------------- samplers

    pub fn create_sampler(&self) -> Result<RawSampler> {
        // SAFETY: see module docs.
        unsafe { self.ctx.create_sampler() }.map_err(Error::CreateFailed)
    }

    pub fn delete_sampler(&self, s: RawSampler) {
        // SAFETY: see module docs.
        unsafe { self.ctx.delete_sampler(s) };
    }

    pub fn sampler_parameter_i32(&self, s: RawSampler, name: u32, value: i32) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.sampler_parameter_i32(s, name, value) };
        self.get_error()
    }

    pub fn bind_sampler(&self, unit: u32, s: RawSampler) -> Result<()> {
        // SAFETY: see module docs.
        unsafe { self.ctx.bind_sampler(unit, Some(s)) };
        self.get_error()
    }
}

/// Parameter names used by the textures/samplers (`gl.Texture.Parameter`).
pub mod param {
    pub const MIN_FILTER: u32 = glow::TEXTURE_MIN_FILTER;
    pub const MAG_FILTER: u32 = glow::TEXTURE_MAG_FILTER;
    pub const WRAP_S: u32 = glow::TEXTURE_WRAP_S;
    pub const WRAP_T: u32 = glow::TEXTURE_WRAP_T;
}

/// The GL debug message callback (`glDebugMessageCallback` in OpenGL.zig).
fn debug_message(src: u32, typ: u32, id: u32, severity: u32, msg: &str) {
    let src_str = match src {
        glow::DEBUG_SOURCE_API => "OpenGL API",
        glow::DEBUG_SOURCE_WINDOW_SYSTEM => "Window System",
        glow::DEBUG_SOURCE_SHADER_COMPILER => "Shader Compiler",
        glow::DEBUG_SOURCE_THIRD_PARTY => "Third Party",
        glow::DEBUG_SOURCE_APPLICATION => "User",
        glow::DEBUG_SOURCE_OTHER => "Other",
        _ => "Unknown",
    };

    let typ_str = match typ {
        glow::DEBUG_TYPE_ERROR => "Error",
        glow::DEBUG_TYPE_DEPRECATED_BEHAVIOR => "Deprecated Behavior",
        glow::DEBUG_TYPE_UNDEFINED_BEHAVIOR => "Undefined Behavior",
        glow::DEBUG_TYPE_PORTABILITY => "Portability Issue",
        glow::DEBUG_TYPE_PERFORMANCE => "Performance Issue",
        glow::DEBUG_TYPE_MARKER => "Marker",
        glow::DEBUG_TYPE_PUSH_GROUP => "Group Push",
        glow::DEBUG_TYPE_POP_GROUP => "Group Pop",
        glow::DEBUG_TYPE_OTHER => "Other",
        _ => "Unknown",
    };

    match severity {
        glow::DEBUG_SEVERITY_HIGH => log::error!("[{id}] ({src_str}: {typ_str}) {msg}"),
        glow::DEBUG_SEVERITY_MEDIUM => log::warn!("[{id}] ({src_str}: {typ_str}) {msg}"),
        glow::DEBUG_SEVERITY_LOW => log::info!("[{id}] ({src_str}: {typ_str}) {msg}"),
        glow::DEBUG_SEVERITY_NOTIFICATION => {
            log::debug!("[{id}] ({src_str}: {typ_str}) {msg}");
        }
        _ => log::warn!("UNKNOWN SEVERITY [{id}] ({src_str}: {typ_str}) {msg}"),
    }
}
