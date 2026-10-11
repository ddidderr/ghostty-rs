//! The primary export of this file is `TABLE`, which contains a
//! compile-time-generated state transition table for VT emulation
//! (ZIG/src/terminal/parse_table.zig).
//!
//! This is based on the vt100.net state machine:
//! https://vt100.net/emu/dec_ansi_parser
//! But has some modifications:
//!
//!   * csi_param accepts the colon character (':') since the SGR command
//!     accepts colon as a valid parameter value.

use crate::parser::{State, TransitionAction as A};

const NUM_STATES: usize = 14;

const STATES: [State; NUM_STATES] = [
    State::Ground,
    State::Escape,
    State::EscapeIntermediate,
    State::CsiEntry,
    State::CsiIntermediate,
    State::CsiParam,
    State::CsiIgnore,
    State::DcsEntry,
    State::DcsParam,
    State::DcsIntermediate,
    State::DcsPassthrough,
    State::DcsIgnore,
    State::OscString,
    State::SosPmApcString,
];

/// Transition is the transition to take within the table
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub state: State,
    pub action: A,
}

/// The type of the state transition table: `[byte][state]`.
pub type Table = [[Transition; NUM_STATES]; 256];

type OptionalTable = [[Option<Transition>; NUM_STATES]; 256];

/// The state transition table, indexed `TABLE[byte][state as usize]`.
pub const TABLE: Table = gen_table();

