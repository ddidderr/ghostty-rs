//! Command launches sub-processes (Windows branch of `Command.zig`:
//! `startWindows` and the Windows `wait`). Uses `CreateProcessW`, attaching
//! the child to a ConPTY pseudo console through a process thread attribute
//! list when one is set.
//!
//! The pure parts (command line quoting, environment block) are in
//! [`crate::windows_args`].

// Win32 FFI takes out-parameters as `*mut`; implicit `&mut` -> pointer coercions
// are the idiomatic spelling at these call sites.
#![allow(clippy::borrow_as_ptr)]

use std::{
    io,
    os::windows::io::{AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle, RawHandle},
    ptr,
};

use ghostty_os::windows::{
    CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    to_wide_nul,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, TRUE, WAIT_FAILED},
    System::{
        Console::HPCON,
        Threading::{
            CreateProcessW,
            DeleteProcThreadAttributeList,
            GetExitCodeProcess,
            INFINITE,
            InitializeProcThreadAttributeList,
            LPPROC_THREAD_ATTRIBUTE_LIST,
            PROCESS_INFORMATION,
            STARTF_USESTDHANDLES,
            STARTUPINFOEXW,
            TerminateProcess,
            UpdateProcThreadAttribute,
            WaitForSingleObject,
        },
    },
};

use crate::windows_args::{self, create_windows_env_block, windows_create_command_line};

/// Errors from [`Command::start`].
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("string or environment is not valid unicode")]
    Encode(#[from] windows_args::EncodeError),
    #[error("windows error: {0}")]
    Os(#[from] io::Error),
}

/// How a process exited. On Windows this is just the exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Exited(u32),
}

/// A command to launch.
#[derive(Default)]
pub struct Command {
    /// Path to the command to run (`lpApplicationName`).
    pub path: String,

    /// Command-line arguments. It is the responsibility of the caller to set
    /// args[0] to the command. If empty, no command line is passed.
    pub args: Vec<String>,

    /// Environment variables for the child process. If this is `None`,
    /// inherits the environment variables from this process. These are the
    /// exact environment variables to set; these are /not/ merged.
    pub env: Option<ghostty_os::EnvMap>,

    /// Working directory for the child. If not set, the working directory of
    /// the calling process is preserved.
    pub cwd: Option<String>,

    /// The file handles to set for stdin/out/err. If not set the NUL device
    /// is used. Ignored if `pseudo_console` is set.
    pub stdin: Option<OwnedHandle>,
    pub stdout: Option<OwnedHandle>,
    pub stderr: Option<OwnedHandle>,

    /// If set, then the process will be created attached to this pseudo
    /// console. `stdin`, `stdout`, and `stderr` will be ignored if set.
    pub pseudo_console: Option<HPCON>,

    /// Process handle (Zig stores it in `pid`), set after start is called.
    process: Option<OwnedHandle>,

    /// Process ID, set after start is called.
    pub pid: Option<u32>,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Command")
            .field("path", &self.path)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl Command {
    pub fn new(path: impl Into<String>, args: &[&str]) -> Self {
        Self {
            path: path.into(),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            ..Self::default()
        }
    }

    /// The process handle, which becomes signaled when the process exits.
    pub fn process_handle(&self) -> Option<BorrowedHandle<'_>> {
        use std::os::windows::io::AsHandle;
        self.process.as_ref().map(AsHandle::as_handle)
    }

