//! Port of `surface.zig`: messages sent from the core to a surface's apprt.

use std::sync::Arc;

use ghostty_config::{config::Config, types_a::WorkingDirectory};
use ghostty_datastruct::{MessageData, blocking_queue::Timeout};
use ghostty_vt::{
    mouse_shape::MouseShape,
    osc::{ProgressReport, color::ColoredTarget},
};

use crate::structs::Clipboard;

/// Represents a write request. Magic number comes from the max size
/// we want this union to be.
pub type WriteReq = MessageData<u8, 255>;

/// Maximum title size (including NUL sentinel space) of `set_title`.
pub const SET_TITLE_LEN: usize = 256;
/// `[63:0]u8` => 64 bytes including the sentinel.
pub const NOTIFICATION_TITLE_LEN: usize = 64;
/// `[255:0]u8` => 256 bytes including the sentinel.
pub const NOTIFICATION_BODY_LEN: usize = 256;

/// The health status of a renderer (`renderer.Health`).
pub use ghostty_renderer::backend::Health as RendererHealth;
/// Scrollbar geometry in rows (`terminal.Scrollbar`).
pub use ghostty_terminal::page_list::Scrollbar;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportTitleStyle {
    Csi21T,
    // This enum is a placeholder for future title styles.
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildExited {
    pub exit_code: u32,
    pub runtime_ms: u64,
}

/// Fixed-size desktop notification (title `[63:0]u8`, body `[255:0]u8`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopNotification {
    /// NUL-terminated.
    pub title: [u8; NOTIFICATION_TITLE_LEN],
    /// NUL-terminated.
    pub body: [u8; NOTIFICATION_BODY_LEN],
}

fn cstr(buf: &[u8]) -> &[u8] {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    &buf[..end]
}

fn fill<const N: usize>(src: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    let n = src.len().min(N - 1);
    out[..n].copy_from_slice(&src[..n]);
    out
}

impl DesktopNotification {
    /// Build from byte strings, truncating to fit (keeping the sentinel).
    #[must_use]
    pub fn new(title: &[u8], body: &[u8]) -> Self {
        Self {
            title: fill(title),
            body: fill(body),
        }
    }

    #[must_use]
    pub fn title(&self) -> &[u8] {
        cstr(&self.title)
    }

    #[must_use]
    pub fn body(&self) -> &[u8] {
        cstr(&self.body)
    }
}

/// The message types that can be sent to a single surface.
#[derive(Debug, Clone)]
pub enum Message {
    /// Set the title of the surface (NUL-terminated within 256 bytes).
    /// TODO(upstream): should become a `WriteReq`.
    SetTitle([u8; SET_TITLE_LEN]),

    /// Report the window title back to the terminal
    ReportTitle(ReportTitleStyle),

    /// Set the mouse shape.
    SetMouseShape(MouseShape),

    /// Read the clipboard and write to the pty.
    ClipboardRead(Clipboard),

    /// Write the clipboard contents.
    ClipboardWrite {
        clipboard_type: Clipboard,
        req: WriteReq,
    },

    /// Change the configuration to the given configuration. In Zig the
    /// pointer is not valid after receiving; here it is shared via `Arc`.
    ChangeConfig(Arc<Config>),

    /// Close the surface. This will only close the current surface that
    /// receives this, not the full application.
    Close,

    /// The child process running in the surface has exited. This may trigger
    /// a surface close, it may not. Additional details about the child
    /// command are given in the `ChildExited` struct.
    ChildExited(ChildExited),

    /// Show a desktop notification.
    DesktopNotification(DesktopNotification),

    /// Health status change for the renderer.
    RendererHealth(RendererHealth),

    /// Tell the surface to present itself to the user. This may require raising
    /// a window and switching tabs.
    PresentSurface,

    /// Notifies the surface that password input has started within
    /// the terminal. This should always be followed by a false value
    /// unless the surface exits.
    PasswordInput(bool),

    /// A terminal color was changed using OSC sequences.
    ColorChange(ColoredTarget),

    /// Notifies the surface that a tick of the timer that is timing
    /// out selection scrolling has occurred. "selection scrolling"
    /// is when the user has clicked and dragged the mouse outside
    /// the viewport of the terminal and the terminal is scrolling
    /// the viewport to follow the mouse cursor.
    SelectionScrollTick(bool),

    /// The terminal has reported a change in the working directory.
    PwdChange(WriteReq),

