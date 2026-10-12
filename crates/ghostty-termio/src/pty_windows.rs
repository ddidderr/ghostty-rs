//! Windows PTY creation and management (`WindowsPty` in `pty.zig`): a ConPTY
//! pseudo console (`CreatePseudoConsole`) connected to a pair of pipes.
//!
//! The Zig struct has `in_pipe`/`out_pipe` (our ends) and
//! `in_pipe_pty`/`out_pipe_pty` (the ends handed to the pseudo console). All
//! handles are `OwnedHandle`s here and are closed on drop, in the same order
//! as `WindowsPty.deinit` (pipes first, the pseudo console last).

// Win32 FFI takes out-parameters as `*mut`; implicit `&mut` -> pointer coercions
// are the idiomatic spelling at these call sites.
#![allow(clippy::borrow_as_ptr)]

use std::{
    io,
    os::windows::io::{AsHandle, BorrowedHandle, FromRawHandle, OwnedHandle, RawHandle},
    ptr,
    sync::atomic::{AtomicU32, Ordering},
};

use windows_sys::Win32::{
    Foundation::{
        FALSE,
        GENERIC_READ,
        HANDLE,
        HANDLE_FLAG_INHERIT,
        INVALID_HANDLE_VALUE,
        S_OK,
        SetHandleInformation,
    },
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{
        CreateFileW,
        FILE_ATTRIBUTE_NORMAL,
        FILE_FLAG_FIRST_PIPE_INSTANCE,
        FILE_FLAG_OVERLAPPED,
        OPEN_EXISTING,
        PIPE_ACCESS_OUTBOUND,
    },
    System::{
        Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole},
        Pipes::{CreateNamedPipeW, CreatePipe, PIPE_TYPE_BYTE},
        Threading::GetCurrentProcessId,
    },
};

/// Redeclare the winsize struct so we can just use a Rust struct. The
/// defaults on this are some reasonable screen size but you should probably
/// not use them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Winsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

impl Default for Winsize {
    fn default() -> Self {
        Self {
            ws_row: 100,
            ws_col: 80,
            ws_xpixel: 800,
            ws_ypixel: 600,
        }
    }
}

/// The modes of a pty. Not all of these modes are supported on
/// all platforms but all platforms share the same mode struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    /// ICANON on POSIX
    pub canonical: bool,

    /// ECHO on POSIX
    pub echo: bool,
}

impl Default for Mode {
    fn default() -> Self {
        Self {
            canonical: true,
            echo: true,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// A Win32 call failed (`windows.unexpectedError`); carries the
    /// `GetLastError` code, or the failing `HRESULT` for ConPTY calls.
    #[error("unexpected windows error {0:#x}")]
    Unexpected(u32),
    #[error("resize failed")]
    ResizeFailed,
}

/// Process-wide counter for pipe names.
static PIPE_NAME_COUNTER: AtomicU32 = AtomicU32::new(1);

/// The pseudo console handle; closed (after the pipes) on drop.
#[derive(Debug)]
struct PseudoConsole(HPCON);

impl Drop for PseudoConsole {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful CreatePseudoConsole and
        // is closed exactly once, here.
        unsafe { ClosePseudoConsole(self.0) };
    }
}

/// A ConPTY pseudo console with its pipes.
///
/// Field order is the drop order: pipes first, the pseudo console last.
#[derive(Debug)]
#[allow(clippy::struct_field_names)]
pub struct Pty {
    // The pseudo console's ends of the pipes are only held to keep them open.
    #[allow(dead_code)]
    in_pipe_pty: OwnedHandle,
    /// Our write end (named pipe, overlapped) for input to the child.
    in_pipe: OwnedHandle,
    #[allow(dead_code)]
    out_pipe_pty: OwnedHandle,
    /// Our read end for output from the child.
    out_pipe: OwnedHandle,
    pseudo_console: PseudoConsole,
    size: Winsize,
}

fn last_error() -> Error {
    Error::Unexpected(io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32)
}

/// The unique named pipe path for the input pipe, as UTF-16 with a NUL.
fn pipe_path(pid: u32, counter: u32) -> Vec<u16> {
    format!("\\\\.\\pipe\\LOCAL\\ghostty-pty-{pid}-{counter}\0")
        .encode_utf16()
        .collect()
}

fn coord(size: Winsize) -> COORD {
    COORD {
        X: size.ws_col.min(i16::MAX as u16) as i16,
        Y: size.ws_row.min(i16::MAX as u16) as i16,
    }
}

impl Pty {
    /// Open a new PTY with the given initial size.
    #[allow(unsafe_code)]
    pub fn open(size: Winsize) -> Result<Pty, Error> {
        let pipe_path_w = pipe_path(
            // SAFETY: no preconditions.
            unsafe { GetCurrentProcessId() },
            PIPE_NAME_COUNTER.fetch_add(1, Ordering::Relaxed),
        );

        let security_attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            bInheritHandle: FALSE,
            lpSecurityDescriptor: ptr::null_mut(),
        };

