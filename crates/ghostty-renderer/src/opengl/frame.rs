//! Wrapper for handling render frames (`opengl/Frame.zig`).
use std::sync::Arc;

use super::{
    OpenGL,
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
/// after presenting; here the frame borrows the graphics API and
/// [`Frame::complete`] returns the [`Health`] for the renderer to report.
pub struct Frame<'a> {
    api: &'a mut OpenGL,
    target: Arc<Target>,
}

impl<'a> Frame<'a> {
    /// Begin encoding a frame.
    ///
    /// The target is presented via the provided API when completed.
    pub fn begin(_opts: Options, api: &'a mut OpenGL, target: Arc<Target>) -> Frame<'a> {
        Frame { api, target }
    }

    /// Add a render pass to this frame with the provided attachments.
    /// Returns a RenderPass which allows render steps to be added.
    pub fn render_pass<'b>(&self, attachments: &'b [Attachment<'b>]) -> RenderPass<'b> {
        let gl = self.api.gl().expect("OpenGL context not initialized");
        RenderPass::begin(gl, RenderPassOptions { attachments })
    }

    /// Complete this frame and present the target.
    ///
    /// If `sync` is true, this will block until the frame is presented.
    ///
    /// NOTE: For OpenGL, `sync` is ignored and we always block.
    pub fn complete(self, _sync: bool) -> Health {
        let Some(gl) = self.api.gl().cloned() else {
            return Health::Unhealthy;
        };
        gl.finish();

        // If there are any GL errors, consider the frame unhealthy.
        let health = if gl.get_error().is_ok() {
            Health::Healthy
        } else {
            Health::Unhealthy
        };

        // If the frame is healthy, present it.
        if health == Health::Healthy
            && let Err(err) = self.api.present(&self.target)
        {
            log::error!("Failed to present render target: err={err}");
        }

        // Report the health to the renderer.
        health
    }
}
