//! DCS command handler (ZIG/src/terminal/dcs.zig). This should be hooked
//! into a stream. The hook/put/unhook functions are meant to be called from
//! the stream's dcsHook, dcsPut, and dcsUnhook functions, respectively.

use crate::parser::Dcs;
use crate::tmux::control::{Notification, Parser as TmuxParser};

/// DCS command handler.
#[derive(Debug)]
pub struct Handler {
    pub state: State,

    /// Maximum bytes any DCS command can take. This is to prevent
    /// malicious input from causing us to allocate too much memory.
    /// This is arbitrarily set to 1MB today, increase if needed.
    pub max_bytes: usize,
}

impl Default for Handler {
    fn default() -> Self {
        Self {
            state: State::Inactive,
            max_bytes: 1024 * 1024,
        }
    }
}

struct Hook {
    state: State,
    command: Option<Command>,
}

impl Handler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hook(&mut self, dcs: &Dcs) -> Option<Command> {
        debug_assert!(matches!(self.state, State::Inactive));

        // Initialize our state to ignore in case of error
        self.state = State::Ignore;

        // Try to parse the hook.
        let Some(hk) = self.try_hook(dcs) else {
            log::info!("unknown DCS hook: {dcs:?}");
            return None;
        };

        self.state = hk.state;
        hk.command
    }

    fn try_hook(&self, dcs: &Dcs) -> Option<Hook> {
        let params = dcs.params();
        match dcs.intermediates() {
            [] => match dcs.final_ {
                // Tmux control mode
                b'p' => {
                    // Tmux control mode must start with ESC P 1000 p
                    if params.len() != 1 || params[0] != 1000 {
                        return None;
                    }

                    let mut parser = TmuxParser::new();
                    parser.max_bytes = self.max_bytes;
                    Some(Hook {
                        state: State::Tmux(Box::new(parser)),
                        command: Some(Command::Tmux(Notification::Enter)),
                    })
                }

                _ => None,
            },

            [b'+'] => match dcs.final_ {
                // XTGETTCAP
                // https://github.com/mitchellh/ghostty/issues/517
                b'q' => Some(Hook {
                    state: State::Xtgettcap(Vec::with_capacity(128)),
                    command: None,
                }),

                _ => None,
            },

            [b'$'] => match dcs.final_ {
                // DECRQSS
                b'q' => Some(Hook {
                    state: State::Decrqss(DecrqssBuffer::default()),
                    command: None,
                }),

                _ => None,
            },

            _ => None,
        }
    }

    /// Put a byte into the DCS handler. This will return a command
    /// if a command needs to be executed.
    pub fn put(&mut self, byte: u8) -> Option<Command> {
        let Ok(v) = self.try_put(byte) else {
            // On error we just discard our state and ignore the rest
            log::info!("error putting byte into DCS handler err=OutOfMemory");
            self.discard();
            self.state = State::Ignore;
            return None;
        };
        v
    }

    fn try_put(&mut self, byte: u8) -> Result<Option<Command>, ()> {
        match &mut self.state {
            State::Inactive | State::Ignore => {}

            State::Tmux(tmux) => {
                return match tmux.put(byte) {
                    Ok(n) => Ok(n.map(Command::Tmux)),
                    Err(_) => Err(()),
                };
            }

            State::Xtgettcap(list) => {
                if list.len() >= self.max_bytes {
                    return Err(());
                }
                list.push(byte);
            }

            State::Decrqss(buffer) => {
                if buffer.len >= buffer.data.len() {
                    return Err(());
                }
                buffer.data[buffer.len] = byte;
                buffer.len += 1;
            }
        }

        Ok(None)
    }

    pub fn unhook(&mut self) -> Option<Command> {
        let state = std::mem::replace(&mut self.state, State::Inactive);
        match state {
            State::Inactive | State::Ignore => None,

            State::Tmux(_) => Some(Command::Tmux(Notification::Exit)),

            State::Xtgettcap(mut list) => {
                list.make_ascii_uppercase();
                Some(Command::Xtgettcap(Xtgettcap { data: list, i: 0 }))
            }

            State::Decrqss(buffer) => Some(Command::Decrqss(match buffer.len {
                0 => Decrqss::None,
                1 => match buffer.data[0] {
                    b'm' => Decrqss::Sgr,
                    b'r' => Decrqss::Decstbm,
                    b's' => Decrqss::Decslrm,
                    _ => Decrqss::None,
                },
                2 => match buffer.data[0] {
                    b' ' => match buffer.data[1] {
                        b'q' => Decrqss::Decscusr,
                        _ => Decrqss::None,
                    },
                    _ => Decrqss::None,
                },
                _ => unreachable!(),
            })),
        }
    }

    fn discard(&mut self) {
        self.state = State::Inactive;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// XTGETTCAP
    Xtgettcap(Xtgettcap),

    /// DECRQSS
    Decrqss(Decrqss),

    /// Tmux control mode
    Tmux(Notification),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Xtgettcap {
    pub data: Vec<u8>,
    pub i: usize,
}

impl Xtgettcap {
    /// Returns the next terminfo key being requested and None
    /// when there are no more keys. The returned value is NOT hex-decoded
    /// because we expect to use a comptime lookup table.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<&[u8]> {
        let items = &self.data;
        if self.i >= items.len() {
            return None;
        }
        let rem = &items[self.i..];
        let idx = rem.iter().position(|&b| b == b';').unwrap_or(rem.len());

        // Note that if we're at the end, idx + 1 is len + 1 so we're over
        // the end but that's okay because our check above is >= so we'll
        // never read.
        self.i += idx + 1;

        Some(&rem[..idx])
    }
}

