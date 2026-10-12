//! Wrapper for handling render frames (`metal/Frame.zig`).
use std::{ptr::NonNull, sync::Arc};

use block2::RcBlock;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandQueue};

use super::{IOSurfaceLayer, Metal, Queue, render_pass::RenderPass, target::Target};
use crate::{
    backend::Health,
    graphics_api::{Attachment, FrameCompletion, GpuFrame},
};

/// A frame being encoded: one MTLCommandBuffer.
pub struct Frame {
    /// MTLCommandBuffer (`None` if the queue failed to create one, in which
    /// case the frame is a no-op that completes unhealthy).
    buffer: Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>>,

    /// The layer the target is presented on.
    layer: IOSurfaceLayer,

    /// The target is presented via the layer when completed.
    target: Arc<Target>,
}

impl Frame {
    /// Begin encoding a frame.
    pub fn begin(queue: &Queue, layer: IOSurfaceLayer, target: Arc<Target>) -> Frame {
        let buffer = queue.commandBuffer();
        if buffer.is_none() {
            log::warn!("unable to create a Metal command buffer");
        }
        Frame {
            buffer,
            layer,
            target,
        }
    }

    /// Add a render pass to this frame with the provided attachments.
    /// Returns a RenderPass which allows render steps to be added.
    pub fn render_pass(&self, attachments: &[Attachment<'_, Metal>]) -> RenderPass {
        RenderPass::begin(self.buffer.as_deref(), attachments)
    }

    /// Complete this frame and present the target.
    ///
    /// If `sync` is true, this will block until the frame is presented.
    pub fn complete(self, sync: bool, completion: FrameCompletion) {
        let Frame {
            buffer,
            layer,
            target,
        } = self;
        let Some(buffer) = buffer else {
            completion(Health::Unhealthy);
            return;
        };

        // What happens when the command buffer has finished (`bufferCompleted`).
        let finish = move |buffer: &ProtocolObject<dyn MTLCommandBuffer>, sync: bool| {
            // Get our command buffer status to pass back to the generic renderer.
            let health = health_for_status(buffer.status());

            // If the frame is healthy, present it.
            if health == Health::Healthy {
                if sync {
                    layer.set_surface_sync(target.surface());
                } else {
                    layer.set_surface(target.surface());
                }
            }

            completion(health);
        };

        if sync {
            // If we need to complete synchronously, we wait until the buffer
            // is completed and invoke the callback directly.
            buffer.commit();
            buffer.waitUntilCompleted();
            finish(&buffer, true);
        } else {
            // Otherwise we add our block as a completion handler. It is
            // copied when we add the handler, and then the copy is deallocated
            // by the objc runtime on success.
            let block = RcBlock::new(
                move |buffer: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                    // SAFETY: Metal passes the (live) command buffer.
                    finish(unsafe { buffer.as_ref() }, false);
                },
            );
            // SAFETY: the block is a valid completion handler and is copied
            // by `addCompletedHandler`.
            unsafe { buffer.addCompletedHandler(RcBlock::as_ptr(&block)) };
            buffer.commit();
        }
    }
}

/// The health of a frame from its command buffer status.
pub fn health_for_status(status: MTLCommandBufferStatus) -> Health {
    if status == MTLCommandBufferStatus::Error {
        Health::Unhealthy
    } else {
        Health::Healthy
    }
}

impl GpuFrame<Metal> for Frame {
    fn render_pass<'a>(&'a self, attachments: &'a [Attachment<'a, Metal>]) -> RenderPass
    where
        Metal: 'a,
    {
        Frame::render_pass(self, attachments)
    }

    fn complete(self, sync: bool, completion: FrameCompletion) {
        Frame::complete(self, sync, completion);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_status_is_unhealthy() {
        assert_eq!(
            health_for_status(MTLCommandBufferStatus::Error),
            Health::Unhealthy
        );
        assert_eq!(
            health_for_status(MTLCommandBufferStatus::Completed),
            Health::Healthy
        );
        assert_eq!(
            health_for_status(MTLCommandBufferStatus::NotEnqueued),
            Health::Healthy
        );
    }
}
