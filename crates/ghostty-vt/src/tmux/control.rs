//! This file contains the implementation for tmux control mode
//! (ZIG/src/terminal/tmux/control.zig). See tmux(1) for more information on
//! control mode. Some basics are documented here but this is not meant to be
//! a comprehensive source of protocol documentation.
//!
//! Deviations from the Zig version:
//! - Notifications own their data (`Vec<u8>`) instead of borrowing from the
//!   parser's buffer, which avoids self-referential lifetimes.
//! - Oniguruma regexes are replaced by hand-written matchers that implement
//!   the same (greedy, backtracking) semantics for the specific patterns.
//! - A numeric ID that overflows `usize` is treated as a failed match (Zig
//!   would hit `unreachable`).

/// Errors returned by [`Parser::put`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("out of memory")]
    OutOfMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Outside of any active notifications. This should drop any output
    /// unless it is '%' on the first byte of a line. The buffer will be
    /// cleared when it sees '%', this is so that the previous notification
    /// data is valid until we receive/process new data.
    Idle,

    /// We experienced unexpected input and are in a broken state
    /// so we cannot continue processing. When this state is set,
    /// the buffer has been freed and must not be accessed.
    Broken,

    /// Inside an active notification (started with '%').
    Notification,

    /// Inside a begin/end block.
    Block,
}

/// A tmux control mode parser. This takes in output from tmux control
/// mode and parses it into a structured notifications.
///
/// It is up to the caller to establish the connection to the tmux
/// control mode session in some way (e.g. via exec, a network socket,
/// whatever). This is fully agnostic to how the data is received and sent.
#[derive(Debug)]
pub struct Parser {
    /// Current state of the client.
    state: State,

    /// The buffer used to store in-progress notifications, output, etc.
    buffer: Vec<u8>,