    /// Start the subprocess. This returns immediately once the child is
    /// started. After this is successful, `pid` and the process handle are
    /// available.
    #[allow(unsafe_code)]
    pub fn start(&mut self) -> Result<(), StartError> {
        let application_w = to_wide_nul(&self.path);
        let cwd_w = self.cwd.as_deref().map(to_wide_nul);
        let mut command_line_w = if self.args.is_empty() {
            None
        } else {
            Some(to_wide_nul(&windows_create_command_line(&self.args)))
        };
        let mut env_w = match &self.env {
            Some(map) => Some(create_windows_env_block(map.iter())?),
            None => None,
        };

        // If any stdio handle is missing (and we're not using a pseudo
        // console) the child gets the NUL device for it.
        let any_null_fd = self.stdin.is_none() || self.stdout.is_none() || self.stderr.is_none();
        let null_fd = if any_null_fd && self.pseudo_console.is_none() {
            Some(
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open("NUL")?,
            )
        } else {
            None
        };

        // TODO (as in Zig): in the case of having handles instead of a pty,
        // set up attributes such that the child process only inherits these
        // handles, then set bInheritsHandles below.

        // The attribute list buffer must outlive CreateProcessW. A u64 buffer
        // guarantees pointer alignment.
        let mut attribute_buf: Vec<u64> = Vec::new();
        let attribute_list: LPPROC_THREAD_ATTRIBUTE_LIST =
            if let Some(pseudo_console) = self.pseudo_console {
                let mut size: usize = 0;
                // SAFETY: querying the required size with a null list is the
                // documented use; it is expected to "fail" with the size set.
                unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size) };
                attribute_buf.resize(size.div_ceil(8), 0);
                let list: LPPROC_THREAD_ATTRIBUTE_LIST = attribute_buf.as_mut_ptr().cast();
                // SAFETY: the buffer is at least `size` bytes and 8-aligned.
                if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
                    return Err(io::Error::last_os_error().into());
                }
                // SAFETY: the list is initialized; the pseudo console handle is
                // passed by value as the attribute value, as the API requires.
                if unsafe {
                    UpdateProcThreadAttribute(
                        list,
                        0,
                        PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                        pseudo_console as *const std::ffi::c_void,
                        std::mem::size_of::<HPCON>(),
                        ptr::null_mut(),
                        ptr::null(),
                    )
                } == 0
                {
                    let err = io::Error::last_os_error();
                    // SAFETY: initialized above.
                    unsafe { DeleteProcThreadAttributeList(list) };
                    return Err(err.into());
                }
                list
            } else {
                ptr::null_mut()
            };

        let handle_of = |h: &Option<OwnedHandle>| -> HANDLE {
            match (h, &null_fd) {
                (Some(h), _) => h.as_raw_handle() as HANDLE,
                (None, Some(f)) => f.as_raw_handle() as HANDLE,
                (None, None) => ptr::null_mut(),
            }
        };
        let (stdin, stdout, stderr) = if attribute_list.is_null() {
            (
                handle_of(&self.stdin),
                handle_of(&self.stdout),
                handle_of(&self.stderr),
            )
        } else {
            (ptr::null_mut(), ptr::null_mut(), ptr::null_mut())
        };

        // SAFETY: STARTUPINFOEXW is plain data; all-zero is a valid value.
        let mut startup_info_ex: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        startup_info_ex.StartupInfo.cb = if attribute_list.is_null() {
            std::mem::size_of::<windows_sys::Win32::System::Threading::STARTUPINFOW>() as u32
        } else {
            std::mem::size_of::<STARTUPINFOEXW>() as u32
        };
        startup_info_ex.StartupInfo.hStdError = stderr;
        startup_info_ex.StartupInfo.hStdOutput = stdout;
        startup_info_ex.StartupInfo.hStdInput = stdin;
        startup_info_ex.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup_info_ex.lpAttributeList = attribute_list;

        let mut flags = CREATE_UNICODE_ENVIRONMENT;
        if !attribute_list.is_null() {
            flags |= EXTENDED_STARTUPINFO_PRESENT;
        }

        // SAFETY: PROCESS_INFORMATION is plain data.
        let mut process_information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: every pointer is either null or points at a NUL-terminated
        // (or properly terminated environment) buffer that outlives the call;
        // the command line buffer is mutable as CreateProcessW requires.
        let ok = unsafe {
            CreateProcessW(
                application_w.as_ptr(),
                command_line_w
                    .as_mut()
                    .map_or(ptr::null_mut(), Vec::as_mut_ptr),
                ptr::null(),
                ptr::null(),
                TRUE,
                flags,
                env_w
                    .as_mut()
                    .map_or(ptr::null(), |w| w.as_mut_ptr().cast_const().cast()),
                cwd_w.as_ref().map_or(ptr::null(), Vec::as_ptr),
                ptr::addr_of!(startup_info_ex.StartupInfo),
                &mut process_information,
            )
        };
        let err = (ok == 0).then(io::Error::last_os_error);
        if !attribute_list.is_null() {
            // SAFETY: initialized above; no longer needed after the call.
            unsafe { DeleteProcThreadAttributeList(attribute_list) };
        }
        if let Some(err) = err {
            return Err(err.into());
        }

        // SAFETY: CreateProcessW succeeded, so both handles are valid and
        // owned by us. Zig leaks the thread handle; we close it.
        let (process, _thread) = unsafe {
            (
                OwnedHandle::from_raw_handle(process_information.hProcess as RawHandle),
                OwnedHandle::from_raw_handle(process_information.hThread as RawHandle),
            )
        };
        self.process = Some(process);
        self.pid = Some(process_information.dwProcessId);
        Ok(())
    }

    /// Wait for the command to exit and return information about how it
    /// exited. Like Zig this always blocks until the process exits (`block`
    /// is ignored on Windows); it returns immediately if the process already
    /// exited.
    #[allow(unsafe_code)]
    pub fn wait(&self, _block: bool) -> io::Result<Exit> {
        let handle = self
            .process
            .as_ref()
            .ok_or_else(|| io::Error::other("process not started"))?
            .as_raw_handle() as HANDLE;

        // SAFETY: the handle is valid while `self` is alive.
        if unsafe { WaitForSingleObject(handle, INFINITE) } == WAIT_FAILED {
            return Err(io::Error::last_os_error());
        }
        self.exit_code().map(Exit::Exited)
    }

    /// The exit code of the (already exited) process (`GetExitCodeProcess`).
    #[allow(unsafe_code)]
    pub fn exit_code(&self) -> io::Result<u32> {
        let handle = self
            .process
            .as_ref()
            .ok_or_else(|| io::Error::other("process not started"))?
            .as_raw_handle() as HANDLE;
        let mut code: u32 = 0;
        // SAFETY: valid handle and out pointer.
        if unsafe { GetExitCodeProcess(handle, &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(code)
    }

    /// Terminate the process (`TerminateProcess(pid, 0)`).
    #[allow(unsafe_code)]
    pub fn terminate(&self) -> io::Result<()> {
        let Some(process) = &self.process else {
            return Ok(());
        };
        // SAFETY: valid handle.
        if unsafe { TerminateProcess(process.as_raw_handle() as HANDLE, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_cmd_and_wait() {
        let mut cmd = Command::new(
            "C:\\Windows\\System32\\cmd.exe",
            &["cmd.exe", "/c", "exit 7"],
        );
        cmd.start().unwrap();
        assert!(cmd.pid.is_some());
        assert_eq!(cmd.wait(true).unwrap(), Exit::Exited(7));
    }
}
