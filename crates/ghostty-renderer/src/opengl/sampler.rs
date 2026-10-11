//! Wrapper for handling samplers (`opengl/Sampler.zig`).
use super::gl::{self, Gl, MagFilter, MinFilter, RawSampler, Wrap};

/// Options for initializing a sampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
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

pub struct Sampler {
    gl: Gl,
    sampler: RawSampler,
}

impl Sampler {
    /// The raw GL sampler handle.
    pub fn raw(&self) -> RawSampler {
        self.sampler
    }

    /// Initialize a sampler
    pub fn init(gl: &Gl, opts: Options) -> Result<Sampler, Error> {
        let sampler = gl.create_sampler().map_err(|_| Error::OpenGLFailed)?;
        // Dropped (and destroyed) on error.
        let this = Sampler {
            gl: gl.clone(),
            sampler,
        };
        let p = |name, v| gl.sampler_parameter_i32(sampler, name, v);
        (|| -> gl::Result<()> {
            p(gl::param::WRAP_S, opts.wrap_s as i32)?;
            p(gl::param::WRAP_T, opts.wrap_t as i32)?;
            p(gl::param::MIN_FILTER, opts.min_filter as i32)?;
            p(gl::param::MAG_FILTER, opts.mag_filter as i32)
        })()
        .map_err(|_| Error::OpenGLFailed)?;
        Ok(this)
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.gl.delete_sampler(self.sampler);
    }
}
