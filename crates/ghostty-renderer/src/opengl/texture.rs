//! Wrapper for handling textures (`opengl/Texture.zig`).
use super::gl::{
    self, Gl, InternalFormat, MagFilter, MinFilter, PixelFormat, RawTexture, TextureTarget, Wrap,
};

/// Options for initializing a texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub format: PixelFormat,
    pub internal_format: InternalFormat,
    pub target: TextureTarget,
    pub min_filter: MinFilter,
    pub mag_filter: MagFilter,
    pub wrap_s: Wrap,
    pub wrap_t: Wrap,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An OpenGL API call failed.
    #[error("OpenGL call failed")]
    OpenGLFailed,
}

pub struct Texture {
    gl: Gl,
    handle: RawTexture,

    /// The width of this texture.
    pub width: usize,
    /// The height of this texture.
    pub height: usize,

    /// Format for this texture.
    pub format: PixelFormat,

    /// Target for this texture.
    pub target: TextureTarget,
}

impl Texture {
    /// The raw GL texture handle.
    pub fn raw(&self) -> RawTexture {
        self.handle
    }

    /// Initialize a texture
    pub fn init(
        gl: &Gl,
        opts: Options,
        width: usize,
        height: usize,
        data: Option<&[u8]>,
    ) -> Result<Texture, Error> {
        let texture = gl.create_texture().map_err(|_| Error::OpenGLFailed)?;
        // On any error below, `this` is dropped and the texture destroyed.
        let this = Texture {
            gl: gl.clone(),
            handle: texture,
            width,
            height,
            format: opts.format,
            target: opts.target,
        };

        let r = (|| -> gl::Result<()> {
            gl.bind_texture(opts.target, Some(texture))?;
            gl.tex_parameter_i32(opts.target, gl::param::WRAP_S, opts.wrap_s as i32)?;
            gl.tex_parameter_i32(opts.target, gl::param::WRAP_T, opts.wrap_t as i32)?;
            gl.tex_parameter_i32(opts.target, gl::param::MIN_FILTER, opts.min_filter as i32)?;
            gl.tex_parameter_i32(opts.target, gl::param::MAG_FILTER, opts.mag_filter as i32)?;
            gl.tex_image_2d(
                opts.target,
                0,
                opts.internal_format,
                width as i32,
                height as i32,
                opts.format,
                data,
            )
        })();
        let _ = gl.bind_texture(opts.target, None);
        r.map_err(|_| Error::OpenGLFailed)?;
        Ok(this)
    }

    /// Replace a region of the texture with the provided data.
    ///
    /// Does NOT check the dimensions of the data to ensure correctness.
    pub fn replace_region(
        &self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        data: &[u8],
    ) -> Result<(), Error> {
        self.gl
            .bind_texture(self.target, Some(self.handle))
            .map_err(|_| Error::OpenGLFailed)?;
        let r = self.gl.tex_sub_image_2d(
            self.target,
            0,
            x as i32,
            y as i32,
            width as i32,
            height as i32,
            self.format,
            data,
        );
        let _ = self.gl.bind_texture(self.target, None);
        r.map_err(|_| Error::OpenGLFailed)
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        self.gl.delete_texture(self.handle);
    }
}
