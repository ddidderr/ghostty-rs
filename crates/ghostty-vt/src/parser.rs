//! VT-series parser for escape and control sequences
//! (ZIG/src/terminal/Parser.zig).
//!
//! This is implemented directly as the state machine described on
//! vt100.net: https://vt100.net/emu/dec_ansi_parser
//!
//! `MAX_PARAMS` and `SepList` were pre-created by the orchestrator because
//! `sgr` depends on them; keep their API stable.
//!
//! Rust port note: Zig's `Action` borrows slices out of the parser
//! (`csi_dispatch.params` etc., "only valid until the next call to next").
//! Here the CSI/ESC/DCS payloads are small fixed-size inline copies exposed
//! through `params()` / `intermediates()` accessors, so an `Action` owns its
//! data and is independent of the parser's lifetime.

use crate::osc;
use crate::parse_table::TABLE;

/// Maximum number of CSI parameters. This is a const in Ghostty so we
/// match it.
pub const MAX_PARAMS: usize = 24;

/// Bitset of length `MAX_PARAMS` marking which parameters were followed by
/// a colon (`:`) separator rather than a semicolon (`;`). Bit `i` set means
/// the separator *after* param `i` was a colon.
/// Equivalent to Zig's `std.StaticBitSet(MAX_PARAMS)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct SepList(u32);

impl SepList {
    /// An empty set (all separators are semicolons).
    pub const fn init_empty() -> Self {
        Self(0)
    }

    /// Whether bit `i` is set (separator after param `i` is a colon).
    pub fn is_set(&self, i: usize) -> bool {
        debug_assert!(i < MAX_PARAMS);
        self.0 & (1 << i) != 0
    }

    /// Set bit `i`.
    pub fn set(&mut self, i: usize) {
        debug_assert!(i < MAX_PARAMS);
        self.0 |= 1 << i;
    }

    /// Clear bit `i`.
    pub fn unset(&mut self, i: usize) {
        debug_assert!(i < MAX_PARAMS);
        self.0 &= !(1 << i);
    }

    /// Number of set bits.
    pub fn count(&self) -> usize {
        self.0.count_ones() as usize
    }

    /// Raw mask.
    pub fn mask(&self) -> u32 {
        self.0
    }
}

/// Maximum number of intermediate characters during parsing. This is
/// 4 because we also use the intermediates array for UTF8 decoding which
/// can be at most 4 bytes.
pub const MAX_INTERMEDIATE: usize = 4;

/// States for the state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiIntermediate,
    CsiParam,
    CsiIgnore,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsIgnore,
    OscString,
    SosPmApcString,
}

/// Transition action is an action that can be taken during a state
/// transition. This is more of an internal action, not one used by
/// end users, typically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransitionAction {
    None,
    Ignore,
    Print,
    Execute,
    Collect,
    Param,
    EscDispatch,
    CsiDispatch,
    Put,
    OscPut,
    ApcPut,
}

/// The separator used for CSI params.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sep {
    Semicolon = 0,
    Colon = 1,
}

/// CSI dispatch payload (Zig `Action.CSI`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Csi {
    intermediates_buf: [u8; MAX_INTERMEDIATE],
    intermediates_len: u8,
    params_buf: [u16; MAX_PARAMS],
    params_len: u8,
    /// The list of separators used for CSI params. The value of the
    /// bit can be mapped to Sep. The index of this bit set specifies
    /// the separator AFTER that param. For example: 0;4:3 would have
    /// index 1 set.
    pub params_sep: SepList,
    pub final_: u8,
}

impl Csi {
    pub fn intermediates(&self) -> &[u8] {
        &self.intermediates_buf[..self.intermediates_len as usize]
    }

    pub fn params(&self) -> &[u16] {
        &self.params_buf[..self.params_len as usize]
    }
}

impl std::fmt::Display for Csi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ESC [ {} {:?} {}",
            String::from_utf8_lossy(self.intermediates()),
            self.params(),
            self.final_ as char
        )
    }
}

/// ESC dispatch payload (Zig `Action.ESC`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Esc {
    intermediates_buf: [u8; MAX_INTERMEDIATE],
    intermediates_len: u8,
    pub final_: u8,
}

impl Esc {
    pub fn intermediates(&self) -> &[u8] {
        &self.intermediates_buf[..self.intermediates_len as usize]
    }
}

impl std::fmt::Display for Esc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ESC {} {}",
            String::from_utf8_lossy(self.intermediates()),
            self.final_ as char
        )
    }
}

