//! The outward-facing mailboxes termio needs from the rest of the program.
//!
//! In Zig these are concrete types (`apprt.surface.Mailbox`,
//! `renderer.Thread.Mailbox`). The renderer thread is not ported yet and the
//! surface mailbox is generic over the core's surface handle and app
//! mailbox, so termio only depends on these two small traits.

use ghostty_apprt::surface::{AppMailbox, Mailbox as ApprtMailbox, Message as SurfaceMessage};
use ghostty_datastruct::blocking_queue::Timeout;
use ghostty_renderer::message::Message as RendererMessage;

/// The mailbox for sending messages to the surface
/// (`apprt.surface.Mailbox`).
///
/// `push` returns the queue size after the push; zero means the push failed
/// (a full queue, an expired timeout, or shutdown cancellation).
pub trait SurfaceMailbox: Send + Sync {
    fn push(&self, msg: SurfaceMessage, timeout: Timeout) -> usize;
}

impl<S, A> SurfaceMailbox for ApprtMailbox<S, A>
where
    S: Clone + Send + Sync,
    A: AppMailbox<S> + Send + Sync,
{
    fn push(&self, msg: SurfaceMessage, timeout: Timeout) -> usize {
        ApprtMailbox::push(self, msg, timeout)
    }
}

/// The mailbox for sending messages to the renderer thread
/// (`renderer.Thread.Mailbox`). The renderer thread also has a wakeup handle
/// (see [`crate::Options::renderer_wakeup`]) which termio signals to hint
/// that a redraw should happen.
pub trait RendererMailbox: Send + Sync {
    fn push(&self, msg: RendererMessage, timeout: Timeout) -> usize;
}