        // The in_pipe needs to be created as a named pipe, since anonymous
        // pipes created with CreatePipe do not support overlapped operations
        // (which Zig's libxev IOCP backend needs; we keep the same shape).
        //
        // SAFETY: the path is NUL terminated and the attributes outlive the
        // call.
        let in_pipe = unsafe {
            CreateNamedPipeW(
                pipe_path_w.as_ptr(),
                PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE,
                1,
                4096,
                4096,
                0,
                &security_attributes,
            )
        };
        if in_pipe == INVALID_HANDLE_VALUE {
            return Err(last_error());
        }
        // SAFETY: valid handle we own.
        let in_pipe = unsafe { OwnedHandle::from_raw_handle(in_pipe as RawHandle) };

        // SAFETY: as above.
        let in_pipe_pty = unsafe {
            CreateFileW(
                pipe_path_w.as_ptr(),
                GENERIC_READ,
                0,
                &security_attributes,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if in_pipe_pty == INVALID_HANDLE_VALUE {
            return Err(last_error());
        }
        // SAFETY: valid handle we own.
        let in_pipe_pty = unsafe { OwnedHandle::from_raw_handle(in_pipe_pty as RawHandle) };

        let mut out_pipe: HANDLE = ptr::null_mut();
        let mut out_pipe_pty: HANDLE = ptr::null_mut();
        // SAFETY: both out pointers are valid for writes.
        if unsafe { CreatePipe(&mut out_pipe, &mut out_pipe_pty, ptr::null(), 0) } == 0 {
            return Err(last_error());
        }
        // SAFETY: CreatePipe succeeded: two valid handles we own.
        let (out_pipe, out_pipe_pty) = unsafe {
            (
                OwnedHandle::from_raw_handle(out_pipe as RawHandle),
                OwnedHandle::from_raw_handle(out_pipe_pty as RawHandle),
            )
        };

        for h in [&in_pipe, &in_pipe_pty, &out_pipe, &out_pipe_pty] {
            set_not_inheritable(h.as_handle())?;
        }

        let mut hpc: HPCON = 0;
        // SAFETY: the pipe handles are valid; `hpc` is a valid out pointer.
        let result = unsafe {
            CreatePseudoConsole(
                coord(size),
                raw(&in_pipe_pty),
                raw(&out_pipe_pty),
                0,
                &mut hpc,
            )
        };
        if result != S_OK {
            return Err(Error::Unexpected(result as u32));
        }

        Ok(Pty {
            in_pipe_pty,
            in_pipe,
            out_pipe_pty,
            out_pipe,
            pseudo_console: PseudoConsole(hpc),
            size,
        })
    }

    /// Return the size of the pty.
    pub fn get_size(&self) -> Result<Winsize, Error> {
        Ok(self.size)
    }

    /// Set the size of the pty.
    #[allow(unsafe_code)]
    pub fn set_size(&mut self, size: Winsize) -> Result<(), Error> {
        // SAFETY: the pseudo console handle is valid until drop.
        let result = unsafe { ResizePseudoConsole(self.pseudo_console.0, coord(size)) };
        if result != S_OK {
            return Err(Error::ResizeFailed);
        }
        self.size = size;
        Ok(())
    }

    /// The pseudo console to attach a child process to (`Command.pseudo_console`).
    pub fn pseudo_console(&self) -> HPCON {
        self.pseudo_console.0
    }

    /// Our read end: output of the child.
    pub fn out_pipe(&self) -> BorrowedHandle<'_> {
        self.out_pipe.as_handle()
    }

    /// Our write end: input to the child (opened for overlapped I/O).
    pub fn in_pipe(&self) -> BorrowedHandle<'_> {
        self.in_pipe.as_handle()
    }
}

fn raw(h: &OwnedHandle) -> HANDLE {
    std::os::windows::io::AsRawHandle::as_raw_handle(h) as HANDLE
}

#[allow(unsafe_code)]
fn set_not_inheritable(h: BorrowedHandle<'_>) -> Result<(), Error> {
    // SAFETY: the handle is valid for the duration of the call.
    if unsafe {
        SetHandleInformation(
            std::os::windows::io::AsRawHandle::as_raw_handle(&h) as HANDLE,
            HANDLE_FLAG_INHERIT,
            0,
        )
    } == 0
    {
        return Err(last_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pty_size() {
        let mut ws = Winsize {
            ws_row: 50,
            ws_col: 80,
            ws_xpixel: 1,
            ws_ypixel: 1,
        };

        let mut pty = Pty::open(ws).unwrap();

        // Initialize size should match what we gave it
        assert_eq!(ws, pty.get_size().unwrap());

        // Can set and read new sizes
        ws.ws_row *= 2;
        pty.set_size(ws).unwrap();
        assert_eq!(ws, pty.get_size().unwrap());
    }

    #[test]
    fn pipe_names_are_unique_and_nul_terminated() {
        let a = pipe_path(7, 1);
        let b = pipe_path(7, 2);
        assert_ne!(a, b);
        assert_eq!(a.last(), Some(&0));
        assert!(String::from_utf16_lossy(&a).starts_with("\\\\.\\pipe\\LOCAL\\ghostty-pty-7-1"));
    }

    #[test]
    fn coord_clamps() {
        let c = coord(Winsize {
            ws_row: 50,
            ws_col: u16::MAX,
            ws_xpixel: 0,
            ws_ypixel: 0,
        });
        assert_eq!((c.X, c.Y), (i16::MAX, 50));
    }
}
