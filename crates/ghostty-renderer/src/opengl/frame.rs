//! Wrapper for handling render frames (`opengl/Frame.zig`).
use std::sync::Arc;

use super::{
    LastTarget,
    OpenGL,
    gl::Gl,
    render_pass::{Attachment, Options as RenderPassOptions, RenderPass},
    target::Target,
};
use crate::backend::Health;

/// Options for beginning a frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {}

/// A frame being encoded.
///
/// Upstream holds `*Renderer` and calls `renderer.frameCompleted(health)`
/// after presenting; here [`Frame::complete`] returns the [`Health`] for the
/// renderer to report. The frame holds its own handles (GL context and the
/// API's last-presented-target slot) rather than borrowing the API.
pub struct Frame {
    gl: Gl,
    last_target: LastTarget,
    target: Arc<Target>,
}

impl Frame {
    /// Begin encoding a frame.
    ///
    /// The target is presented via the provided API when completed.
    pub fn begin(_opts: Options, api: &OpenGL, target: Arc<Target>) -> Frame {
        Frame {
            gl: api.gl().expect("OpenGL context not initialized").clone(),
            last_target: api.last_target_slot(),
            target,
        }
    }

    /// Add a render pass to this frame with the provided attachments.
    /// Returns a RenderPass which allows render steps to be added.
    pub fn render_pass<'b>(&self, attachments: &'b [Attachment<'b>]) -> RenderPass<'b> {
        RenderPass::begin(&self.gl, RenderPassOptions { attachments })
    }

    /// Complete this frame and present the target.
    ///
    /// If `sync` is true, this will block until the frame is presented.
    ///
    /// NOTE: For OpenGL, `sync` is ignored and we always block.
    pub fn complete(self, _sync: bool) -> Health {
        self.gl.finish();

        // If there are any GL errors, consider the frame unhealthy.
        let health = if self.gl.get_error().is_ok() {
            Health::Healthy
        } else {
            Health::Unhealthy
        };

        // If the frame is healthy, present it.
        if health == Health::Healthy
            && let Err(err) = super::present_target(&self.gl, &self.last_target, &self.target)
        {
            log::error!("Failed to present render target: err={err}");
        }

        // Report the health to the renderer.
        health
    }
}
