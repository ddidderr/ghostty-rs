//! A pool of values with stable slots (`segmented_pool.zig`).

/// Returned by [`SegmentedPool::get`] when there are no values available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("out of values")]
pub struct OutOfValues;

/// A data structure where you can get stable (never moved) slots for
/// a type that automatically grows if necessary. The values can be "put back"
/// but are expected to be put back IN ORDER.
///
/// This is implemented specifically for libuv write requests, since the
/// write requests must have a stable pointer and are guaranteed to be processed
/// in order for a single stream.
///
/// Instead of raw pointers, slots are identified by a stable index returned
/// from [`get`](Self::get) and accessed with [`at`](Self::at) /
/// [`at_mut`](Self::at_mut).
///
/// This is NOT thread safe.
#[derive(Debug, Clone)]
pub struct SegmentedPool<T: Default, const PREALLOC: usize> {
    i: usize,
    available: usize,
    list: Vec<T>,
}

impl<T: Default, const PREALLOC: usize> Default for SegmentedPool<T, PREALLOC> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Default, const PREALLOC: usize> SegmentedPool<T, PREALLOC> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            i: 0,
            available: PREALLOC,
            list: (0..PREALLOC).map(|_| T::default()).collect(),
        }
    }

    /// Number of values currently available without growing.
    #[must_use]
    pub fn available(&self) -> usize {
        self.available
    }

    /// Total number of slots.
    #[must_use]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Access the slot at an index returned by `get`.
    #[must_use]
    pub fn at(&self, idx: usize) -> &T {
        &self.list[idx]
    }

    /// Mutably access the slot at an index returned by `get`.
    pub fn at_mut(&mut self, idx: usize) -> &mut T {
        &mut self.list[idx]
    }

    /// Get the next available value out of the list (as a slot index). This
    /// will not grow the list.
    pub fn get(&mut self) -> Result<usize, OutOfValues> {
        // Error to not have any
        if self.available == 0 {
            return Err(OutOfValues);
        }

        // The index we grab is just i % len, so we wrap around to the front.
        let i = self.i % self.list.len();
        self.i = self.i.wrapping_add(1); // Wrapping addition so we go back to 0
        self.available -= 1;
        Ok(i)
    }

    /// Get the next available value out of the list and grow the list
    /// if necessary.
    pub fn get_grow(&mut self) -> Result<usize, OutOfValues> {
        if self.available == 0 {
            self.grow();
        }
        self.get()
    }

    fn grow(&mut self) {
        let old = self.list.len();
        self.list.extend((0..old).map(|_| T::default()));
        self.i = old;
        self.available = old;
    }

    /// Put a value back. The value put back is expected to be the
    /// in order of get.
    pub fn put(&mut self) {
        self.available += 1;
        assert!(self.available <= self.list.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segmented_pool() {
        let mut list: SegmentedPool<u8, 2> = SegmentedPool::new();
        assert_eq!(list.available(), 2);

        // Get to capacity
        let v1 = list.get().unwrap();
        let v2 = list.get().unwrap();
        assert_ne!(v1, v2);
        assert_eq!(list.get(), Err(OutOfValues));

        // Test writing for later
        *list.at_mut(v1) = 42;

        // Put a value back
        list.put();
        let temp = list.get().unwrap();
        assert_eq!(v1, temp);
        assert_eq!(*list.at(temp), 42);
        assert_eq!(list.get(), Err(OutOfValues));

        // Grow
        let v3 = list.get_grow().unwrap();
        assert!(v1 != v3 && v2 != v3);
        list.get().unwrap();
        assert_eq!(list.get(), Err(OutOfValues));

        // Put a value back
        list.put();
        assert_eq!(v1, list.get().unwrap());
        assert_eq!(list.get(), Err(OutOfValues));
    }
}
