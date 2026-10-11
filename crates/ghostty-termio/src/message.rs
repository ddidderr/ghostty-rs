//! The messages that can be sent to an IO thread (`termio/message.zig`).

use ghostty_datastruct::MessageData;
use ghostty_renderer::size::Size;
use ghostty_terminal::terminal::ScrollViewport;

use crate::termio::DerivedConfig;

/// Represents a write request. Magic number comes from the largest
/// other union value. It can be upped if we add a larger union member
/// in the future.
pub type WriteReq = MessageData<u8, 38>;

/// The types of size reports that we support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeReport {
    Mode2048,
    Csi14T,
    Csi16T,
    Csi18T,
}

/// The messages that can be sent to an IO thread.
///
/// Zig has three write variants (`write_small`, `write_stable`,
/// `write_alloc`); in Rust they are the three variants of the single
/// [`WriteReq`] (`MessageData`) enum carried by [`Message::Write`].
#[derive(Debug, Clone)]
pub enum Message {
    /// Request a color scheme report is sent to the pty.
    ColorSchemeReport {
        /// Force write the current color scheme
        force: bool,
    },

    /// Purposely crash the renderer. This is used for testing and debugging.
    /// See the "crash" binding action.
    Crash,

    /// The derived configuration to update the implementation with.
    ChangeConfig(Box<DerivedConfig>),

    /// Activate or deactivate the inspector.
    Inspector(bool),

    /// Resize the window.
    Resize(Size),

    /// Request a size report is sent to the pty ([in-band
    /// size report, mode 2048](https://gist.github.com/rockorager/e695fb2924d36b2bcf1fff4a3704bd83) and
    /// [XTWINOPS](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h4-Functions-using-CSI-_-ordered-by-the-final-character-lparen-s-rparen:CSI-Ps;Ps;Ps-t.1EB0)).
    SizeReport(SizeReport),

    /// Clear the screen.
    ClearScreen {
        /// Include clearing the history
        history: bool,
    },

    /// Scroll the viewport
    ScrollViewport(ScrollViewport),

    /// Selection scrolling. If this is set to true then the termio
    /// thread starts a timer that will trigger a `SelectionScrollTick`
    /// message back to the surface. This ping/pong is because the
    /// surface thread doesn't have access to an event loop from libghostty.
    SelectionScroll(bool),

    /// Jump forward/backward n prompts.
    JumpToPrompt(isize),

    /// Send this when a synchronized output mode is started. This will
    /// start the timer so that the output mode is disabled after a
    /// period of time so that a bad actor can't hang the terminal.
    StartSynchronizedOutput,

    /// Enable or disable linefeed mode (mode 20).
    LinefeedMode(bool),

    /// The surface gained or lost focus.
    Focused(bool),

    /// Write data to the pty (`write_small` / `write_stable` / `write_alloc`).
    Write(WriteReq),
}

impl Message {
    /// Return a write request for the given data. This will use the small
    /// variant if it fits or allocate otherwise. This should NOT be used for
    /// stable (`'static`) data, use [`Message::write_stable`] for that.
    pub fn write_req(data: &[u8]) -> Message {
        Message::Write(WriteReq::init(data))
    }

    /// A write request for data with a stable pointer (`write_stable`).
    pub fn write_stable(data: &'static [u8]) -> Message {
        Message::Write(WriteReq::Stable(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Zig asserts `@sizeOf(Message) == 40`. The Rust layout differs (a tag
    // plus the 38-byte inline buffer, a Vec, and boxed config), so we only
    // guard against accidental growth.
    #[test]
    fn message_size_does_not_grow() {
        assert!(
            std::mem::size_of::<Message>() <= 72,
            "Message grew to {} bytes",
            std::mem::size_of::<Message>()
        );
    }

    #[test]
    fn write_req_small_vs_alloc() {
        let Message::Write(w) = Message::write_req(b"hi") else {
            panic!()
        };
        assert!(matches!(w, WriteReq::Small { .. }));
        assert_eq!(w.slice(), b"hi");

        let big = vec![b'x'; 100];
        let Message::Write(w) = Message::write_req(&big) else {
            panic!()
        };
        assert!(matches!(w, WriteReq::Alloc(_)));
        assert_eq!(w.slice(), &big[..]);

        let Message::Write(w) = Message::write_stable(b"abc") else {
            panic!()
        };
        assert!(matches!(w, WriteReq::Stable(_)));
    }
}
