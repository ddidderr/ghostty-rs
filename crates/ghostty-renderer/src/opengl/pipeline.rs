//! Wrapper for handling render pipelines (`opengl/Pipeline.zig`).
use super::gl::{
    self, AttribType, FramebufferTarget, Gl, RawFramebuffer, RawProgram, RawVertexArray,
};

/// Vertex step function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepFunction {
    Constant,
    #[default]
    PerVertex,
    PerInstance,
}

/// Options for initializing a render pipeline.
#[derive(Debug, Clone)]
pub struct Options<'a> {
    /// GLSL source of the vertex function
    pub vertex_fn: &'a str,
    /// GLSL source of the fragment function
    pub fragment_fn: &'a str,

    /// Vertex step function
    pub step_fn: StepFunction,

    /// Whether to enable blending.
    pub blending_enabled: bool,
}

impl<'a> Options<'a> {
    pub fn new(vertex_fn: &'a str, fragment_fn: &'a str) -> Self {
        Options {
            vertex_fn,
            fragment_fn,
            step_fn: StepFunction::PerVertex,
            blending_enabled: true,
        }
    }
}

/// One vertex attribute of a vertex struct (what Zig derives by comptime
/// reflection over the struct fields in `autoAttribute`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attribute {
    /// Number of components (1 for scalars, N for `[T; N]`).
    pub size: i32,
    /// Component type.
    pub ty: AttribType,
    /// Byte offset within the vertex struct.
    pub offset: u32,
}

/// A vertex struct whose fields map 1:1 to vertex attribute locations
/// 0, 1, 2, ... in declaration order.
pub trait Vertex: bytemuck::Pod {
    const ATTRIBUTES: &'static [Attribute];
}

pub struct Pipeline {
    gl: Gl,

    pub program: RawProgram,

    pub fbo: RawFramebuffer,

    pub vao: RawVertexArray,

    pub stride: usize,

    pub blending_enabled: bool,
}

impl Pipeline {
    /// Build a pipeline with vertex attributes derived from `V`
    /// (`Pipeline.init(VertexAttributes, opts)`).
    pub fn init<V: Vertex>(gl: &Gl, opts: &Options<'_>) -> gl::Result<Pipeline> {
        Self::init_impl(gl, Some((V::ATTRIBUTES, size_of::<V>())), opts)
    }

    /// Build a pipeline without any vertex attributes
    /// (`Pipeline.init(null, opts)`).
    pub fn init_no_attributes(gl: &Gl, opts: &Options<'_>) -> gl::Result<Pipeline> {
        Self::init_impl(gl, None, opts)
    }

    fn init_impl(
        gl: &Gl,
        attrs: Option<(&[Attribute], usize)>,
        opts: &Options<'_>,
    ) -> gl::Result<Pipeline> {
        // Load and compile our shaders.
        let program = gl.create_program_vf(opts.vertex_fn, opts.fragment_fn)?;

        let fbo = match gl.create_framebuffer() {
            Ok(f) => f,
            Err(e) => {
                gl.delete_program(program);
                return Err(e);
            }
        };
        let vao = match gl.create_vertex_array() {
            Ok(v) => v,
            Err(e) => {
                gl.delete_framebuffer(fbo);
                gl.delete_program(program);
                return Err(e);
            }
        };

        // From here on, dropping `this` releases everything on error.
        let this = Pipeline {
            gl: gl.clone(),
            program,
            fbo,
            vao,
            stride: attrs.map_or(0, |(_, stride)| stride),
            blending_enabled: opts.blending_enabled,
        };

        let r = (|| -> gl::Result<()> {
            gl.use_program(Some(program))?;
            let fbobind = gl.bind_framebuffer(fbo, FramebufferTarget::Framebuffer);
            let r = (|| {
                gl.bind_vertex_array(Some(vao))?;
                let r = match attrs {
                    Some((attrs, _)) => auto_attribute(gl, attrs, opts.step_fn),
                    None => Ok(()),
                };
                let _ = gl.bind_vertex_array(None);
                r
            })();
            gl.unbind_framebuffer(fbobind);
            let _ = gl.use_program(None);
            r
        })();
        r?;
        Ok(this)
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.gl.delete_program(self.program);
        self.gl.delete_framebuffer(self.fbo);
        self.gl.delete_vertex_array(self.vao);
    }
}

fn auto_attribute(gl: &Gl, attrs: &[Attribute], step_fn: StepFunction) -> gl::Result<()> {
    let divisor: u32 = match step_fn {
        StepFunction::PerVertex => 0,
        StepFunction::PerInstance => 1,
        StepFunction::Constant => u32::MAX,
    };

    for (i, a) in attrs.iter().enumerate() {
        gl.setup_vertex_attrib(i as u32, 0, divisor, a.size, a.ty, a.offset)?;
    }
    Ok(())
}
