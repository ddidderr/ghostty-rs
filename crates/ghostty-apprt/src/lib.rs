//! ghostty-apprt: the contract between the core and app runtimes
//! (ZIG/src/apprt/{action,structs,surface,ipc,runtime,none}.zig).
//!
//! The C-ABI parts of the Zig sources (`C` structs, `cval`, `CValue`,
//! `ghostty.h` sync tests) belong to the embedded runtime and are not ported.

pub mod action;
pub mod ipc;
pub mod runtime;
pub mod runtime_api;
pub mod structs;
pub mod surface;

pub use action::{Action, Target};
pub use structs::{
    Clipboard,
    ClipboardContent,
    ClipboardRequest,
    ClipboardRequestType,
    ColorScheme,
    ContentScale,
    CursorPos,
    ImePos,
    Selection,
    SurfaceSize,
};