/// Function to generate the full state transition table for VT emulation.
const fn gen_table() -> Table {
    // We accumulate using an "optional" table so we can detect unset entries.
    let mut result: OptionalTable = [[None; NUM_STATES]; 256];

    // anywhere transitions
    let mut s = 0;
    while s < NUM_STATES {
        let source = STATES[s];

        // anywhere => ground
        single(&mut result, 0x18, source, State::Ground, A::Execute);
        single(&mut result, 0x1A, source, State::Ground, A::Execute);
        range(&mut result, 0x80, 0x8F, source, State::Ground, A::Execute);
        range(&mut result, 0x91, 0x97, source, State::Ground, A::Execute);
        single(&mut result, 0x99, source, State::Ground, A::Execute);
        single(&mut result, 0x9A, source, State::Ground, A::Execute);
        single(&mut result, 0x9C, source, State::Ground, A::None);

        // anywhere => escape
        single(&mut result, 0x1B, source, State::Escape, A::None);

        // anywhere => sos_pm_apc_string
        single(&mut result, 0x98, source, State::SosPmApcString, A::None);
        single(&mut result, 0x9E, source, State::SosPmApcString, A::None);
        single(&mut result, 0x9F, source, State::SosPmApcString, A::None);

        // anywhere => csi_entry
        single(&mut result, 0x9B, source, State::CsiEntry, A::None);

        // anywhere => dcs_entry
        single(&mut result, 0x90, source, State::DcsEntry, A::None);

        // anywhere => osc_string
        single(&mut result, 0x9D, source, State::OscString, A::None);
        s += 1;
    }
    // ground
    {
        // events
        single(&mut result, 0x19, State::Ground, State::Ground, A::Execute);
        range(
            &mut result,
            0,
            0x17,
            State::Ground,
            State::Ground,
            A::Execute,
        );
        range(
            &mut result,
            0x1C,
            0x1F,
            State::Ground,
            State::Ground,
            A::Execute,
        );
        range(
            &mut result,
            0x20,
            0x7F,
            State::Ground,
            State::Ground,
            A::Print,
        );
    }

    // escape_intermediate
    {
        let source = State::EscapeIntermediate;

        single(&mut result, 0x19, source, source, A::Execute);
        range(&mut result, 0, 0x17, source, source, A::Execute);
        range(&mut result, 0x1C, 0x1F, source, source, A::Execute);
        range(&mut result, 0x20, 0x2F, source, source, A::Collect);
        single(&mut result, 0x7F, source, source, A::Ignore);

        // => ground
        range(
            &mut result,
            0x30,
            0x7E,
            source,
            State::Ground,
            A::EscDispatch,
        );
    }

    // sos_pm_apc_string
    {
        let source = State::SosPmApcString;

        // events
        single(&mut result, 0x19, source, source, A::ApcPut);
        range(&mut result, 0, 0x17, source, source, A::ApcPut);
        range(&mut result, 0x1C, 0x1F, source, source, A::ApcPut);
        range(&mut result, 0x20, 0x7F, source, source, A::ApcPut);
    }

    // escape
    {
        let source = State::Escape;

        // events
        single(&mut result, 0x19, source, source, A::Execute);
        range(&mut result, 0, 0x17, source, source, A::Execute);
        range(&mut result, 0x1C, 0x1F, source, source, A::Execute);
        single(&mut result, 0x7F, source, source, A::Ignore);

        // => ground
        range(
            &mut result,
            0x30,
            0x4F,
            source,
            State::Ground,
            A::EscDispatch,
        );
        range(
            &mut result,
            0x51,
            0x57,
            source,
            State::Ground,
            A::EscDispatch,
        );
        range(
            &mut result,
            0x60,
            0x7E,
            source,
            State::Ground,
            A::EscDispatch,
        );
        single(&mut result, 0x59, source, State::Ground, A::EscDispatch);
        single(&mut result, 0x5A, source, State::Ground, A::EscDispatch);
        single(&mut result, 0x5C, source, State::Ground, A::EscDispatch);

        // => escape_intermediate
        range(
            &mut result,
            0x20,
            0x2F,
            source,
            State::EscapeIntermediate,
            A::Collect,
        );

        // => sos_pm_apc_string
        single(&mut result, 0x58, source, State::SosPmApcString, A::None);
        single(&mut result, 0x5E, source, State::SosPmApcString, A::None);
        single(&mut result, 0x5F, source, State::SosPmApcString, A::None);

        // => dcs_entry
        single(&mut result, 0x50, source, State::DcsEntry, A::None);

        // => csi_entry
        single(&mut result, 0x5B, source, State::CsiEntry, A::None);

        // => osc_string
        single(&mut result, 0x5D, source, State::OscString, A::None);
    }

    // dcs_entry
    {
        let source = State::DcsEntry;

        // events
        single(&mut result, 0x19, source, source, A::Ignore);
        range(&mut result, 0, 0x17, source, source, A::Ignore);
        range(&mut result, 0x1C, 0x1F, source, source, A::Ignore);
        single(&mut result, 0x7F, source, source, A::Ignore);

        // => dcs_intermediate
        range(
            &mut result,
            0x20,
            0x2F,
            source,
            State::DcsIntermediate,
            A::Collect,
        );

        // => dcs_ignore
        single(&mut result, 0x3A, source, State::DcsIgnore, A::None);

        // => dcs_param
        range(&mut result, 0x30, 0x39, source, State::DcsParam, A::Param);
        single(&mut result, 0x3B, source, State::DcsParam, A::Param);
        range(&mut result, 0x3C, 0x3F, source, State::DcsParam, A::Collect);

        // => dcs_passthrough
        range(
            &mut result,
            0x40,
            0x7E,
            source,
            State::DcsPassthrough,
            A::None,
        );
    }

    // dcs_intermediate
    {
        let source = State::DcsIntermediate;

        // events
        single(&mut result, 0x19, source, source, A::Ignore);
        range(&mut result, 0, 0x17, source, source, A::Ignore);
        range(&mut result, 0x1C, 0x1F, source, source, A::Ignore);
        range(&mut result, 0x20, 0x2F, source, source, A::Collect);
        single(&mut result, 0x7F, source, source, A::Ignore);

        // => dcs_ignore
        range(&mut result, 0x30, 0x3F, source, State::DcsIgnore, A::None);

        // => dcs_passthrough
        range(
            &mut result,
            0x40,
            0x7E,
            source,
            State::DcsPassthrough,
            A::None,
        );
    }

    // dcs_ignore
    {
        let source = State::DcsIgnore;

        // events
        single(&mut result, 0x19, source, source, A::Ignore);
        range(&mut result, 0, 0x17, source, source, A::Ignore);
        range(&mut result, 0x1C, 0x1F, source, source, A::Ignore);
    }

    // dcs_param
    {
        let source = State::DcsParam;

        // events
        single(&mut result, 0x19, source, source, A::Ignore);
        range(&mut result, 0, 0x17, source, source, A::Ignore);
        range(&mut result, 0x1C, 0x1F, source, source, A::Ignore);
        range(&mut result, 0x30, 0x39, source, source, A::Param);
        single(&mut result, 0x3B, source, source, A::Param);
        single(&mut result, 0x7F, source, source, A::Ignore);

        // => dcs_ignore
        single(&mut result, 0x3A, source, State::DcsIgnore, A::None);
        range(&mut result, 0x3C, 0x3F, source, State::DcsIgnore, A::None);

        // => dcs_intermediate
        range(
            &mut result,
            0x20,
            0x2F,
            source,
            State::DcsIntermediate,
            A::Collect,
        );

        // => dcs_passthrough
        range(
            &mut result,
            0x40,
            0x7E,
            source,
            State::DcsPassthrough,
            A::None,
        );
    }

    // dcs_passthrough
    {
        let source = State::DcsPassthrough;

        // events
        single(&mut result, 0x19, source, source, A::Put);
        range(&mut result, 0, 0x17, source, source, A::Put);
        range(&mut result, 0x1C, 0x1F, source, source, A::Put);
        range(&mut result, 0x20, 0x7E, source, source, A::Put);
        single(&mut result, 0x7F, source, source, A::Ignore);
    }

    // csi_param
    {
        let source = State::CsiParam;

        // events
        single(&mut result, 0x19, source, source, A::Execute);
        range(&mut result, 0, 0x17, source, source, A::Execute);
        range(&mut result, 0x1C, 0x1F, source, source, A::Execute);
        range(&mut result, 0x30, 0x39, source, source, A::Param);
        single(&mut result, 0x3A, source, source, A::Param);
        single(&mut result, 0x3B, source, source, A::Param);
        single(&mut result, 0x7F, source, source, A::Ignore);

        // => ground
        range(
            &mut result,
            0x40,
            0x7E,
            source,
            State::Ground,
            A::CsiDispatch,
        );

        // => csi_ignore
        range(&mut result, 0x3C, 0x3F, source, State::CsiIgnore, A::None);

        // => csi_intermediate
        range(
            &mut result,
            0x20,
            0x2F,
            source,
            State::CsiIntermediate,
            A::Collect,
        );
    }

    // csi_ignore
    {
        let source = State::CsiIgnore;

        // events
        single(&mut result, 0x19, source, source, A::Execute);
        range(&mut result, 0, 0x17, source, source, A::Execute);
        range(&mut result, 0x1C, 0x1F, source, source, A::Execute);
        range(&mut result, 0x20, 0x3F, source, source, A::Ignore);
        single(&mut result, 0x7F, source, source, A::Ignore);

        // => ground
        range(&mut result, 0x40, 0x7E, source, State::Ground, A::None);
    }

    // csi_intermediate
    {
        let source = State::CsiIntermediate;

        // events
        single(&mut result, 0x19, source, source, A::Execute);
        range(&mut result, 0, 0x17, source, source, A::Execute);
        range(&mut result, 0x1C, 0x1F, source, source, A::Execute);
        range(&mut result, 0x20, 0x2F, source, source, A::Collect);
        single(&mut result, 0x7F, source, source, A::Ignore);

        // => ground
        range(
            &mut result,
            0x40,
            0x7E,
            source,
            State::Ground,
            A::CsiDispatch,
        );

        // => csi_ignore
        range(&mut result, 0x30, 0x3F, source, State::CsiIgnore, A::None);
    }

    // csi_entry
    {
        let source = State::CsiEntry;

        // events
        single(&mut result, 0x19, source, source, A::Execute);
        range(&mut result, 0, 0x17, source, source, A::Execute);
        range(&mut result, 0x1C, 0x1F, source, source, A::Execute);
        single(&mut result, 0x7F, source, source, A::Ignore);

        // => ground
        range(
            &mut result,
            0x40,
            0x7E,
            source,
            State::Ground,
            A::CsiDispatch,
        );

        // => csi_ignore
        single(&mut result, 0x3A, source, State::CsiIgnore, A::None);

        // => csi_intermediate
        range(
            &mut result,
            0x20,
            0x2F,
            source,
            State::CsiIntermediate,
            A::Collect,
        );

        // => csi_param
        range(&mut result, 0x30, 0x39, source, State::CsiParam, A::Param);
        single(&mut result, 0x3B, source, State::CsiParam, A::Param);
        range(&mut result, 0x3C, 0x3F, source, State::CsiParam, A::Collect);
    }

    // osc_string
    {
        let source = State::OscString;

        // events
        single(&mut result, 0x19, source, source, A::Ignore);
        range(&mut result, 0, 0x06, source, source, A::Ignore);
        range(&mut result, 0x08, 0x17, source, source, A::Ignore);
        range(&mut result, 0x1C, 0x1F, source, source, A::Ignore);
        range(&mut result, 0x20, 0xFF, source, source, A::OscPut);

        // XTerm accepts either BEL  or ST  for terminating OSC
        // sequences, and when returning information, uses the same
        // terminator used in a query.
        single(&mut result, 0x07, source, State::Ground, A::None);
    }

    // Create our immutable version
    let mut fin: Table = [[Transition {
        state: State::Ground,
        action: A::None,
    }; NUM_STATES]; 256];
    let mut i = 0;
    while i < 256 {
        let mut j = 0;
        while j < NUM_STATES {
            fin[i][j] = match result[i][j] {
                Some(t) => t,
                None => transition(STATES[j], A::None),
            };
            j += 1;
        }
        i += 1;
    }

    fin
}

const fn single(t: &mut OptionalTable, c: u8, s0: State, s1: State, a: A) {
    t[c as usize][s0 as usize] = Some(transition(s1, a));
}

const fn range(t: &mut OptionalTable, from: u8, to: u8, s0: State, s1: State, a: A) {
    let mut i = from;
    loop {
        single(t, i, s0, s1, a);
        // If 'to' is 0xFF, our next pass would overflow.
        if i == to {
            break;
        }
        i += 1;
    }
}

const fn transition(state: State, action: A) -> Transition {
    Transition { state, action }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn table_builds() {
        // This forces evaluation of the table, so we're just testing
        // that it succeeds in creation.
        let _ = &TABLE;
        assert_eq!(
            TABLE[b'a' as usize][State::Ground as usize],
            transition(State::Ground, A::Print)
        );
        assert_eq!(
            TABLE[0x1B][State::CsiParam as usize],
            transition(State::Escape, A::None)
        );
    }
}
