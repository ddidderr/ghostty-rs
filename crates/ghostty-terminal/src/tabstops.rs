//! Keep track of the location of tabstops.
//!
//! This is implemented as a bit set. There is a preallocation segment that
//! is used for almost all screen sizes. Then there is a dynamically allocated
//! segment if the screen is larger than the preallocation amount.
//!
//! In reality, tabstops don't need to be the most performant in any metric.
//! This implementation tries to balance denser memory usage (by using a bitset)
//! and minimizing unnecessary allocations.
use std::collections::TryReserveError;

/// Unit is the type we use per tabstop unit (see file docs).
type Unit = u8;
const UNIT_BITS: usize = Unit::BITS as usize;

/// The number of columns we preallocate for. This is kind of high which
/// costs us some memory, but this is more columns than my 6k monitor at
/// 12-point font size, so this should prevent allocation in almost all
/// real world scenarios for the price of wasting at most
/// (columns / sizeOf(Unit)) bytes.
const PREALLOC_COLUMNS: usize = 512;

/// The number of entries we need for our preallocation.
const PREALLOC_COUNT: usize = PREALLOC_COLUMNS / UNIT_BITS;

/// We precompute all the possible masks since we never use a huge bit size.
const MASKS: [Unit; UNIT_BITS] = {
    let mut res = [0; UNIT_BITS];
    let mut i = 0;
    while i < UNIT_BITS {
        res[i] = 1 << i;
        i += 1;
    }
    res
};

/// Returns the entry in the stops array that would contain this column.
#[inline]
const fn entry(col: usize) -> usize {
    col / UNIT_BITS
}

#[inline]
const fn index(col: usize) -> usize {
    col % UNIT_BITS
}

/// Tabstop bit set.
#[derive(Debug, Clone)]
pub struct Tabstops {
    /// The number of columns this tabstop is set to manage. Use `resize`
    /// to change this number.
    pub cols: usize,

    /// Preallocated tab stops.
    prealloc_stops: [Unit; PREALLOC_COUNT],

    /// Dynamically expanded stops above prealloc stops.
    dynamic_stops: Vec<Unit>,
}

impl Default for Tabstops {
    fn default() -> Self {
        Self {
            cols: 0,
            prealloc_stops: [0; PREALLOC_COUNT],
            dynamic_stops: Vec::new(),
        }
    }
}

impl Tabstops {
    pub fn new(cols: usize, interval: usize) -> Result<Self, TryReserveError> {
        let mut res = Self::default();
        res.resize(cols)?;
        res.reset(interval);
        Ok(res)
    }

    /// Set the tabstop at a certain column. The columns are 0-indexed.
    pub fn set(&mut self, col: usize) {
        let i = entry(col);
        let idx = index(col);
        if i < PREALLOC_COUNT {
            self.prealloc_stops[i] |= MASKS[idx];
            return;
        }

        let dynamic_i = i - PREALLOC_COUNT;
        assert!(dynamic_i < self.dynamic_stops.len());
        self.dynamic_stops[dynamic_i] |= MASKS[idx];
    }

    /// Unset the tabstop at a certain column. The columns are 0-indexed.
    ///
    /// Note: like the Zig implementation this XORs the bit, so unsetting a
    /// tabstop that is not set will set it.
    pub fn unset(&mut self, col: usize) {
        let i = entry(col);
        let idx = index(col);
        if i < PREALLOC_COUNT {
            self.prealloc_stops[i] ^= MASKS[idx];
            return;
        }

        let dynamic_i = i - PREALLOC_COUNT;
        assert!(dynamic_i < self.dynamic_stops.len());
        self.dynamic_stops[dynamic_i] ^= MASKS[idx];
    }

    /// Get the value of a tabstop at a specific column. The columns are 0-indexed.
    pub fn get(&self, col: usize) -> bool {
        let i = entry(col);
        let idx = index(col);
        let mask = MASKS[idx];
        let unit = if i < PREALLOC_COUNT {
            self.prealloc_stops[i]
        } else {
            let dynamic_i = i - PREALLOC_COUNT;
            assert!(dynamic_i < self.dynamic_stops.len());
            self.dynamic_stops[dynamic_i]
        };

        unit & mask == mask
    }

    /// Resize this to support up to cols columns.
    // TODO: needs interval to set new tabstops
    pub fn resize(&mut self, cols: usize) -> Result<(), TryReserveError> {
        // Do nothing if it fits.
        if cols <= PREALLOC_COLUMNS {
            self.cols = cols;
            return Ok(());
        }

        // What we need in the dynamic size
        let size = cols - PREALLOC_COLUMNS;
        if size < self.dynamic_stops.len() {
            self.cols = cols;
            return Ok(());
        }

        // Allocate first so that failure leaves our state untouched.
        let mut new: Vec<Unit> = Vec::new();
        new.try_reserve_exact(size)?;
        new.resize(size, 0);
        new[..self.dynamic_stops.len()].copy_from_slice(&self.dynamic_stops);

        self.dynamic_stops = new;
        self.cols = cols;
        Ok(())
    }

    /// Return the maximum number of columns this can support currently.
    pub fn capacity(&self) -> usize {
        (PREALLOC_COUNT + self.dynamic_stops.len()) * UNIT_BITS
    }

    /// Unset all tabstops and then reset the initial tabstops to the given
    /// interval. An interval of 0 sets no tabstops.
    pub fn reset(&mut self, interval: usize) {
        self.prealloc_stops.fill(0);
        self.dynamic_stops.fill(0);

        if interval > 0 {
            let mut i = interval;
            while i + 1 < self.cols {
                self.set(i);
                i += interval;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabstops_basic() {
        let mut t = Tabstops::default();
        assert_eq!(0, entry(4));
        assert_eq!(1, entry(8));
        assert_eq!(0, index(0));
        assert_eq!(1, index(1));
        assert_eq!(1, index(9));

        assert_eq!(0b00001000, MASKS[3]);
        assert_eq!(0b00010000, MASKS[4]);

        assert!(!t.get(4));
        t.set(4);
        assert!(t.get(4));
        assert!(!t.get(3));

        t.reset(0);
        assert!(!t.get(4));

        t.set(4);
        assert!(t.get(4));
        t.unset(4);
        assert!(!t.get(4));
    }

    #[test]
    fn tabstops_dynamic_allocations() {
        let mut t = Tabstops::default();

        // Grow the capacity by 2.
        let cap = t.capacity();
        t.resize(cap * 2).unwrap();

        // Set something that was out of range of the first
        t.set(cap + 5);
        assert!(t.get(cap + 5));
        assert!(!t.get(cap + 4));

        // Prealloc still works
        assert!(!t.get(5));
    }

    #[test]
    fn tabstops_interval() {
        let t = Tabstops::new(80, 4).unwrap();
        assert!(!t.get(0));
        assert!(t.get(4));
        assert!(!t.get(5));
        assert!(t.get(8));
    }

    #[test]
    fn tabstops_count_on_80() {
        // https://superuser.com/questions/710019/why-there-are-11-tabstops-on-a-80-column-console
        let t = Tabstops::new(80, 8).unwrap();

        // Count the tabstops
        let count = (0..80).filter(|&i| t.get(i)).count();
        assert_eq!(9, count);
    }

    // "Tabstops: resize alloc failure preserves state" is skipped: it uses
    // Zig tripwire allocation failure injection. The Rust resize allocates
    // before mutating, so a failure leaves `cols` unchanged by construction.
}