/// DCS hook payload (Zig `Action.DCS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dcs {
    intermediates_buf: [u8; MAX_INTERMEDIATE],
    intermediates_len: u8,
    params_buf: [u16; MAX_PARAMS],
    params_len: u8,
    pub final_: u8,
}

impl Dcs {
    /// Build a DCS from slices (extra elements beyond the maximums are
    /// dropped). Useful for tests and for consumers constructing hooks.
    pub fn new(intermediates: &[u8], params: &[u16], final_: u8) -> Self {
        let mut d = Dcs {
            intermediates_buf: [0; MAX_INTERMEDIATE],
            intermediates_len: 0,
            params_buf: [0; MAX_PARAMS],
            params_len: 0,
            final_,
        };
        let n = intermediates.len().min(MAX_INTERMEDIATE);
        d.intermediates_buf[..n].copy_from_slice(&intermediates[..n]);
        d.intermediates_len = n as u8;
        let n = params.len().min(MAX_PARAMS);
        d.params_buf[..n].copy_from_slice(&params[..n]);
        d.params_len = n as u8;
        d
    }

    pub fn intermediates(&self) -> &[u8] {
        &self.intermediates_buf[..self.intermediates_len as usize]
    }

    pub fn params(&self) -> &[u16] {
        &self.params_buf[..self.params_len as usize]
    }
}

/// Action is the action that a caller of the parser is expected to
/// take as a result of some input character.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Draw character to the screen. This is a unicode codepoint.
    Print(u32),

    /// Execute the C0 or C1 function.
    Execute(u8),

    /// Execute the CSI command.
    CsiDispatch(Csi),

    /// Execute the ESC command.
    EscDispatch(Esc),

    /// Execute the OSC command.
    OscDispatch(osc::Command),

    /// DCS-related events.
    DcsHook(Dcs),
    DcsPut(u8),
    DcsUnhook,

    /// APC data
    ApcStart,
    ApcPut(u8),
    ApcEnd,
}

/// The VT parser state machine.
#[derive(Debug, Clone)]
pub struct Parser {
    /// Current state of the state machine
    pub state: State,

    /// Intermediate tracking.
    intermediates: [u8; MAX_INTERMEDIATE],
    intermediates_idx: u8,

    /// Param tracking, building
    params: [u16; MAX_PARAMS],
    params_sep: SepList,
    params_idx: u8,
    param_acc: u16,
    param_acc_idx: u8,

    /// Parser for OSC sequences
    pub osc_parser: osc::Parser,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Parser {
        Parser {
            state: State::Ground,
            intermediates: [0; MAX_INTERMEDIATE],
            intermediates_idx: 0,
            params: [0; MAX_PARAMS],
            params_sep: SepList::init_empty(),
            params_idx: 0,
            param_acc: 0,
            param_acc_idx: 0,
            osc_parser: osc::Parser::init(false),
        }
    }

    /// Next consumes the next character c and returns the actions to execute.
    /// Up to 3 actions may need to be executed -- in order -- representing
    /// the state exit, transition, and entry actions.
    pub fn next(&mut self, c: u8) -> [Option<Action>; 3] {
        let effect = TABLE[c as usize][self.state as usize];

        let next_state = effect.state;
        let action = effect.action;

        // When going from one state to another, the actions take place in this order:
        //
        // 1. exit action from old state
        // 2. transition action
        // 3. entry action to new state

        // Exit depends on current state
        let exit = if self.state == next_state {
            None
        } else {
            match self.state {
                State::OscString => self.osc_parser.end(Some(c)).map(Action::OscDispatch),
                State::DcsPassthrough => Some(Action::DcsUnhook),
                State::SosPmApcString => Some(Action::ApcEnd),
                _ => None,
            }
        };

        let transition = self.do_action(action, c);

        // Entry depends on new state
        let entry = if self.state == next_state {
            None
        } else {
            match next_state {
                State::Escape | State::DcsEntry | State::CsiEntry => {
                    self.clear();
                    None
                }
                State::OscString => {
                    self.osc_parser.reset();
                    None
                }
                State::DcsPassthrough => 'dcs_hook: {
                    // Ignore too many parameters
                    if self.params_idx as usize >= MAX_PARAMS {
                        break 'dcs_hook None;
                    }
                    // Finalize parameters
                    if self.param_acc_idx > 0 {
                        self.params[self.params_idx as usize] = self.param_acc;
                        self.params_idx += 1;
                    }
                    Some(Action::DcsHook(Dcs::new(
                        &self.intermediates[..self.intermediates_idx as usize],
                        &self.params[..self.params_idx as usize],
                        c,
                    )))
                }
                State::SosPmApcString => Some(Action::ApcStart),
                _ => None,
            }
        };

