//! Windows stand-in for `signal.rs`: Zig's `crash/sentry.zig` does not
//! initialise crash capture on Windows, so there are no fatal-signal
//! handlers. Only the helpers the report writer shares are provided.

use std::hash::{BuildHasher, Hasher, RandomState};

/// Fill `out` with random bytes (used for the event UUID). The standard
/// library's per-instance random hasher keys are good enough for an id.
pub(crate) fn random_bytes(out: &mut [u8; 16]) {
    for chunk in out.chunks_mut(8) {
        let mut h = RandomState::new().build_hasher();
        h.write_usize(chunk.len());
        chunk.copy_from_slice(&h.finish().to_ne_bytes()[..chunk.len()]);
    }
}

/// No fatal-signal handlers on Windows.
pub(crate) fn install_handlers() {}