/// Supported DECRQSS settings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decrqss {
    None,
    Sgr,
    Decscusr,
    Decstbm,
    Decslrm,
}

#[derive(Debug, Default)]
pub struct DecrqssBuffer {
    data: [u8; 2],
    len: usize,
}

#[derive(Debug, Default)]
pub enum State {
    /// We're not in a DCS state at the moment.
    #[default]
    Inactive,

    /// We're hooked, but its an unknown DCS command or one that went
    /// invalid due to some bad input, so we're ignoring the rest.
    Ignore,

    /// XTGETTCAP
    Xtgettcap(Vec<u8>),

    /// DECRQSS
    Decrqss(DecrqssBuffer),

    /// Tmux control mode: https://github.com/tmux/tmux/wiki/Control-Mode
    Tmux(Box<TmuxParser>),
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn dcs(inter: &[u8], params: &[u16], final_: u8) -> Dcs {
        Dcs::new(inter, params, final_)
    }

    #[test]
    fn unknown_dcs_command() {
        let mut h = Handler::new();
        assert!(h.hook(&dcs(b"", &[], b'A')).is_none());
        assert!(matches!(h.state, State::Ignore));
        assert!(h.unhook().is_none());
        assert!(matches!(h.state, State::Inactive));
    }

    fn xtgettcap(input: &str) -> Xtgettcap {
        let mut h = Handler::new();
        assert!(h.hook(&dcs(b"+", &[], b'q')).is_none());
        for b in input.bytes() {
            let _ = h.put(b);
        }
        match h.unhook().unwrap() {
            Command::Xtgettcap(x) => x,
            _ => panic!("expected xtgettcap"),
        }
    }

    #[test]
    fn xtgettcap_command() {
        let mut cmd = xtgettcap("536D756C78");
        assert_eq!(cmd.next(), Some(&b"536D756C78"[..]));
        assert_eq!(cmd.next(), None);
    }

    #[test]
    fn xtgettcap_mixed_case() {
        let mut cmd = xtgettcap("536d756C78");
        assert_eq!(cmd.next(), Some(&b"536D756C78"[..]));
        assert_eq!(cmd.next(), None);
    }

    #[test]
    fn xtgettcap_command_multiple_keys() {
        let mut cmd = xtgettcap("536D756C78;536D756C78");
        assert_eq!(cmd.next(), Some(&b"536D756C78"[..]));
        assert_eq!(cmd.next(), Some(&b"536D756C78"[..]));
        assert_eq!(cmd.next(), None);
    }

    #[test]
    fn xtgettcap_command_invalid_data() {
        let mut cmd = xtgettcap("who;536D756C78");
        assert_eq!(cmd.next(), Some(&b"WHO"[..]));
        assert_eq!(cmd.next(), Some(&b"536D756C78"[..]));
        assert_eq!(cmd.next(), None);
    }

    #[test]
    fn xtgettcap_max_bytes() {
        let mut h = Handler::new();
        h.max_bytes = 2;
        assert!(h.hook(&dcs(b"+", &[], b'q')).is_none());
        for b in b"abc" {
            let _ = h.put(*b);
        }
        assert!(matches!(h.state, State::Ignore));
        assert!(h.unhook().is_none());
    }

    #[test]
    fn decrqss_command() {
        let mut h = Handler::new();
        assert!(h.hook(&dcs(b"$", &[], b'q')).is_none());
        let _ = h.put(b'm');
        assert_eq!(h.unhook(), Some(Command::Decrqss(Decrqss::Sgr)));
    }

    #[test]
    fn decrqss_invalid_command() {
        let mut h = Handler::new();
        assert!(h.hook(&dcs(b"$", &[], b'q')).is_none());
        let _ = h.put(b'z');
        assert_eq!(h.unhook(), Some(Command::Decrqss(Decrqss::None)));

        h.discard();

        assert!(h.hook(&dcs(b"$", &[], b'q')).is_none());
        let _ = h.put(b'"');
        let _ = h.put(b' ');
        let _ = h.put(b'q');
        assert!(h.unhook().is_none());
    }

    #[test]
    fn tmux_enter_and_implicit_exit() {
        let mut h = Handler::new();

        let cmd = h.hook(&dcs(b"", &[1000], b'p')).unwrap();
        assert_eq!(cmd, Command::Tmux(Notification::Enter));

        let cmd = h.unhook().unwrap();
        assert_eq!(cmd, Command::Tmux(Notification::Exit));
    }

    #[test]
    fn tmux_requires_1000_param() {
        let mut h = Handler::new();
        assert!(h.hook(&dcs(b"", &[1], b'p')).is_none());
        assert!(matches!(h.state, State::Ignore));
    }
}
