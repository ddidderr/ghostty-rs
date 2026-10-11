//! Three-level lookup tables for Unicode properties.
//!
//! This whole file is based on the algorithm described here:
//! <https://here-be-braces.com/fast-lookup-of-unicode-properties/>
//!
//! NOTE: this file is also `#[path]`-included by `build.rs`, so it must not
//! depend on anything else in the crate.
#![allow(dead_code)]
#![allow(
    clippy::unwrap_used,
    clippy::single_match_else,
    clippy::checked_conversions,
    clippy::trivially_copy_pass_by_ref,
    clippy::expect_used
)]

use std::collections::HashMap;
use std::fmt::Write as _;

const BLOCK_SIZE: usize = 256;

/// Number of codepoints covered by the tables (`maxInt(u21) + 1`).
pub const CODEPOINT_COUNT: usize = 0x20_0000;

/// Errors from table generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerateError {
    BlockTooLarge,
    Stage2TooLarge,
}

/// Owned lookup tables, the output of [`generate`].
#[derive(Debug, Clone)]
pub struct OwnedTables<E> {
    pub stage1: Vec<u16>,
    pub stage2: Vec<u16>,
    pub stage3: Vec<E>,
}

/// Generate the three-level lookup table for all `u21` codepoints.
/// `get` returns the mapping for a codepoint; mappings are compared with `==`
/// (the Zig `Context.eql`).
pub fn generate<E: Clone + PartialEq>(
    mut get: impl FnMut(u32) -> E,
) -> Result<OwnedTables<E>, GenerateError> {
    let mut blocks_map: HashMap<Vec<u16>, u16> = HashMap::new();
    let mut stage1: Vec<u16> = Vec::new();
    let mut stage2: Vec<u16> = Vec::new();
    let mut stage3: Vec<E> = Vec::new();

    let mut block: Vec<u16> = Vec::with_capacity(BLOCK_SIZE);
    for cp in 0..CODEPOINT_COUNT {
        // Get our block value and find the matching result value in our list
        // of possible values in stage3. This way, each possible mapping only
        // gets one entry in stage3.
        let elem = get(cp as u32);
        let idx = match stage3.iter().position(|item| *item == elem) {
            Some(i) => i,
            None => {
                stage3.push(elem);
                stage3.len() - 1
            }
        };
        block.push(u16::try_from(idx).map_err(|_| GenerateError::BlockTooLarge)?);

        if block.len() < BLOCK_SIZE && cp != CODEPOINT_COUNT - 1 {
            continue;
        }
        block.resize(BLOCK_SIZE, 0);

        let next = stage2.len();
        let s2 = match blocks_map.get(&block) {
            Some(&s) => s,
            None => {
                let s = u16::try_from(next).map_err(|_| GenerateError::Stage2TooLarge)?;
                stage2.extend_from_slice(&block);
                blocks_map.insert(block.clone(), s);
                s
            }
        };
        stage1.push(s2);
        block.clear();
    }

    assert!(stage1.len() <= usize::from(u16::MAX));
    assert!(stage2.len() <= usize::from(u16::MAX));
    assert!(stage3.len() <= usize::from(u16::MAX));
    Ok(OwnedTables {
        stage1,
        stage2,
        stage3,
    })
}

impl<E> OwnedTables<E> {
    /// Borrow as lookup [`Tables`].
    #[must_use]
    pub fn as_tables(&self) -> Tables<'_, E> {
        Tables {
            stage1: &self.stage1,
            stage2: &self.stage2,
            stage3: &self.stage3,
        }
    }

    /// Writes the lookup table as Rust source. The output defines three
    /// statics (`STAGE1`, `STAGE2`, `STAGE3`) that can be used to rebuild the
    /// lookup table.
    pub fn write_rust(&self, elem_ty: &str, mut fmt_elem: impl FnMut(&E) -> String) -> String {
        let mut out = String::new();
        out.push_str("// This file is auto-generated. Do not edit.\n\n");
        let _ = write!(out, "pub static STAGE1: [u16; {}] = [", self.stage1.len());
        for e in &self.stage1 {
            let _ = write!(out, "{e},");
        }
        out.push_str("];\n\n");
        let _ = write!(out, "pub static STAGE2: [u16; {}] = [", self.stage2.len());
        for e in &self.stage2 {
            let _ = write!(out, "{e},");
        }
        out.push_str("];\n\n");
        let _ = write!(
            out,
            "pub static STAGE3: [{elem_ty}; {}] = [",
            self.stage3.len()
        );
        for e in &self.stage3 {
            let _ = write!(out, "{},", fmt_elem(e));
        }
        out.push_str("];\n");
        out
    }
}

/// A 3-level lookup table from a Unicode codepoint to a mapping of type `E`.
#[derive(Debug, Clone, Copy)]
pub struct Tables<'a, E> {
    pub stage1: &'a [u16],
    pub stage2: &'a [u16],
    pub stage3: &'a [E],
}

impl<E: Copy> Tables<'_, E> {
    /// Given a codepoint (at most `0x1F_FFFF`), returns the mapping for it.
    #[inline]
    #[must_use]
    pub fn get(&self, cp: u32) -> E {
        let high = (cp >> 8) as usize;
        let low = (cp & 0xFF) as usize;
        self.stage3[self.stage2[self.stage1[high] as usize + low] as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_roundtrip() {
        let f = |cp: u32| -> u8 {
            if cp < 0x80 {
                0
            } else if (0x1000..0x1100).contains(&cp) {
                1
            } else if cp == 0x1F600 {
                2
            } else {
                3
            }
        };
        let t = generate(f).unwrap();
        let tables = t.as_tables();
        for cp in 0..CODEPOINT_COUNT as u32 {
            assert_eq!(tables.get(cp), f(cp), "cp={cp:x}");
        }
        // Blocks are deduplicated.
        assert!(t.stage2.len() < 10 * BLOCK_SIZE);
        assert_eq!(t.stage1.len(), CODEPOINT_COUNT / BLOCK_SIZE);
    }

    #[test]
    fn write_rust_output() {
        let t = generate(|cp| cp == 0x41).unwrap();
        let src = t.write_rust("bool", ToString::to_string);
        assert!(src.contains("pub static STAGE3: [bool; 2] = [false,true,];"));
    }
}
