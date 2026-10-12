//! Platform independent pieces of the native exception handlers (macOS Mach
//! exception ports, Windows unhandled-exception filter): decoding of the
//! raw exception messages and mapping of exception codes to the names and
//! descriptions recorded in the event. Unit-tested on every platform.

// Only used by the macOS/Windows handlers; the tests run everywhere.
#![cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]

use std::fmt::Write as _;

/// `mach_exception_raise` message id (`EXCEPTION_DEFAULT | MACH_EXCEPTION_CODES`).
pub(crate) const MACH_EXCEPTION_RAISE: i32 = 2405;

/// `EXC_*` exception types.
pub(crate) const EXC_BAD_ACCESS: u32 = 1;
pub(crate) const EXC_BAD_INSTRUCTION: u32 = 2;
pub(crate) const EXC_ARITHMETIC: u32 = 3;
pub(crate) const EXC_SOFTWARE: u32 = 5;
pub(crate) const EXC_BREAKPOINT: u32 = 6;

/// `EXC_SOFT_SIGNAL`: the code of an `EXC_SOFTWARE` exception that stands for
/// a BSD signal (the subcode is the signal number). This is how an
/// `abort()` (SIGABRT) is described, as Apple's crash reporter does.
pub(crate) const EXC_SOFT_SIGNAL: i64 = 0x10003;

/// Message id of the "write a dump for this signal" request the signal
/// handler sends to the exception handler thread (not a Mach exception id).
pub(crate) const ABORT_MSG_ID: i32 = 0x6768_6f73;

impl MachException {
    /// The exception describing a fatal BSD signal that never went through
    /// the Mach exception path (`abort()` raises SIGABRT directly).
    pub(crate) fn from_signal(signo: i32) -> MachException {
        MachException {
            kind: EXC_SOFTWARE,
            code: EXC_SOFT_SIGNAL,
            subcode: i64::from(signo),
        }
    }
}

/// Wire layout of the abort request after the 24 byte Mach header:
/// `thread: u32`, `signo: i32` (native endian).
pub(crate) const ABORT_BODY_LEN: usize = 8;

/// Encode the body of an abort request.
pub(crate) fn abort_body(thread: u32, signo: i32) -> [u8; ABORT_BODY_LEN] {
    let mut b = [0u8; ABORT_BODY_LEN];
    b[..4].copy_from_slice(&thread.to_ne_bytes());
    b[4..].copy_from_slice(&signo.to_ne_bytes());
    b
}

/// Decode an abort request message (header included): the faulting thread
/// and the signal. `None` for any other message.
pub(crate) fn parse_abort(msg: &[u8]) -> Option<(u32, i32)> {
    if u32_at(msg, 20)? as i32 != ABORT_MSG_ID {
        return None;
    }
    Some((u32_at(msg, HEADER)?, u32_at(msg, HEADER + 4)? as i32))
}

/// A decoded Mach exception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MachException {
    pub kind: u32,
    pub code: i64,
    pub subcode: i64,
}

/// `kern_return_t` values reported as the code of `EXC_BAD_ACCESS`.
fn kern_name(code: i64) -> Option<&'static str> {
    Some(match code {
        1 => "KERN_INVALID_ADDRESS",
        2 => "KERN_PROTECTION_FAILURE",
        _ => return None,
    })
}

impl MachException {
    /// `EXC_BAD_ACCESS`, ...
    pub(crate) fn name(&self) -> &'static str {
        match self.kind {
            EXC_BAD_ACCESS => "EXC_BAD_ACCESS",
            EXC_BAD_INSTRUCTION => "EXC_BAD_INSTRUCTION",
            EXC_ARITHMETIC => "EXC_ARITHMETIC",
            4 => "EXC_EMULATION",
            5 => "EXC_SOFTWARE",
            EXC_BREAKPOINT => "EXC_BREAKPOINT",
            _ => "EXC_UNKNOWN",
        }
    }

    /// The BSD signal the kernel would turn this exception into
    /// (`ux_exception`), recorded as `mechanism.meta.signal`.
    pub(crate) fn signal(&self) -> (i32, i32) {
        // (signo, si_code) following xnu's `ux_exception`.
        match self.kind {
            EXC_BAD_ACCESS => match self.code {
                1 => (11, 1), // SIGSEGV, SEGV_MAPERR
                2 => (11, 2), // SIGSEGV, SEGV_ACCERR
                _ => (10, 0), // SIGBUS
            },
            EXC_BAD_INSTRUCTION => (4, 1), // SIGILL, ILL_ILLOPC
            EXC_ARITHMETIC => (8, 0),      // SIGFPE
            EXC_BREAKPOINT => (5, 1),      // SIGTRAP, TRAP_BRKPT
            EXC_SOFTWARE if self.code == EXC_SOFT_SIGNAL => (self.subcode as i32, 0),
            _ => (0, 0),
        }
    }

    /// The faulting address for `EXC_BAD_ACCESS`.
    pub(crate) fn fault_addr(&self) -> u64 {
        if self.kind == EXC_BAD_ACCESS {
            self.subcode as u64
        } else {
            0
        }
    }

    /// Human readable description.
    pub(crate) fn describe(&self, out: &mut String) {
        let _ = write!(out, "{}", self.name());
        if let Some(k) = (self.kind == EXC_BAD_ACCESS)
            .then(|| kern_name(self.code))
            .flatten()
        {
            let _ = write!(out, " ({k}) at 0x{:x}", self.subcode as u64);
        } else {
            let _ = write!(
                out,
                " (code=0x{:x}, subcode=0x{:x})",
                self.code, self.subcode
            );
        }
    }
}