    /// The terminal encountered a bell character.
    RingBell,

    /// Report the progress of an action using a GUI element
    ProgressReport(ProgressReport),

    /// A command has started in the shell, start a timer.
    StartCommand,

    /// A command has finished in the shell, stop the timer and send out
    /// notifications as appropriate. The optional u8 is the exit code
    /// of the command.
    StopCommand(Option<u8>),

    /// The scrollbar state changed for the surface.
    Scrollbar(Scrollbar),

    /// Search progress update
    SearchTotal(Option<usize>),

    /// Selected search index change
    SearchSelected(Option<usize>),
}

/// The app-level mailbox a surface mailbox forwards into.
///
/// TODO(integration): in Zig this is `App.Mailbox` taking an
/// `App.Message::surface_message { surface, message }`. ghostty-core
/// implements this trait for its app mailbox (the `S` handle being the core
/// surface pointer/id).
pub trait AppMailbox<S> {
    /// Push a surface-directed message to the app thread. Returns the queue
    /// size after the push; zero means the push failed.
    fn push_surface_message(&self, surface: S, message: Message, timeout: Timeout) -> usize;
}

/// A surface mailbox.
#[derive(Debug, Clone)]
pub struct Mailbox<S, A> {
    pub surface: S,
    pub app: A,
}

impl<S: Clone, A: AppMailbox<S>> Mailbox<S, A> {
    /// Send a message to the surface. Surface message sending is actually
    /// implemented on the app thread, so we rewrap the message with our
    /// surface handle and send it to the app thread.
    pub fn push(&self, msg: Message, timeout: Timeout) -> usize {
        self.app
            .push_surface_message(self.surface.clone(), msg, timeout)
    }
}

/// Context for new surface creation to determine inheritance behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum NewSurfaceContext {
    Window = 0,
    Tab = 1,
    Split = 2,
}

#[must_use]
pub fn should_inherit_working_directory(context: NewSurfaceContext, config: &Config) -> bool {
    match context {
        NewSurfaceContext::Window => config.window_inherit_working_directory,
        NewSurfaceContext::Tab => config.tab_inherit_working_directory,
        NewSurfaceContext::Split => config.split_inherit_working_directory,
    }
}

/// Returns a new config for a surface that should be used for any new
/// surfaces.
///
/// `prev_pwd` is the pwd of the previously focused surface (Zig:
/// `app.focusedSurface()?.pwd()`), which the caller (core) must look up.
// TODO(integration): core could wrap this to resolve the focused surface.
#[must_use]
pub fn new_config(config: &Config, context: NewSurfaceContext, prev_pwd: Option<String>) -> Config {
    let mut copy = config.clone();
    if let Some(pwd) = prev_pwd
        && should_inherit_working_directory(context, config)
    {
        copy.working_directory = Some(WorkingDirectory::Path(pwd));
    }
    copy
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct Rec(RefCell<Vec<(u32, bool)>>);
    impl AppMailbox<u32> for Rec {
        fn push_surface_message(&self, s: u32, m: Message, _t: Timeout) -> usize {
            self.0.borrow_mut().push((s, matches!(m, Message::Close)));
            self.0.borrow().len()
        }
    }

    #[test]
    fn mailbox_wraps_surface() {
        let mb = Mailbox {
            surface: 7u32,
            app: Rec(RefCell::new(vec![])),
        };
        assert_eq!(mb.push(Message::Close, Timeout::Instant), 1);
        assert_eq!(mb.app.0.borrow()[0], (7, true));
    }

    #[test]
    fn notification_truncates() {
        let n = DesktopNotification::new(&[b'a'; 100], b"hi");
        assert_eq!(n.title().len(), 63);
        assert_eq!(n.body(), b"hi");
    }

    #[test]
    fn inherit_and_new_config() {
        let c = Config {
            tab_inherit_working_directory: false,
            ..Config::default()
        };
        assert!(should_inherit_working_directory(
            NewSurfaceContext::Window,
            &c
        ));
        assert!(!should_inherit_working_directory(
            NewSurfaceContext::Tab,
            &c
        ));
        let n = new_config(&c, NewSurfaceContext::Window, Some("/tmp".into()));
        assert_eq!(
            n.working_directory,
            Some(WorkingDirectory::Path("/tmp".into()))
        );
        let n = new_config(&c, NewSurfaceContext::Tab, Some("/tmp".into()));
        assert_eq!(n.working_directory, None);
    }
}