        // After generating the actions, we set our next state.
        self.state = next_state;

        [exit, transition, entry]
    }

    #[inline]
    pub fn collect(&mut self, c: u8) {
        if self.intermediates_idx as usize >= MAX_INTERMEDIATE {
            log::warn!("invalid intermediates count");
            return;
        }

        self.intermediates[self.intermediates_idx as usize] = c;
        self.intermediates_idx += 1;
    }

    #[inline]
    fn do_action(&mut self, action: TransitionAction, c: u8) -> Option<Action> {
        match action {
            TransitionAction::None | TransitionAction::Ignore => None,
            TransitionAction::Print => Some(Action::Print(c as u32)),
            TransitionAction::Execute => Some(Action::Execute(c)),
            TransitionAction::Collect => {
                self.collect(c);
                None
            }
            TransitionAction::Param => {
                // Semicolon separates parameters. If we encounter a semicolon
                // we need to store and move on to the next parameter.
                if c == b';' || c == b':' {
                    // Ignore too many parameters
                    if self.params_idx as usize >= MAX_PARAMS {
                        return None;
                    }

                    // Set param final value
                    self.params[self.params_idx as usize] = self.param_acc;
                    if c == b':' {
                        self.params_sep.set(self.params_idx as usize);
                    }
                    self.params_idx += 1;

                    // Reset current param value to 0
                    self.param_acc = 0;
                    self.param_acc_idx = 0;
                    return None;
                }

                // A numeric value. Add it to our accumulator.
                self.param_acc = self.param_acc.saturating_mul(10);
                self.param_acc = self.param_acc.saturating_add((c - b'0') as u16);

                // Increment our accumulator index. If we overflow then
                // we're out of bounds and we exit immediately.
                self.param_acc_idx = self.param_acc_idx.wrapping_add(1);

                // The client is expected to perform no action.
                None
            }
            TransitionAction::OscPut => {
                self.osc_parser.next(c);
                None
            }
            TransitionAction::CsiDispatch => {
                // Ignore too many parameters
                if self.params_idx as usize >= MAX_PARAMS {
                    return None;
                }

                // Finalize parameters if we have one
                if self.param_acc_idx > 0 {
                    self.params[self.params_idx as usize] = self.param_acc;
                    self.params_idx += 1;
                }

                let mut csi = Csi {
                    intermediates_buf: self.intermediates,
                    intermediates_len: self.intermediates_idx,
                    params_buf: [0; MAX_PARAMS],
                    params_len: self.params_idx,
                    params_sep: self.params_sep,
                    final_: c,
                };
                csi.params_buf[..self.params_idx as usize]
                    .copy_from_slice(&self.params[..self.params_idx as usize]);

                // We only allow colon or mixed separators for the 'm' command.
                if c != b'm' && self.params_sep.count() > 0 {
                    log::warn!(
                        "CSI colon or mixed separators only allowed for 'm' command, got: {csi}"
                    );
                    return None;
                }

                Some(Action::CsiDispatch(csi))
            }
            TransitionAction::EscDispatch => Some(Action::EscDispatch(Esc {
                intermediates_buf: self.intermediates,
                intermediates_len: self.intermediates_idx,
                final_: c,
            })),
            TransitionAction::Put => Some(Action::DcsPut(c)),
            TransitionAction::ApcPut => Some(Action::ApcPut(c)),
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.intermediates_idx = 0;
        self.params_idx = 0;
        self.params_sep = SepList::init_empty();
        self.param_acc = 0;
        self.param_acc_idx = 0;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::osc::Command;

    fn feed_quiet(p: &mut Parser, s: &[u8]) {
        for &c in s {
            let a = p.next(c);
            assert!(a[0].is_none());
            assert!(a[1].is_none());
            assert!(a[2].is_none());
        }
    }

    fn csi(a: &[Option<Action>; 3]) -> Csi {
        match a[1].as_ref().expect("csi_dispatch") {
            Action::CsiDispatch(c) => *c,
            other => panic!("expected csi_dispatch, got {other:?}"),
        }
    }

    #[test]
    fn parser_basic() {
        let mut p = Parser::new();
        p.next(0x9E);
        assert_eq!(p.state, State::SosPmApcString);
        p.next(0x9C);
        assert_eq!(p.state, State::Ground);

        {
            let a = p.next(b'a');
            assert_eq!(p.state, State::Ground);
            assert!(a[0].is_none());
            assert!(matches!(a[1], Some(Action::Print(_))));
            assert!(a[2].is_none());
        }

        {
            let a = p.next(0x19);
            assert_eq!(p.state, State::Ground);
            assert!(a[0].is_none());
            assert!(matches!(a[1], Some(Action::Execute(_))));
            assert!(a[2].is_none());
        }
    }

    #[test]
    fn esc_esc_paren_b() {
        let mut p = Parser::new();
        p.next(0x1B);
        p.next(b'(');

        let a = p.next(b'B');
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(a[2].is_none());
        let Some(Action::EscDispatch(d)) = &a[1] else {
            panic!("expected esc_dispatch");
        };
        assert_eq!(d.final_, b'B');
        assert_eq!(d.intermediates(), b"(");
    }

    #[test]
    fn csi_esc_bracket_h() {
        let mut p = Parser::new();
        p.next(0x1B);
        p.next(0x5B);

        let a = p.next(0x48);
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(a[2].is_none());
        let d = csi(&a);
        assert_eq!(d.final_, 0x48);
        assert_eq!(d.params().len(), 0);
    }

    #[test]
    fn csi_esc_bracket_1_semicolon_4_h() {
        let mut p = Parser::new();
        p.next(0x1B);
        p.next(0x5B);
        p.next(0x31); // 1
        p.next(0x3B); // ;
        p.next(0x34); // 4

        let a = p.next(0x48); // H
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(a[2].is_none());
        let d = csi(&a);
        assert_eq!(d.final_, b'H');
        assert_eq!(d.params(), &[1, 4]);
    }

    #[test]
    fn csi_sgr_esc_bracket_38_colon_2_m() {
        let mut p = Parser::new();
        p.next(0x1B);
        p.next(b'[');
        p.next(b'3');
        p.next(b'8');
        p.next(b':');
        p.next(b'2');

        let a = p.next(b'm');
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(a[2].is_none());
        let d = csi(&a);
        assert_eq!(d.final_, b'm');
        assert_eq!(d.params(), &[38, 2]);
        assert!(d.params_sep.is_set(0));
        assert!(!d.params_sep.is_set(1));
    }

    #[test]
    fn csi_sgr_colon_followed_by_semicolon() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(&mut p, b"[48:2");

        {
            let a = p.next(b'm');
            assert_eq!(p.state, State::Ground);
            assert!(a[0].is_none());
            assert!(matches!(a[1], Some(Action::CsiDispatch(_))));
            assert!(a[2].is_none());
        }

        p.next(0x1B);
        p.next(b'[');
        {
            let a = p.next(b'H');
            assert_eq!(p.state, State::Ground);
            assert!(a[0].is_none());
            assert!(matches!(a[1], Some(Action::CsiDispatch(_))));
            assert!(a[2].is_none());
        }
    }

    #[test]
    fn csi_sgr_mixed_colon_and_semicolon() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(&mut p, b"[38:5:1;48:5:0");

        let a = p.next(b'm');
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(matches!(a[1], Some(Action::CsiDispatch(_))));
        assert!(a[2].is_none());
    }

    #[test]
    fn csi_sgr_esc_bracket_48_colon_2_m() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(&mut p, b"[48:2:240:143:104");

        let a = p.next(b'm');
        assert_eq!(p.state, State::Ground);
        let d = csi(&a);
        assert_eq!(d.final_, b'm');
        assert_eq!(d.params(), &[48, 2, 240, 143, 104]);
        for (i, expect) in [true, true, true, true, false].iter().enumerate() {
            assert_eq!(d.params_sep.is_set(i), *expect, "sep {i}");
        }
    }

    #[test]
    fn csi_sgr_esc_bracket_4_colon_3_m_colon() {
        let mut p = Parser::new();
        p.next(0x1B);
        p.next(b'[');
        p.next(b'4');
        p.next(b':');
        p.next(b'3');

        let a = p.next(b'm');
        assert_eq!(p.state, State::Ground);
        let d = csi(&a);
        assert_eq!(d.final_, b'm');
        assert_eq!(d.params(), &[4, 3]);
        assert!(d.params_sep.is_set(0));
        assert!(!d.params_sep.is_set(1));
    }

    #[test]
    fn csi_sgr_with_many_blank_and_colon() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(&mut p, b"[58:2::240:143:104");

        let a = p.next(b'm');
        assert_eq!(p.state, State::Ground);
        let d = csi(&a);
        assert_eq!(d.final_, b'm');
        assert_eq!(d.params(), &[58, 2, 0, 240, 143, 104]);
        for (i, expect) in [true, true, true, true, true, false].iter().enumerate() {
            assert_eq!(d.params_sep.is_set(i), *expect, "sep {i}");
        }
    }

    // This is from a Kakoune actual SGR sequence.
    #[test]
    fn csi_sgr_mixed_colon_and_semicolon_with_blank() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(&mut p, b"[;4:3;38;2;175;175;215;58:2::190:80:70");

        let a = p.next(b'm');
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(a[2].is_none());
        let d = csi(&a);
        assert_eq!(d.final_, b'm');
        assert_eq!(
            d.params(),
            &[0, 4, 3, 38, 2, 175, 175, 215, 58, 2, 0, 190, 80, 70]
        );
        let set = [1usize, 8, 9, 10, 11, 12];
        for i in 0..14 {
            assert_eq!(d.params_sep.is_set(i), set.contains(&i), "sep {i}");
        }
    }

    // This is from a Kakoune actual SGR sequence also.
    #[test]
    fn csi_sgr_mixed_colon_and_semicolon_setting_underline_bg_fg() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(
            &mut p,
            b"[4:3;38;2;51;51;51;48;2;170;170;170;58;2;255;97;136",
        );

        let a = p.next(b'm');
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(a[2].is_none());
        let d = csi(&a);
        assert_eq!(d.final_, b'm');
        assert_eq!(
            d.params(),
            &[
                4, 3, 38, 2, 51, 51, 51, 48, 2, 170, 170, 170, 58, 2, 255, 97, 136
            ]
        );
        for i in 0..17 {
            assert_eq!(d.params_sep.is_set(i), i == 0, "sep {i}");
        }
    }

    #[test]
    fn csi_colon_for_non_m_final() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(&mut p, b"[38:2h");
        assert_eq!(p.state, State::Ground);
    }

    #[test]
    fn csi_request_mode_decrqm() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(&mut p, b"[?2026$");

        let a = p.next(b'p');
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(a[2].is_none());
        let d = csi(&a);
        assert_eq!(d.final_, b'p');
        assert_eq!(d.intermediates(), b"?$");
        assert_eq!(d.params(), &[2026]);
    }

    #[test]
    fn csi_change_cursor() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(&mut p, b"[3 ");

        let a = p.next(b'q');
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(a[2].is_none());
        let d = csi(&a);
        assert_eq!(d.final_, b'q');
        assert_eq!(d.intermediates(), b" ");
        assert_eq!(d.params(), &[3]);
    }

    #[test]
    fn osc_change_window_title() {
        let mut p = Parser::new();
        for c in [0x1B, b']', b'0', b';', b'a', b'b', b'c'] {
            p.next(c);
        }

        let a = p.next(0x07); // BEL
        assert_eq!(p.state, State::Ground);
        assert!(a[1].is_none());
        assert!(a[2].is_none());
        assert_eq!(
            a[0],
            Some(Action::OscDispatch(Command::ChangeWindowTitle(
                b"abc".to_vec()
            )))
        );
    }

    #[test]
    fn osc_change_window_title_end_in_esc() {
        let mut p = Parser::new();
        for c in [0x1B, b']', b'0', b';', b'a', b'b', b'c'] {
            p.next(c);
        }

        let a = p.next(0x1B);
        p.next(b'\\');
        assert_eq!(p.state, State::Ground);
        assert!(a[1].is_none());
        assert!(a[2].is_none());
        assert_eq!(
            a[0],
            Some(Action::OscDispatch(Command::ChangeWindowTitle(
                b"abc".to_vec()
            )))
        );
    }

    // https://github.com/darrenstarr/VtNetCore/pull/14
    // Saw this on HN, decided to add a test case because why not.
    #[test]
    fn osc_112_incomplete_sequence() {
        let mut p = Parser::new();
        p.osc_parser.alloc = true;

        for c in [0x1B, b']', b'1', b'1', b'2'] {
            p.next(c);
        }

        let a = p.next(0x07);
        assert_eq!(p.state, State::Ground);
        assert!(a[1].is_none());
        assert!(a[2].is_none());
        let Some(Action::OscDispatch(Command::ColorOperation {
            op,
            requests,
            terminator,
        })) = a[0].clone()
        else {
            panic!("expected color_operation");
        };
        assert_eq!(terminator, osc::Terminator::Bel);
        assert_eq!(op, osc::color::Operation::Osc112);
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0],
            osc::color::Request::Reset(osc::color::Target::Dynamic(crate::color::Dynamic::Cursor))
        );
    }

    #[test]
    fn osc_104_empty() {
        let mut p = Parser::new();
        p.osc_parser.alloc = true;

        for c in [0x1B, b']', b'1', b'0', b'4'] {
            p.next(c);
        }

        let a = p.next(0x07);
        assert_eq!(p.state, State::Ground);
        assert!(a[1].is_none());
        assert!(a[2].is_none());
        let Some(Action::OscDispatch(Command::ColorOperation {
            op,
            requests,
            terminator,
        })) = a[0].clone()
        else {
            panic!("expected color_operation");
        };
        assert_eq!(terminator, osc::Terminator::Bel);
        assert_eq!(op, osc::color::Operation::Osc104);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0], osc::color::Request::ResetPalette);
    }

    #[test]
    fn csi_too_many_params() {
        let mut p = Parser::new();
        p.next(0x1B);
        p.next(b'[');
        for _ in 0..100 {
            p.next(b'1');
            p.next(b';');
        }
        p.next(b'1');

        let a = p.next(b'C');
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(a[1].is_none());
        assert!(a[2].is_none());
    }

    #[test]
    fn csi_sgr_with_up_to_our_max_parameters() {
        for max in 1..=MAX_PARAMS {
            let mut p = Parser::new();
            p.next(0x1B);
            p.next(b'[');

            for _ in 0..max - 1 {
                p.next(b'1');
                p.next(b';');
            }
            p.next(b'2');

            let a = p.next(b'H');
            assert_eq!(p.state, State::Ground);
            assert!(a[0].is_none());
            assert!(a[2].is_none());
            let c = csi(&a);
            assert_eq!(c.params().len(), max);
            assert_eq!(c.params()[max - 1], 2);
        }
    }

    #[test]
    fn csi_sgr_beyond_our_max_drops_it() {
        // Has to be +2 for the loops below
        let max = MAX_PARAMS + 2;

        let mut p = Parser::new();
        p.next(0x1B);
        p.next(b'[');

        for _ in 0..max - 1 {
            p.next(b'1');
            p.next(b';');
        }
        p.next(b'2');

        let a = p.next(b'H');
        assert_eq!(p.state, State::Ground);
        assert!(a[0].is_none());
        assert!(a[1].is_none());
        assert!(a[2].is_none());
    }

    #[test]
    fn dcs_xtgettcap() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(&mut p, b"P+");

        let a = p.next(b'q');
        assert_eq!(p.state, State::DcsPassthrough);
        assert!(a[0].is_none());
        assert!(a[1].is_none());
        let Some(Action::DcsHook(hook)) = &a[2] else {
            panic!("expected dcs_hook");
        };
        assert_eq!(hook.intermediates(), b"+");
        assert_eq!(hook.params(), &[] as &[u16]);
        assert_eq!(hook.final_, b'q');
    }

    #[test]
    fn dcs_params() {
        let mut p = Parser::new();
        p.next(0x1B);
        feed_quiet(&mut p, b"P1000");

        let a = p.next(b'p');
        assert_eq!(p.state, State::DcsPassthrough);
        assert!(a[0].is_none());
        assert!(a[1].is_none());
        let Some(Action::DcsHook(hook)) = &a[2] else {
            panic!("expected dcs_hook");
        };
        assert_eq!(hook.params(), &[1000]);
        assert_eq!(hook.final_, b'p');
    }

    #[test]
    fn dcs_too_many_params() {
        // Regression test for a crash found by fuzzing (afl). When a DCS
        // sequence has more than MAX_PARAMS parameters and param_acc_idx > 0,
        // entering dcs_passthrough wrote to params[params_idx] without a
        // bounds check, causing an out-of-bounds access.
        let mut p = Parser::new();
        p.next(0x1B); // ESC
        p.next(b'P'); // DCS entry

        // Feed a digit then MAX_PARAMS semicolons to fill all param slots.
        p.next(b'6');
        for _ in 0..MAX_PARAMS {
            p.next(b';');
        }
        // Feed another digit so param_acc_idx > 0 while params_idx == MAX_PARAMS.
        p.next(b'7');

        // A final byte triggers entry to dcs_passthrough. The DCS should
        // be dropped entirely, consistent with how CSI handles overflow.
        let a = p.next(b'p');
        assert!(a[0].is_none());
        assert!(a[1].is_none());
        assert!(a[2].is_none());
    }
}