/// The wire layout of a `mach_exception_raise` request (`exc.defs` with
/// `MACH_EXCEPTION_CODES`), 4 byte packed like all Mach messages:
///
/// ```text
/// mach_msg_header_t   24 bytes
/// mach_msg_body_t      4  (descriptor count = 2)
/// thread descriptor   12
/// task descriptor     12
/// NDR_record_t         8
/// exception_type_t     4
/// codeCnt              4
/// int64 code[codeCnt]
/// ```
pub(crate) struct RaiseRequest {
    pub thread: u32,
    pub task: u32,
    pub exception: MachException,
}

const HEADER: usize = 24;
const BODY: usize = 4;
const DESC: usize = 12;
const OFF_THREAD: usize = HEADER + BODY;
const OFF_TASK: usize = OFF_THREAD + DESC;
const OFF_NDR: usize = OFF_TASK + DESC;
const OFF_EXC: usize = OFF_NDR + 8;
const OFF_CNT: usize = OFF_EXC + 4;
const OFF_CODES: usize = OFF_CNT + 4;

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

fn i64_at(b: &[u8], off: usize) -> Option<i64> {
    Some(i64::from_ne_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

/// Decode a received `mach_exception_raise` message. `None` for any other
/// message id or a truncated/inconsistent message.
pub(crate) fn parse_raise(msg: &[u8]) -> Option<RaiseRequest> {
    let id = u32_at(msg, 20)? as i32;
    if id != MACH_EXCEPTION_RAISE {
        return None;
    }
    // Complex message (bit 31 of msgh_bits) carrying two port descriptors.
    if u32_at(msg, 0)? & 0x8000_0000 == 0 || u32_at(msg, HEADER)? != 2 {
        return None;
    }
    let count = u32_at(msg, OFF_CNT)? as usize;
    if count > 2 {
        return None;
    }
    let code = if count >= 1 {
        i64_at(msg, OFF_CODES)?
    } else {
        0
    };
    let subcode = if count >= 2 {
        i64_at(msg, OFF_CODES + 8)?
    } else {
        0
    };
    Some(RaiseRequest {
        thread: u32_at(msg, OFF_THREAD)?,
        task: u32_at(msg, OFF_TASK)?,
        exception: MachException {
            kind: u32_at(msg, OFF_EXC)?,
            code,
            subcode,
        },
    })
}

/// Name and description of a Windows `NTSTATUS` exception code.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn windows_exception(code: u32) -> (&'static str, &'static str) {
    match code {
        0xC000_0005 => ("EXCEPTION_ACCESS_VIOLATION", "Access violation"),
        0x8000_0002 => ("EXCEPTION_DATATYPE_MISALIGNMENT", "Datatype misalignment"),
        0x8000_0003 => ("EXCEPTION_BREAKPOINT", "Breakpoint"),
        0xC000_0006 => ("EXCEPTION_IN_PAGE_ERROR", "In-page error"),
        0xC000_001D => ("EXCEPTION_ILLEGAL_INSTRUCTION", "Illegal instruction"),
        0xC000_0025 => (
            "EXCEPTION_NONCONTINUABLE_EXCEPTION",
            "Noncontinuable exception",
        ),
        0xC000_0094 => ("EXCEPTION_INT_DIVIDE_BY_ZERO", "Integer divide by zero"),
        0xC000_0095 => ("EXCEPTION_INT_OVERFLOW", "Integer overflow"),
        0xC000_008E => ("EXCEPTION_FLT_DIVIDE_BY_ZERO", "Float divide by zero"),
        0xC000_0096 => ("EXCEPTION_PRIV_INSTRUCTION", "Privileged instruction"),
        0xC000_00FD => ("EXCEPTION_STACK_OVERFLOW", "Stack overflow"),
        0xC000_0409 => ("STATUS_STACK_BUFFER_OVERRUN", "Fast fail"),
        _ => ("EXCEPTION", "Unhandled exception"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn raise(kind: u32, codes: &[i64]) -> Vec<u8> {
        let mut m = Vec::new();
        m.extend_from_slice(&0x8000_0011u32.to_ne_bytes()); // complex | COPY_SEND
        m.extend_from_slice(&0u32.to_ne_bytes()); // size (unused)
        m.extend_from_slice(&[0; 12]); // ports
        m.extend_from_slice(&MACH_EXCEPTION_RAISE.to_ne_bytes());
        m.extend_from_slice(&2u32.to_ne_bytes());
        for port in [7u32, 9] {
            m.extend_from_slice(&port.to_ne_bytes());
            m.extend_from_slice(&[0; 8]);
        }
        m.extend_from_slice(&[0; 8]); // NDR
        m.extend_from_slice(&kind.to_ne_bytes());
        m.extend_from_slice(&(codes.len() as u32).to_ne_bytes());
        for c in codes {
            m.extend_from_slice(&c.to_ne_bytes());
        }
        m
    }

    #[test]
    fn parses_bad_access() {
        let r = parse_raise(&raise(EXC_BAD_ACCESS, &[1, 0xdead_beef])).unwrap();
        assert_eq!((r.thread, r.task), (7, 9));
        assert_eq!(r.exception.kind, EXC_BAD_ACCESS);
        assert_eq!(r.exception.fault_addr(), 0xdead_beef);
        assert_eq!(r.exception.signal(), (11, 1));
        let mut s = String::new();
        r.exception.describe(&mut s);
        assert_eq!(s, "EXC_BAD_ACCESS (KERN_INVALID_ADDRESS) at 0xdeadbeef");
    }

    #[test]
    fn rejects_other_ids_and_truncated() {
        let mut m = raise(EXC_BAD_ACCESS, &[1, 2]);
        assert!(parse_raise(&m[..40]).is_none());
        m[20] ^= 1;
        assert!(parse_raise(&m).is_none());
        assert!(parse_raise(&raise(1, &[1, 2, 3])).is_none());
    }

    #[test]
    fn signal_mapping() {
        let e = |kind, code| MachException {
            kind,
            code,
            subcode: 0,
        };
        assert_eq!(e(EXC_BAD_ACCESS, 2).signal(), (11, 2));
        assert_eq!(e(EXC_BAD_ACCESS, 0x32).signal().0, 10);
        assert_eq!(e(EXC_BAD_INSTRUCTION, 1).signal().0, 4);
        assert_eq!(e(EXC_ARITHMETIC, 1).signal().0, 8);
        assert_eq!(e(EXC_BREAKPOINT, 1).signal().0, 5);
    }

    #[test]
    fn abort_request_round_trips() {
        let mut msg = vec![0u8; 20];
        msg.extend_from_slice(&ABORT_MSG_ID.to_ne_bytes());
        msg.extend_from_slice(&abort_body(77, 6));
        assert_eq!(parse_abort(&msg), Some((77, 6)));
        // A Mach exception message or a truncated one is not an abort request.
        assert_eq!(parse_abort(&raise(EXC_BAD_ACCESS, &[1, 2])), None);
        assert_eq!(parse_abort(&msg[..26]), None);
    }

    #[test]
    fn sigabrt_is_described_as_a_soft_signal() {
        let e = MachException::from_signal(6);
        assert_eq!(e.name(), "EXC_SOFTWARE");
        assert_eq!(e.signal(), (6, 0));
        assert_eq!(e.fault_addr(), 0);
        let mut s = String::new();
        e.describe(&mut s);
        assert_eq!(s, "EXC_SOFTWARE (code=0x10003, subcode=0x6)");
    }

    #[test]
    fn windows_names() {
        assert_eq!(
            windows_exception(0xC000_0005).0,
            "EXCEPTION_ACCESS_VIOLATION"
        );
        assert_eq!(windows_exception(0xC000_00FD).1, "Stack overflow");
        assert_eq!(windows_exception(1).0, "EXCEPTION");
    }
}
