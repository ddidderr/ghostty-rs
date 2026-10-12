//! Native crash context for the event JSON: signal info, CPU registers and a
//! stack trace of the faulting thread.
//!
//! This is what sentry-native's signal handler records on macOS (where
//! Ghostty's Zig build links sentry with the breakpad backend, whose handler
//! captures the faulting thread's machine state): the `siginfo`, the thread
//! state (registers) and the stack. We have no minidump writer that can run
//! from a signal handler on Darwin (a Mach exception port handler thread
//! would be required), so the same information goes into the event itself as
//! `exception.values[0].stacktrace` (`frames` + `registers`) and
//! `mechanism.meta.signal`, following the Sentry event schema.
//!
//! Everything here is allocation free and architecture independent (the
//! Darwin glue in `darwin.rs` only fills [`Native`] from the `ucontext`), so
//! it is unit-tested on every platform.

use std::fmt;

/// Maximum number of stack frames recorded.
pub(crate) const MAX_FRAMES: usize = 64;
/// Maximum number of registers recorded (x86_64: 21, arm64: 34).
pub(crate) const MAX_REGS: usize = 40;

/// Machine/signal context of the faulting thread.
pub(crate) struct Native {
    pub signo: i32,
    pub code: i32,
    /// Fault address (`si_addr`).
    pub addr: u64,
    pub reg_names: [&'static str; MAX_REGS],
    pub reg_vals: [u64; MAX_REGS],
    pub nregs: usize,
    /// Return addresses, innermost first (`frames[0]` is the pc).
    pub frames: [u64; MAX_FRAMES],
    pub nframes: usize,
}

impl Native {
    pub(crate) const fn new() -> Native {
        Native {
            signo: 0,
            code: 0,
            addr: 0,
            reg_names: [""; MAX_REGS],
            reg_vals: [0; MAX_REGS],
            nregs: 0,
            frames: [0; MAX_FRAMES],
            nframes: 0,
        }
    }

    pub(crate) fn push_reg(&mut self, name: &'static str, val: u64) {
        if self.nregs < MAX_REGS {
            self.reg_names[self.nregs] = name;
            self.reg_vals[self.nregs] = val;
            self.nregs += 1;
        }
    }

    pub(crate) fn push_frame(&mut self, addr: u64) {
        if self.nframes < MAX_FRAMES {
            self.frames[self.nframes] = addr;
            self.nframes += 1;
        }
    }

    /// Record the pc and then walk the frame-pointer chain starting at `fp`.
    ///
    /// Both x86_64 and arm64 Darwin keep a frame record `[prev_fp,
    /// return_address]` at `fp`. `read` copies 16 bytes of the crashed
    /// process' memory, returning false instead of faulting when the address
    /// is unmapped. The walk stops at a null/misaligned/non-increasing frame
    /// pointer, a failed read or a null return address.
    pub(crate) fn walk(
        &mut self,
        pc: u64,
        mut fp: u64,
        mut read: impl FnMut(u64) -> Option<[u8; 16]>,
    ) {
        self.push_frame(pc);
        let mut prev = 0u64;
        while self.nframes < MAX_FRAMES && fp != 0 && fp.is_multiple_of(8) && fp > prev {
            let Some(rec) = read(fp) else { break };
            let next = u64::from_ne_bytes([
                rec[0], rec[1], rec[2], rec[3], rec[4], rec[5], rec[6], rec[7],
            ]);
            let ret = u64::from_ne_bytes([
                rec[8], rec[9], rec[10], rec[11], rec[12], rec[13], rec[14], rec[15],
            ]);
            if ret == 0 {
                break;
            }
            self.push_frame(ret);
            prev = fp;
            fp = next;
        }
    }
}

/// `si_code` names for the fault signals (POSIX / Darwin `<sys/signal.h>`).
fn code_name(signo: i32, code: i32) -> &'static str {
    match (signo, code) {
        (11, 1) => "SEGV_MAPERR",
        (11, 2) => "SEGV_ACCERR",
        (10, 1) => "BUS_ADRALN",
        (10, 2) => "BUS_ADRERR",
        (10, 3) => "BUS_OBJERR",
        (4, 1) => "ILL_ILLOPC",
        (4, 2) => "ILL_ILLTRP",
        (4, 3) => "ILL_PRVOPC",
        (8, 1) => "FPE_INTDIV",
        (8, 2) => "FPE_INTOVF",
        (8, 3) => "FPE_FLTDIV",
        (8, 4) => "FPE_FLTOVF",
        (8, 5) => "FPE_FLTUND",
        (8, 6) => "FPE_FLTRES",
        (8, 7) => "FPE_FLTINV",
        (_, 0) => "SI_USER",
        _ => "",
    }
}

