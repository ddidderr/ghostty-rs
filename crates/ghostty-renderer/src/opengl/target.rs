//! Represents a render target (`opengl/Target.zig`).
//!
//! In this case, an OpenGL renderbuffer-backed framebuffer.
use super::gl::{
    self,
    Attachment,
    FramebufferTarget,
    Gl,
    InternalFormat,
    RawFramebuffer,
    RawRenderbuffer,
};

/// Options for initializing a Target
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Desired width
    pub width: usize,
    /// Desired height
    pub height: usize,

    /// Internal format for the renderbuffer.
    pub internal_format: InternalFormat,
}

pub struct Target {
    gl: Gl,

    /// The underlying framebuffer.
    framebuffer: RawFramebuffer,

    /// The underlying renderbuffer.
    renderbuffer: RawRenderbuffer,

    /// Current width of this target.
    pub width: usize,
    /// Current height of this target.
    pub height: usize,
}

impl Target {
    pub fn framebuffer(&self) -> RawFramebuffer {
        self.framebuffer
    }

    pub fn renderbuffer(&self) -> RawRenderbuffer {
        self.renderbuffer
    }

    pub fn init(gl: &Gl, opts: Options) -> gl::Result<Target> {
        let rbo = gl.create_renderbuffer()?;
        gl.bind_renderbuffer(Some(rbo));
        let r =
            gl.renderbuffer_storage(opts.internal_format, opts.width as i32, opts.height as i32);
        gl.bind_renderbuffer(None);
        if let Err(e) = r {
            gl.delete_renderbuffer(rbo);
            return Err(e);
        }

        let fbo = match gl.create_framebuffer() {
            Ok(f) => f,
            Err(e) => {
                gl.delete_renderbuffer(rbo);
                return Err(e);
            }
        };
        // Dropping `this` on error cleans up both objects.
        let this = Target {
            gl: gl.clone(),
            framebuffer: fbo,
            renderbuffer: rbo,
            width: opts.width,
            height: opts.height,
        };
        let bound = gl.bind_framebuffer(fbo, FramebufferTarget::Framebuffer);
        let r = gl.framebuffer_renderbuffer(&bound, Attachment::Color0, rbo);
        gl.unbind_framebuffer(bound);
        r?;
        Ok(this)
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        self.gl.delete_framebuffer(self.framebuffer);
        self.gl.delete_renderbuffer(self.renderbuffer);
    }
}
