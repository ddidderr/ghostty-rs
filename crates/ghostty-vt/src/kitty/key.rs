//! Kitty keyboard protocol support (ZIG/src/terminal/kitty/key.zig).

/// Stack for the key flags. This implements the push/pop behavior
/// of the CSI > u and CSI < u sequences. We implement the stack as
/// fixed size to avoid heap allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlagStack {
    pub flags: [Flags; FlagStack::LEN],
    /// Index into `flags`; wraps modulo `LEN` (a `u3` in Zig).
    pub idx: u8,
}

impl FlagStack {
    pub const LEN: usize = 8;

    /// Return the current stack value
    pub fn current(&self) -> Flags {
        self.flags[self.idx as usize]
    }

    /// Perform the "set" operation as described in the spec for
    /// the CSI = u sequence.
    pub fn set(&mut self, mode: SetMode, v: Flags) {
        let cur = &mut self.flags[self.idx as usize];
        *cur = match mode {
            SetMode::Set => v,
            SetMode::Or => Flags::from_int(cur.int() | v.int()),
            SetMode::Not => Flags::from_int(cur.int() & !v.int()),
        };
    }

    /// Push a new set of flags onto the stack. If the stack is full
    /// then the oldest entry is evicted.
    pub fn push(&mut self, flags: Flags) {
        self.idx = (self.idx + 1) % Self::LEN as u8;
        self.flags[self.idx as usize] = flags;
    }

    /// Pop `n` entries from the stack. This will just wrap around
    /// if `n` is greater than the amount in the stack.
    pub fn pop(&mut self, n: usize) {
        // If n is more than our length then we just reset the stack.
        // This also avoids a DoS vector where a malicious client
        // could send a huge number of pop commands to waste cpu.
        if n >= Self::LEN {
            self.idx = 0;
            self.flags = [Flags::DISABLED; Self::LEN];
            return;
        }

        for _ in 0..n {
            self.flags[self.idx as usize] = Flags::DISABLED;
            self.idx = (self.idx + Self::LEN as u8 - 1) % Self::LEN as u8;
        }
    }
}

/// The possible flags for the Kitty keyboard protocol.
/// Bit order matches the Zig `packed struct(u5)` (disambiguate = bit 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[allow(clippy::struct_excessive_bools)]
pub struct Flags {
    pub disambiguate: bool,
    pub report_events: bool,
    pub report_alternates: bool,
    pub report_all: bool,
    pub report_associated: bool,
}

impl Flags {
    /// Kitty keyboard protocol disabled (all flags off).
    pub const DISABLED: Flags = Flags {
        disambiguate: false,
        report_events: false,
        report_alternates: false,
        report_all: false,
        report_associated: false,
    };

    /// Sets all modes on.
    pub const TRUE: Flags = Flags {
        disambiguate: true,
        report_events: true,
        report_alternates: true,
        report_all: true,
        report_associated: true,
    };

    /// The flags as a 5-bit integer.
    pub fn int(self) -> u8 {
        u8::from(self.disambiguate)
            | u8::from(self.report_events) << 1
            | u8::from(self.report_alternates) << 2
            | u8::from(self.report_all) << 3
            | u8::from(self.report_associated) << 4
    }

    /// Build flags from the low 5 bits of `v`.
    pub fn from_int(v: u8) -> Flags {
        Flags {
            disambiguate: v & 1 != 0,
            report_events: v & 2 != 0,
            report_alternates: v & 4 != 0,
            report_all: v & 8 != 0,
            report_associated: v & 16 != 0,
        }
    }
}

/// The possible modes for setting the Kitty keyboard protocol flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SetMode {
    Set = 1,
    Or = 2,
    Not = 3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_stack_overflow() {
        let mut stack = FlagStack {
            idx: (FlagStack::LEN - 1) as u8,
            ..FlagStack::default()
        };
        stack.push(Flags::DISABLED);
        assert_eq!(stack.idx, 0);
        stack.pop(1);
        assert_eq!(stack.idx as usize, FlagStack::LEN - 1);
    }

    #[test]
    fn flags_int() {
        assert_eq!(Flags::DISABLED.int(), 0b0_0000);
        assert_eq!(
            Flags {
                disambiguate: true,
                ..Flags::DISABLED
            }
            .int(),
            0b0_0001
        );
        assert_eq!(
            Flags {
                report_events: true,
                ..Flags::DISABLED
            }
            .int(),
            0b0_0010
        );
        assert_eq!(Flags::TRUE.int(), 0b1_1111);
        assert_eq!(Flags::from_int(0b1_0100).int(), 0b1_0100);
    }

    #[test]
    fn flag_stack_set_push_pop() {
        let mut stack = FlagStack::default();
        stack.set(SetMode::Set, Flags::from_int(1));
        stack.set(SetMode::Or, Flags::from_int(4));
        assert_eq!(stack.current().int(), 5);
        stack.set(SetMode::Not, Flags::from_int(1));
        assert_eq!(stack.current().int(), 4);
        stack.push(Flags::from_int(2));
        assert_eq!(stack.current().int(), 2);
        stack.pop(1);
        assert_eq!(stack.current().int(), 4);
        stack.pop(100);
        assert_eq!(stack, FlagStack::default());
    }

    #[test]
    fn flag_stack_push_pop() {
        let mut stack = FlagStack::default();
        stack.push(Flags {
            disambiguate: true,
            ..Flags::DISABLED
        });
        assert_eq!(
            stack.current(),
            Flags {
                disambiguate: true,
                ..Flags::DISABLED
            }
        );
        stack.pop(1);
        assert_eq!(stack.current(), Flags::DISABLED);
    }

    #[test]
    fn flag_stack_pop_big_number() {
        let mut stack = FlagStack::default();
        stack.pop(100);
        assert_eq!(stack.current(), Flags::DISABLED);
    }

    #[test]
    fn flag_stack_set() {
        let mut stack = FlagStack::default();
        let dis = Flags {
            disambiguate: true,
            ..Flags::DISABLED
        };
        let ev = Flags {
            report_events: true,
            ..Flags::DISABLED
        };
        stack.set(SetMode::Set, dis);
        assert_eq!(stack.current(), dis);
        stack.set(SetMode::Or, ev);
        assert_eq!(
            stack.current(),
            Flags {
                disambiguate: true,
                report_events: true,
                ..Flags::DISABLED
            }
        );
        stack.set(SetMode::Not, ev);
        assert_eq!(stack.current(), dis);
    }
}