/// Write `"meta":{...}` (inside the mechanism object) and, after the
/// mechanism, the `stacktrace` of the exception value. The caller has just
/// written `"mechanism":{"type":..,"handled":false`; this appends the
/// remaining mechanism fields and closes the mechanism object.
pub(crate) fn write_mechanism_rest_and_stacktrace<W: fmt::Write>(
    w: &mut W,
    n: &Native,
) -> fmt::Result {
    write!(
        w,
        ",\"meta\":{{\"signal\":{{\"number\":{},\"code\":{}",
        n.signo, n.code
    )?;
    let name = code_name(n.signo, n.code);
    if !name.is_empty() {
        write!(w, ",\"code_name\":\"{name}\"")?;
    }
    write!(
        w,
        "}}}},\"data\":{{\"relevant_address\":\"0x{:x}\"}}}}",
        n.addr
    )?;

    // Sentry wants frames oldest first.
    w.write_str(",\"stacktrace\":{\"frames\":[")?;
    for (i, addr) in n.frames[..n.nframes].iter().rev().enumerate() {
        if i > 0 {
            w.write_char(',')?;
        }
        write!(w, "{{\"instruction_addr\":\"0x{addr:x}\"}}")?;
    }
    w.write_str("],\"registers\":{")?;
    for i in 0..n.nregs {
        if i > 0 {
            w.write_char(',')?;
        }
        write!(w, "\"{}\":\"0x{:x}\"", n.reg_names[i], n.reg_vals[i])?;
    }
    w.write_str("}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Native {
        let mut n = Native::new();
        n.signo = 11;
        n.code = 1;
        n.addr = 0xdead_beef;
        n.push_reg("pc", 0x1000);
        n.push_reg("sp", 0x7000);
        n.walk(0x1000, 0x7000, |fp| {
            // Two frames: [0x7100, 0x2000], [0x7200, 0x3000], then end.
            let (next, ret) = match fp {
                0x7000 => (0x7100u64, 0x2000u64),
                0x7100 => (0x7200, 0x3000),
                _ => return None,
            };
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&next.to_ne_bytes());
            b[8..].copy_from_slice(&ret.to_ne_bytes());
            Some(b)
        });
        n
    }

    #[test]
    fn walks_frame_pointer_chain() {
        let n = sample();
        assert_eq!(&n.frames[..n.nframes], &[0x1000, 0x2000, 0x3000]);
    }

    #[test]
    fn walk_stops_on_cycles_and_bad_pointers() {
        let mut n = Native::new();
        // fp chain pointing at itself: must terminate.
        n.walk(1, 0x7000, |_| {
            let mut b = [0u8; 16];
            b[..8].copy_from_slice(&0x7000u64.to_ne_bytes());
            b[8..].copy_from_slice(&0x42u64.to_ne_bytes());
            Some(b)
        });
        assert_eq!(&n.frames[..n.nframes], &[1, 0x42]);
        // Misaligned fp: no read at all.
        let mut m = Native::new();
        m.walk(1, 0x7001, |_| panic!("read"));
        assert_eq!(m.nframes, 1);
    }

    #[test]
    fn event_json_is_valid_with_registers_and_frames() {
        let n = sample();
        let mut s = String::from("{\"mechanism\":{\"type\":\"signalhandler\",\"handled\":false");
        write_mechanism_rest_and_stacktrace(&mut s, &n).expect("format");
        s.push('}');
        let v: serde_json::Value = serde_json::from_str(&s).expect("valid json");
        let mech = &v["mechanism"];
        assert_eq!(mech["meta"]["signal"]["number"], 11);
        assert_eq!(mech["meta"]["signal"]["code_name"], "SEGV_MAPERR");
        assert_eq!(mech["data"]["relevant_address"], "0xdeadbeef");
        let frames = v["stacktrace"]["frames"].as_array().expect("array");
        // Oldest first.
        assert_eq!(frames[0]["instruction_addr"], "0x3000");
        assert_eq!(frames[2]["instruction_addr"], "0x1000");
        assert_eq!(v["stacktrace"]["registers"]["pc"], "0x1000");
        assert_eq!(v["stacktrace"]["registers"]["sp"], "0x7000");
    }
}
