//! This structure caches the shaped cells for a given text run.
//!
//! At one point, shaping was the most expensive part of rendering text
//! (accounting for 96% of frame time on my machine). To speed it up, this
//! was introduced so that shaping results can be cached depending on the
//! run.
//!
//! The cache key is the text run. The text run builds its own hash value
//! based on the font, style, codepoint, etc. This just utilizes the hash that
//! the text run provides.

use ghostty_datastruct::cache_table::{CacheContext, CacheTable};

use crate::shape::{Cell, TextRun};

/// Context for cache table.
#[derive(Debug, Default)]
pub struct CellCacheTableContext;

impl CacheContext<u64, Vec<Cell>> for CellCacheTableContext {
    fn hash(&mut self, key: &u64) -> u64 {
        *key
    }
    fn eql(&mut self, a: &u64, b: &u64) -> bool {
        a == b
    }
}

/// Cache table for run hash -> shaped cells.
///
/// Capacity is slightly arbitrary. These numbers are guesses: I'd expect an
/// average of 256 frequently cached runs is a safe guess for most terminal
/// screens, with 8 items per bucket to give decent resiliency to important
/// runs.
type CellCacheTable = CacheTable<u64, Vec<Cell>, CellCacheTableContext, 256, 8>;

/// The cache table of shaped cells.
#[derive(Debug)]
pub struct Cache {
    map: CellCacheTable,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    pub fn new() -> Cache {
        Cache {
            map: CellCacheTable::new(CellCacheTableContext),
        }
    }

    /// Get the shaped cells for the given text run,
    /// or None if they are not in the cache.
    pub fn get(&mut self, run: &TextRun) -> Option<&[Cell]> {
        self.map.get(&run.hash).map(Vec::as_slice)
    }

    /// Insert the shaped cells for the given text run into the cache.
    ///
    /// The cells will be duplicated.
    pub fn put(&mut self, run: &TextRun, cells: &[Cell]) {
        // An evicted entry is simply dropped.
        let _evicted = self.map.put(run.hash, cells.to_vec());
    }

    /// Remove everything from the cache.
    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::Index;

    #[test]
    fn cache() {
        let mut c = Cache::new();

        let run = TextRun {
            hash: 1,
            offset: 0,
            cells: 0,
            font_index: Index::default(),
        };
        assert!(c.get(&run).is_none());
        c.put(
            &run,
            &[
                Cell {
                    x: 0,
                    glyph_index: 0,
                    ..Cell::default()
                },
                Cell {
                    x: 1,
                    glyph_index: 1,
                    ..Cell::default()
                },
            ],
        );

        let actual = c.get(&run).unwrap();
        assert_eq!(actual.len(), 2);
    }
}