    /// The maximum size in bytes of the buffer. This is used to limit
    /// memory usage. If the buffer exceeds this size, the client will
    /// enter a broken state (the control mode session will be forcibly
    /// exited and future data dropped).
    pub max_bytes: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            buffer: Vec::new(),
            max_bytes: 1024 * 1024,
        }
    }

    /// Handle a byte of input.
    ///
    /// If we reach our byte limit this will return `OutOfMemory`. It only
    /// does this on the first time we exceed the limit; subsequent calls
    /// will return None as we drop all input in a broken state.
    pub fn put(&mut self, byte: u8) -> Result<Option<Notification>, Error> {
        // If we're in a broken state, just do nothing.
        if self.state == State::Broken {
            return Ok(None);
        }

        if self.buffer.len() >= self.max_bytes {
            self.broken();
            return Err(Error::OutOfMemory);
        }

        match self.state {
            State::Broken => return Ok(None),

            // Waiting for a notification so if the byte is not '%' then
            // we're in a broken state. Control mode output should always
            // be wrapped in '%begin/%end' orelse we expect a notification.
            // Return an exit notification.
            State::Idle => {
                if byte == b'%' {
                    self.buffer.clear();
                    self.state = State::Notification;
                } else {
                    self.broken();
                    return Ok(Some(Notification::Exit));
                }
            }

            // If we're in a notification and its not a newline then
            // we accumulate. If it is a newline then we have a
            // complete notification we need to parse.
            State::Notification => {
                if byte == b'\n' {
                    // If parsing failed we do not mark the state as broken
                    // because we may be able to continue parsing other types
                    // of notifications.
                    return Ok(self.parse_notification());
                }
            }

            // If we're in a block then we accumulate until we see a newline
            // and then we check to see if that line ended the block.
            State::Block => {
                if byte == b'\n' {
                    let idx = self
                        .buffer
                        .iter()
                        .rposition(|&b| b == b'\n')
                        .map_or(0, |v| v + 1);
                    let line = &self.buffer[idx..];

                    if line.starts_with(b"%end") || line.starts_with(b"%error") {
                        let err = line.starts_with(b"%error");
                        let output = trim_right_crlf(&self.buffer[..idx]).to_vec();

                        // If it is an error then log it.
                        if err {
                            log::warn!(
                                "tmux control mode error={}",
                                String::from_utf8_lossy(&output)
                            );
                        }

                        self.state = State::Idle;
                        return Ok(Some(if err {
                            Notification::BlockErr(output)
                        } else {
                            Notification::BlockEnd(output)
                        }));
                    }

                    // Didn't end the block, continue accumulating.
                }
            }
        }

        self.buffer.push(byte);
        Ok(None)
    }

    fn parse_notification(&mut self) -> Option<Notification> {
        debug_assert_eq!(self.state, State::Notification);

        let mut line: &[u8] = &self.buffer;
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        let cmd_end = line.iter().position(|&b| b == b' ').unwrap_or(line.len());
        let cmd = &line[..cmd_end];

        // The notification MUST exist because we guard entering the notification
        // state on seeing at least a '%'.
        let result: Option<Notification> = match cmd {
            b"%begin" => {
                // We don't use the rest of the tokens for now because tmux
                // claims to guarantee that begin/end are always in order and
                // never intermixed.

                // Move to block state because we expect a corresponding end/error
                // and want to accumulate the data.
                self.state = State::Block;
                self.buffer.clear();
                return None;
            }
            b"%output" => match_output(line),
            b"%session-changed" => match_session_changed(line),
            b"%sessions-changed" => {
                if line == b"%sessions-changed" {
                    Some(Notification::SessionsChanged)
                } else {
                    log::warn!(
                        "failed to match notification cmd=%sessions-changed line=\"{}\"",
                        String::from_utf8_lossy(line)
                    );
                    None
                }
            }
            b"%layout-change" => match_layout_change(line),
            b"%window-add" => match_window_add(line),
            b"%window-renamed" => match_window_renamed(line),
            b"%window-pane-changed" => match_window_pane_changed(line),
            b"%client-detached" => match_client_detached(line),
            b"%client-session-changed" => match_client_session_changed(line),
            _ => {
                // Unknown notification, log it and return to idle state.
                log::warn!(
                    "unknown tmux control mode notification={}",
                    String::from_utf8_lossy(cmd)
                );
                None
            }
        };

        if result.is_none() && cmd_is_known(cmd) && cmd != b"%sessions-changed" {
            log::warn!(
                "failed to match notification cmd={} line=\"{}\"",
                String::from_utf8_lossy(cmd),
                String::from_utf8_lossy(line)
            );
        }

        // Whether matched or not, clear the buffer and return to idle. (The
        // Zig code avoids clearing when slices point into the buffer, but we
        // own our data so clearing is always safe.)
        self.buffer.clear();
        self.state = State::Idle;
        result
    }

    /// Mark the tmux state as broken.
    fn broken(&mut self) {
        self.state = State::Broken;
        self.buffer = Vec::new();
    }
}

fn cmd_is_known(cmd: &[u8]) -> bool {
    matches!(
        cmd,
        b"%output"
            | b"%session-changed"
            | b"%sessions-changed"
            | b"%layout-change"
            | b"%window-add"
            | b"%window-renamed"
            | b"%window-pane-changed"
            | b"%client-detached"
            | b"%client-session-changed"
    )
}

fn trim_right_crlf(mut s: &[u8]) -> &[u8] {
    while let Some((&last, rest)) = s.split_last() {
        if last == b'\r' || last == b'\n' {
            s = rest;
        } else {
            break;
        }
    }
    s
}

// ---------------------------------------------------------------------
// Hand-written matchers equivalent to the Zig regexes. `.` does not match
// '\n' in Oniguruma, but lines never contain '\n' here.

