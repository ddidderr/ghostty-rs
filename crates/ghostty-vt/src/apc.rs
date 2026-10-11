//! APC command handler (ZIG/src/terminal/apc.zig). This should be hooked
//! into a stream. The start/feed/end functions are meant to be called from
//! the stream's apcStart, apcPut, and apcEnd functions, respectively.

use crate::kitty::graphics_command as kitty_gfx;

#[derive(Debug, Default)]
pub enum State {
    /// We're not in the middle of an APC command yet.
    #[default]
    Inactive,

    /// We got an unrecognized APC sequence or the APC sequence we
    /// recognized became invalid. We're just dropping bytes.
    Ignore,

    /// We're waiting to identify the APC sequence. This is done by
    /// inspecting the first byte of the sequence.
    Identify,

    /// Kitty graphics protocol
    Kitty(kitty_gfx::Parser),
}

/// Possible APC commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Kitty(kitty_gfx::Command),
}

#[derive(Debug, Default)]
pub struct Handler {
    pub state: State,
}

impl Handler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self) {
        self.state = State::Identify;
    }

    pub fn feed(&mut self, byte: u8) {
        match &mut self.state {
            State::Inactive => unreachable!("APC feed while inactive"),

            // We're ignoring this APC command, likely because we don't
            // recognize it so there is no need to store the data in memory.
            State::Ignore => {}

            // We identify the APC command by the first byte.
            State::Identify => {
                self.state = match byte {
                    // Kitty graphics protocol
                    b'G' => State::Kitty(kitty_gfx::Parser::new()),

                    // Unknown
                    _ => State::Ignore,
                };
            }

            State::Kitty(p) => {
                if let Err(err) = p.feed(byte) {
                    log::warn!("kitty graphics protocol error: {err}");
                    self.state = State::Ignore;
                }
            }
        }
    }

    pub fn end(&mut self) -> Option<Command> {
        let state = std::mem::take(&mut self.state);
        match state {
            State::Inactive => unreachable!("APC end while inactive"),
            State::Ignore | State::Identify => None,
            State::Kitty(mut p) => match p.complete() {
                Ok(command) => Some(Command::Kitty(command)),
                Err(err) => {
                    log::warn!("kitty graphics protocol error: {err}");
                    None
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn run(input: &str) -> Option<Command> {
        let mut h = Handler::new();
        h.start();
        for c in input.bytes() {
            h.feed(c);
        }
        h.end()
    }

    #[test]
    fn unknown_apc_command() {
        assert!(run("Xabcdef1234").is_none());
    }

    #[test]
    fn garbage_kitty_command() {
        assert!(run("Gabcdef1234").is_none());
    }

    #[test]
    fn kitty_command_with_overflow_u32() {
        assert!(run("Ga=p,i=10000000000").is_none());
    }

    #[test]
    fn kitty_command_with_overflow_i32() {
        assert!(run("Ga=p,i=1,z=-9999999999").is_none());
    }

    #[test]
    fn valid_kitty_command() {
        let cmd = run("Gf=24,s=10,v=20,hello=world").unwrap();
        assert!(matches!(cmd, Command::Kitty(_)));
    }

    #[test]
    fn end_resets_to_inactive() {
        let mut h = Handler::new();
        h.start();
        h.feed(b'X');
        assert!(h.end().is_none());
        assert!(matches!(h.state, State::Inactive));
    }
}
