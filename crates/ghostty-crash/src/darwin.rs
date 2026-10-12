//! Darwin glue for [`crate::native`]: fill the native crash context from the
//! `siginfo_t` and `ucontext_t` passed to the fatal-signal handler.
//!
//! The handler runs on the faulting thread, so the `ucontext` holds the
//! thread state at the fault (the equivalent of `thread_get_state` on the
//! crashed thread that breakpad's Mach exception handler does). Memory is read
//! with `mach_vm_read_overwrite`, which returns an error instead of faulting
//! for unmapped addresses, so walking a corrupted stack is safe.
#![allow(unsafe_code)]

use crate::native::Native;

/// `struct __darwin_ucontext` (`<sys/_types/_ucontext.h>`, 64-bit).
#[repr(C)]
#[allow(clippy::struct_field_names)]
struct Ucontext {
    uc_onstack: libc::c_int,
    uc_sigmask: u32,
    uc_stack: libc::stack_t,
    uc_link: *mut Ucontext,
    uc_mcsize: usize,
    /// `struct __darwin_mcontext64 *`: exception state (16 bytes on both
    /// x86_64 and arm64) followed by the thread state.
    uc_mcontext: *const u8,
}

#[cfg(target_arch = "aarch64")]
const X_NAMES: [&str; 29] = [
    "x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13", "x14",
    "x15", "x16", "x17", "x18", "x19", "x20", "x21", "x22", "x23", "x24", "x25", "x26", "x27",
    "x28",
];

/// Size of `__darwin_x86_exception_state64` / `__darwin_arm_exception_state64`,
/// which precedes the thread state in `__darwin_mcontext64`.
const EXCEPTION_STATE_SIZE: usize = 16;

fn read_mem(addr: u64) -> Option<[u8; 16]> {
    let mut buf = [0u8; 16];
    let mut out: mach2::vm_types::mach_vm_size_t = 0;
    // SAFETY: `buf` is a valid 16 byte destination; the call validates the
    // source address and fails instead of faulting.
    let kr = unsafe {
        mach2::vm::mach_vm_read_overwrite(
            mach2::traps::mach_task_self(),
            addr,
            16,
            buf.as_mut_ptr() as mach2::vm_types::mach_vm_address_t,
            &raw mut out,
        )
    };
    (kr == mach2::kern_return::KERN_SUCCESS && out == 16).then_some(buf)
}

/// Fill `n` from the handler arguments.
///
/// # Safety
/// `info` and `uc` must be the pointers the kernel passed to an `SA_SIGINFO`
/// handler (or null).
pub(crate) unsafe fn capture(
    n: &mut Native,
    info: *const libc::siginfo_t,
    uc: *const libc::c_void,
) {
    // SAFETY: per the contract (a non-null siginfo is valid for reads).
    if let Some(info) = unsafe { info.as_ref() } {
        n.signo = info.si_signo;
        n.code = info.si_code;
        n.addr = info.si_addr as u64;
    }
    // SAFETY: per the contract.
    let Some(uc) = (unsafe { uc.cast::<Ucontext>().as_ref() }) else {
        return;
    };
    if uc.uc_mcontext.is_null() {
        return;
    }
    // SAFETY: the mcontext holds the exception state followed by the thread
    // state; the kernel built it for this signal frame.
    let ss = unsafe { uc.uc_mcontext.add(EXCEPTION_STATE_SIZE) };

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: as above; the layout is `x86_thread_state64_t`.
        let s = unsafe { &*ss.cast::<mach2::structs::x86_thread_state64_t>() };
        for (name, v) in [
            ("rax", s.__rax),
            ("rbx", s.__rbx),
            ("rcx", s.__rcx),
            ("rdx", s.__rdx),
            ("rdi", s.__rdi),
            ("rsi", s.__rsi),
            ("rbp", s.__rbp),
            ("rsp", s.__rsp),
            ("r8", s.__r8),
            ("r9", s.__r9),
            ("r10", s.__r10),
            ("r11", s.__r11),
            ("r12", s.__r12),
            ("r13", s.__r13),
            ("r14", s.__r14),
            ("r15", s.__r15),
            ("rip", s.__rip),
            ("rflags", s.__rflags),
            ("cs", s.__cs),
            ("fs", s.__fs),
            ("gs", s.__gs),
        ] {
            n.push_reg(name, v);
        }
        n.walk(s.__rip, s.__rbp, read_mem);
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: as above; the layout is `arm_thread_state64_t`.
        let s = unsafe { &*ss.cast::<mach2::structs::arm_thread_state64_t>() };
        for (name, v) in X_NAMES.iter().zip(s.__x) {
            n.push_reg(name, v);
        }
        n.push_reg("fp", s.__fp);
        n.push_reg("lr", s.__lr);
        n.push_reg("sp", s.__sp);
        n.push_reg("pc", s.__pc);
        n.push_reg("cpsr", u64::from(s.__cpsr));
        // arm64 may sign pointers (PAC): strip the high bits before use.
        let strip = |v: u64| v & 0x0000_007f_ffff_ffff;
        // The pc is exact; the faulting leaf function did not necessarily
        // push a frame record, so the link register is the caller.
        n.walk(s.__pc, strip(s.__fp), read_mem);
        if n.nframes == 1 && s.__lr != 0 {
            n.push_frame(strip(s.__lr));
        }
    }
}