/// Parse `[0-9]+` at the start of `s`; returns (value, rest).
fn digits(s: &[u8]) -> Option<(usize, &[u8])> {
    let n = s.iter().take_while(|b| b.is_ascii_digit()).count();
    if n == 0 {
        return None;
    }
    let v = std::str::from_utf8(&s[..n]).ok()?.parse::<usize>().ok()?;
    Some((v, &s[n..]))
}

/// Strip an exact literal prefix.
fn lit<'a>(s: &'a [u8], p: &[u8]) -> Option<&'a [u8]> {
    s.strip_prefix(p)
}

/// `(.+)` followed by end: non-empty remainder.
fn nonempty(s: &[u8]) -> Option<&[u8]> {
    if s.is_empty() { None } else { Some(s) }
}

fn match_output(line: &[u8]) -> Option<Notification> {
    // ^%output %([0-9]+) (.+)$
    let s = lit(line, b"%output %")?;
    let (id, s) = digits(s)?;
    let s = lit(s, b" ")?;
    let data = nonempty(s)?;
    Some(Notification::Output {
        pane_id: id,
        data: data.to_vec(),
    })
}

fn match_session_changed(line: &[u8]) -> Option<Notification> {
    // ^%session-changed \$([0-9]+) (.+)$
    let s = lit(line, b"%session-changed $")?;
    let (id, s) = digits(s)?;
    let s = lit(s, b" ")?;
    let name = nonempty(s)?;
    Some(Notification::SessionChanged {
        id,
        name: name.to_vec(),
    })
}

fn match_layout_change(line: &[u8]) -> Option<Notification> {
    // ^%layout-change @([0-9]+) (.+) (.+) (.*)$
    let s = lit(line, b"%layout-change @")?;
    let (id, s) = digits(s)?;
    let rest = lit(s, b" ")?;
    // Greedy group 2, then group 3 (non-empty), then group 4 (any).
    // The last space starts group 4; group 2 ends at the last space at or
    // before `last - 2` (so that group 3 is non-empty) and at index >= 1.
    let last = rest.iter().rposition(|&b| b == b' ')?;
    if last < 3 {
        return None;
    }
    let p = rest[1..=last - 2].iter().rposition(|&b| b == b' ')? + 1;
    let layout = &rest[..p];
    let visible = &rest[p + 1..last];
    let flags = &rest[last + 1..];
    Some(Notification::LayoutChange {
        window_id: id,
        layout: layout.to_vec(),
        visible_layout: visible.to_vec(),
        raw_flags: flags.to_vec(),
    })
}

fn match_window_add(line: &[u8]) -> Option<Notification> {
    // ^%window-add @([0-9]+)$
    let s = lit(line, b"%window-add @")?;
    let (id, s) = digits(s)?;
    if !s.is_empty() {
        return None;
    }
    Some(Notification::WindowAdd { id })
}

fn match_window_renamed(line: &[u8]) -> Option<Notification> {
    // ^%window-renamed @([0-9]+) (.+)$
    let s = lit(line, b"%window-renamed @")?;
    let (id, s) = digits(s)?;
    let s = lit(s, b" ")?;
    let name = nonempty(s)?;
    Some(Notification::WindowRenamed {
        id,
        name: name.to_vec(),
    })
}

fn match_window_pane_changed(line: &[u8]) -> Option<Notification> {
    // ^%window-pane-changed @([0-9]+) %([0-9]+)$
    let s = lit(line, b"%window-pane-changed @")?;
    let (window_id, s) = digits(s)?;
    let s = lit(s, b" %")?;
    let (pane_id, s) = digits(s)?;
    if !s.is_empty() {
        return None;
    }
    Some(Notification::WindowPaneChanged { window_id, pane_id })
}

fn match_client_detached(line: &[u8]) -> Option<Notification> {
    // ^%client-detached (.+)$
    let s = lit(line, b"%client-detached ")?;
    let client = nonempty(s)?;
    Some(Notification::ClientDetached {
        client: client.to_vec(),
    })
}

fn match_client_session_changed(line: &[u8]) -> Option<Notification> {
    // ^%client-session-changed (.+) \$([0-9]+) (.+)$
    let s = lit(line, b"%client-session-changed ")?;
    // Greedy client: try the latest split first.
    for p in (1..s.len()).rev() {
        if let Some(r) = s[p..].strip_prefix(b" $")
            && let Some((session_id, r)) = digits(r)
            && let Some(r) = lit(r, b" ")
            && let Some(name) = nonempty(r)
        {
            return Some(Notification::ClientSessionChanged {
                client: s[..p].to_vec(),
                session_id,
                name: name.to_vec(),
            });
        }
    }
    None
}

/// Possible notification types from tmux control mode. These are documented
/// in tmux(1). A lot of the simple documentation was copied from that man
/// page here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    /// Entering tmux control mode. This isn't an actual event sent by
    /// tmux but is one sent by us to indicate that we have detected that
    /// tmux control mode is starting.
    Enter,

    /// Exit.
    ///
    /// NOTE: The tmux protocol contains a "reason" string (human friendly)
    /// associated with this. We currently drop it because we don't need it
    /// but this may be something we want to add later.
    Exit,

    /// Dispatched at the end of a begin/end block with the raw data.
    /// The control mode parser can't parse the data because it is unaware
    /// of the command that was sent to trigger this output.
    BlockEnd(Vec<u8>),
    BlockErr(Vec<u8>),

    /// Raw output from a pane.
    Output {
        pane_id: usize,
        /// unescaped
        data: Vec<u8>,
    },

    /// The client is now attached to the session with ID session-id, which is
    /// named name.
    SessionChanged {
        id: usize,
        name: Vec<u8>,
    },

    /// A session was created or destroyed.
    SessionsChanged,

    /// The layout of the window with ID window-id changed.
    LayoutChange {
        window_id: usize,
        layout: Vec<u8>,
        visible_layout: Vec<u8>,
        raw_flags: Vec<u8>,
    },

    /// The window with ID window-id was linked to the current session.
    WindowAdd {
        id: usize,
    },

    /// The window with ID window-id was renamed to name.
    WindowRenamed {
        id: usize,
        name: Vec<u8>,
    },

    /// The active pane in the window with ID window-id changed to the pane
    /// with ID pane-id.
    WindowPaneChanged {
        window_id: usize,
        pane_id: usize,
    },

    /// The client has detached.
    ClientDetached {
        client: Vec<u8>,
    },

    /// The client is now attached to the session with ID session-id, which is
    /// named name.
    ClientSessionChanged {
        client: Vec<u8>,
        session_id: usize,
        name: Vec<u8>,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn feed(c: &mut Parser, s: &str) {
        for b in s.bytes() {
            assert_eq!(c.put(b).unwrap(), None);
        }
    }

    #[test]
    fn tmux_begin_end_empty() {
        let mut c = Parser::new();
        feed(&mut c, "%begin 1578922740 269 1\n");
        feed(&mut c, "%end 1578922740 269 1");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(n, Notification::BlockEnd(vec![]));
    }

    #[test]
    fn tmux_begin_error_empty() {
        let mut c = Parser::new();
        feed(&mut c, "%begin 1578922740 269 1\n");
        feed(&mut c, "%error 1578922740 269 1");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(n, Notification::BlockErr(vec![]));
    }

    #[test]
    fn tmux_begin_end_data() {
        let mut c = Parser::new();
        feed(&mut c, "%begin 1578922740 269 1\n");
        feed(&mut c, "hello\nworld\n");
        feed(&mut c, "%end 1578922740 269 1");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(n, Notification::BlockEnd(b"hello\nworld".to_vec()));
    }

    #[test]
    fn tmux_output() {
        let mut c = Parser::new();
        feed(&mut c, "%output %42 foo bar baz");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(
            n,
            Notification::Output {
                pane_id: 42,
                data: b"foo bar baz".to_vec()
            }
        );
    }

    #[test]
    fn tmux_session_changed() {
        let mut c = Parser::new();
        feed(&mut c, "%session-changed $42 foo");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(
            n,
            Notification::SessionChanged {
                id: 42,
                name: b"foo".to_vec()
            }
        );
    }

    #[test]
    fn tmux_sessions_changed() {
        let mut c = Parser::new();
        feed(&mut c, "%sessions-changed");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(n, Notification::SessionsChanged);
    }

    #[test]
    fn tmux_sessions_changed_carriage_return() {
        let mut c = Parser::new();
        feed(&mut c, "%sessions-changed\r");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(n, Notification::SessionsChanged);
    }

    #[test]
    fn tmux_layout_change() {
        let mut c = Parser::new();
        feed(
            &mut c,
            "%layout-change @2 1234x791,0,0{617x791,0,0,0,617x791,618,0,1} 1234x791,0,0{617x791,0,0,0,617x791,618,0,1} *-",
        );
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(
            n,
            Notification::LayoutChange {
                window_id: 2,
                layout: b"1234x791,0,0{617x791,0,0,0,617x791,618,0,1}".to_vec(),
                visible_layout: b"1234x791,0,0{617x791,0,0,0,617x791,618,0,1}".to_vec(),
                raw_flags: b"*-".to_vec(),
            }
        );
    }

    #[test]
    fn tmux_layout_change_empty_flags() {
        let mut c = Parser::new();
        feed(&mut c, "%layout-change @2 a b ");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(
            n,
            Notification::LayoutChange {
                window_id: 2,
                layout: b"a".to_vec(),
                visible_layout: b"b".to_vec(),
                raw_flags: vec![],
            }
        );
    }

    #[test]
    fn tmux_window_add() {
        let mut c = Parser::new();
        feed(&mut c, "%window-add @14");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(n, Notification::WindowAdd { id: 14 });
    }

    #[test]
    fn tmux_window_renamed() {
        let mut c = Parser::new();
        feed(&mut c, "%window-renamed @42 bar");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(
            n,
            Notification::WindowRenamed {
                id: 42,
                name: b"bar".to_vec()
            }
        );
    }

    #[test]
    fn tmux_window_pane_changed() {
        let mut c = Parser::new();
        feed(&mut c, "%window-pane-changed @42 %2");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(
            n,
            Notification::WindowPaneChanged {
                window_id: 42,
                pane_id: 2
            }
        );
    }

    #[test]
    fn tmux_client_detached() {
        let mut c = Parser::new();
        feed(&mut c, "%client-detached /dev/pts/1");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(
            n,
            Notification::ClientDetached {
                client: b"/dev/pts/1".to_vec()
            }
        );
    }

    #[test]
    fn tmux_client_session_changed() {
        let mut c = Parser::new();
        feed(&mut c, "%client-session-changed /dev/pts/1 $2 mysession");
        let n = c.put(b'\n').unwrap().unwrap();
        assert_eq!(
            n,
            Notification::ClientSessionChanged {
                client: b"/dev/pts/1".to_vec(),
                session_id: 2,
                name: b"mysession".to_vec(),
            }
        );
    }

    #[test]
    fn tmux_idle_garbage_is_exit_then_broken() {
        let mut c = Parser::new();
        assert_eq!(c.put(b'x').unwrap(), Some(Notification::Exit));
        assert_eq!(c.put(b'%').unwrap(), None);
    }

    #[test]
    fn tmux_max_bytes() {
        let mut c = Parser::new();
        c.max_bytes = 4;
        for b in b"%out" {
            assert_eq!(c.put(*b).unwrap(), None);
        }
        assert_eq!(c.put(b'p'), Err(Error::OutOfMemory));
        assert_eq!(c.put(b'p').unwrap(), None);
    }
}
