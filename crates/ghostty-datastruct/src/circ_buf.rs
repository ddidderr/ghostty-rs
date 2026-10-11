//! A circular buffer (`circ_buf.zig`).

use crate::OutOfMemory;

/// Direction of a [`Iter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Reverse,
}

/// A circular buffer containing type `T`, filled with a `default` value.
#[derive(Debug, Clone)]
pub struct CircBuf<T> {
    // Implementation note: there's a lot of unsafe addition of usize
    // here in this implementation that can technically overflow. If someone
    // wants to fix this and make it overflow safe (use subtractions for
    // checks prior to additions) then I welcome it. In reality, we'd
    // have to be a really, really large terminal screen to even worry
    // about this so I'm punting it.
    pub storage: Vec<T>,
    pub head: usize,
    pub tail: usize,

    // We could remove this and just use math with head/tail to figure
    // it out, but our usage of circular buffers stores so much data that
    // this minor overhead is not worth optimizing out.
    pub full: bool,

    default: T,
}

/// Iterator over the circular buffer.
#[derive(Debug, Clone)]
pub struct Iter<'a, T> {
    buf: &'a CircBuf<T>,
    idx: usize,
    direction: Direction,
}

impl<T> Iter<'_, T> {
    /// Seek the iterator by a given amount. This will clamp
    /// the values to the bounds of the buffer so overflows are
    /// not possible.
    pub fn seek_by(&mut self, amount: isize) {
        if amount > 0 {
            self.idx = self.idx.saturating_add(amount as usize);
        } else {
            self.idx = self.idx.saturating_sub(amount.unsigned_abs());
        }
    }

    /// Reset the iterator back to the first value.
    pub fn reset(&mut self) {
        self.idx = 0;
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        if self.idx >= self.buf.len() {
            return None;
        }

        // Get our index from the tail
        let tail_idx = match self.direction {
            Direction::Forward => self.idx,
            Direction::Reverse => self.buf.len() - self.idx - 1,
        };

        // Translate the tail index to a storage index
        let storage_idx = (self.buf.tail + tail_idx) % self.buf.capacity();
        self.idx += 1;
        Some(&self.buf.storage[storage_idx])
    }
}

impl<T> CircBuf<T> {
    /// Returns if the buffer is currently empty. To check if its
    /// full, just check the "full" attribute.
    #[must_use]
    pub fn empty(&self) -> bool {
        !self.full && self.head == self.tail
    }

    /// Returns the total capacity allocated for this buffer.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.storage.len()
    }

    /// Returns the length in elements that are used.
    #[must_use]
    pub fn len(&self) -> usize {
        if self.full {
            return self.storage.len();
        }
        if self.head >= self.tail {
            return self.head - self.tail;
        }
        self.storage.len() - (self.tail - self.head)
    }

    /// Alias for [`empty`](Self::empty) (the clippy-conventional name).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.empty()
    }
}

impl<T: Clone> CircBuf<T> {
    /// Initialize a new circular buffer that can store `size` elements.
    #[must_use]
    pub fn new(size: usize, default: T) -> Self {
        Self {
            storage: vec![default.clone(); size],
            head: 0,
            tail: 0,
            full: size == 0,
            default,
        }
    }

    /// Append a single value to the buffer. If the buffer is full,
    /// an error will be returned.
    pub fn append(&mut self, v: T) -> Result<(), OutOfMemory> {
        if self.full {
            return Err(OutOfMemory);
        }
        self.storage[self.head] = v;
        self.head += 1;
        if self.head >= self.storage.len() {
            self.head = 0;
        }
        self.full = self.head == self.tail;
        Ok(())
    }

    /// Append a single value to the buffer, assuming there is capacity.
    pub fn append_assume_capacity(&mut self, v: T) {
        assert!(!self.full);
        self.storage[self.head] = v;
        self.head += 1;
        if self.head >= self.storage.len() {
            self.head = 0;
        }
        self.full = self.head == self.tail;
    }

    /// Append a slice to the buffer.
    pub fn append_slice_assume_capacity(&mut self, slice: &[T]) {
        let len = self.len();
        let [a, b] = self.get_ptr_slice(len, slice.len());
        let a_len = a.len();
        a.clone_from_slice(&slice[..a_len]);
        b.clone_from_slice(&slice[a_len..]);
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.full = false;
    }

    /// Iterate over the circular buffer.
    #[must_use]
    pub fn iterator(&self, direction: Direction) -> Iter<'_, T> {
        Iter {
            buf: self,
            idx: 0,
            direction,
        }
    }

    /// Get the first (oldest) value in the buffer.
    #[must_use]
    pub fn first(&self) -> Option<&T> {
        self.iterator(Direction::Forward).next()
    }

    /// Get the last (newest) value in the buffer.
    #[must_use]
    pub fn last(&self) -> Option<&T> {
        self.iterator(Direction::Reverse).next()
    }

    /// Mutable access to the first (oldest) value in the buffer.
    pub fn first_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            return None;
        }
        let idx = self.tail % self.capacity();
        Some(&mut self.storage[idx])
    }

    /// Mutable access to the last (newest) value in the buffer.
    pub fn last_mut(&mut self) -> Option<&mut T> {
        let len = self.len();
        if len == 0 {
            return None;
        }
        let idx = (self.tail + len - 1) % self.capacity();
        Some(&mut self.storage[idx])
    }

    /// Ensures that there is enough capacity to store `amount` more
    /// items via append.
    pub fn ensure_unused_capacity(&mut self, amount: usize) {
        let new_cap = self.len() + amount;
        if new_cap <= self.capacity() {
            return;
        }
        self.resize(new_cap);
    }

    /// Resize the buffer to the given size (larger or smaller).
    /// If larger, new values will be set to the default value.
    pub fn resize(&mut self, size: usize) {
        // Rotate to zero so it is aligned.
        self.rotate_to_zero();

        // Reallocate, this adds to the end so we're ready to go.
        let prev_len = self.len();
        let prev_cap = self.storage.len();
        self.storage.resize(size, self.default.clone());

        // If we grew, we need to set our new defaults. We can add it
        // at the end since we rotated to start. (Done by `resize` above.)
        if size > prev_cap {
            // Fix up our head/tail
            if self.full {
                self.head = prev_len;
                self.full = false;
            }
        }
    }

    /// Rotate the data so that it is zero-aligned.
    pub(crate) fn rotate_to_zero(&mut self) {
        // If we're already at zero then do nothing.
        if self.tail == 0 {
            return;
        }

        // Rotate our storage in-place.
        self.storage.rotate_left(self.tail);

        // Then fix up our head and tail.
        self.head = self.len() % self.storage.len();
        self.tail = 0;
    }

    /// Delete the oldest n values from the buffer. If there are less
    /// than n values in the buffer, it'll delete everything.
    pub fn delete_oldest(&mut self, n: usize) {
        assert!(n <= self.storage.len());

        // Special case n == 0 otherwise we will accidentally break
        // our circular buffer.
        if n == 0 {
            return;
        }

        // Clear the values back to default
        let default = self.default.clone();
        let [a, b] = self.get_ptr_slice(0, n);
        a.fill(default.clone());
        b.fill(default);

        // If we're not full, we can just advance the tail. We know
        // it'll be less than the length because otherwise we'd be full.
        self.tail += self.len().min(n);
        if self.tail >= self.storage.len() {
            self.tail -= self.storage.len();
        }
        self.full = false;
    }

    /// Returns the two (possibly empty) slices covering `slice_len` values at
    /// `offset`, and considers this full amount of data "written" if it is
    /// beyond the end of our buffer. This never "rotates" the buffer because
    /// the offset can only be within the size of the buffer.
    pub fn get_ptr_slice(&mut self, offset: usize, slice_len: usize) -> [&mut [T]; 2] {
        // Special case the empty slice fast-path.
        if slice_len == 0 {
            return [&mut [], &mut []];
        }

        assert!(offset + slice_len <= self.capacity());

        // End offset is the last offset (exclusive) for our slice.
        // We use exclusive because it makes the math easier.
        let end_offset = offset + slice_len;

        // If our slice can't fit it in our length, then we need to advance.
        if end_offset > self.len() {
            self.advance(end_offset - self.len());
        }

        // Our start and end indexes into the storage buffer
        let start_idx = self.storage_offset(offset);
        let end_idx = self.storage_offset(end_offset - 1);

        // Optimistically, our data fits in one slice
        if end_idx >= start_idx {
            return [&mut self.storage[start_idx..=end_idx], &mut []];
        }

        let (front, back) = self.storage.split_at_mut(start_idx);
        [back, &mut front[..=end_idx]]
    }

    /// Advances the head/tail so that we can store amount.
    fn advance(&mut self, amount: usize) {
        assert!(amount <= self.storage.len() - self.len());

        // Optimistically add our amount
        self.head += amount;

        // If we exceeded the length of the buffer, wrap around.
        if self.head >= self.storage.len() {
            self.head -= self.storage.len();
        }

        // If we're full, we have to keep tail lined up.
        if self.full {
            self.tail = self.head;
        }

        // We're full if the head reached the tail. The head can never
        // pass the tail because advance asserts amount is only in
        // available space left
        self.full = self.head == self.tail;
    }

    /// For a given offset from zero, this returns the offset in the
    /// storage buffer where this data can be found.
    fn storage_offset(&self, offset: usize) -> usize {
        assert!(offset < self.storage.len());

        // This should be subtraction ideally to avoid overflows but
        // it would take a really, really, huge buffer to overflow.
        let fits_offset = self.tail + offset;
        if fits_offset < self.storage.len() {
            return fits_offset;
        }
        fits_offset - self.storage.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(buf: &CircBuf<u8>, d: Direction) -> Vec<u8> {
        buf.iterator(d).copied().collect()
    }

    #[test]
    fn circ_buf_basic() {
        let buf = CircBuf::new(12, 0u8);

        assert!(buf.empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn circ_buf_append() {
        let mut buf = CircBuf::new(3, 0u8);

        buf.append(1).unwrap();
        buf.append(2).unwrap();
        buf.append(3).unwrap();
        assert_eq!(buf.append(4), Err(OutOfMemory));
        buf.delete_oldest(1);
        buf.append(4).unwrap();
        assert_eq!(buf.append(5), Err(OutOfMemory));
    }

    #[test]
    fn circ_buf_forward_iterator() {
        let mut buf = CircBuf::new(3, 0u8);

        // Empty
        assert!(buf.iterator(Direction::Forward).next().is_none());

        // Partially full
        buf.append(1).unwrap();
        buf.append(2).unwrap();
        assert_eq!(collect(&buf, Direction::Forward), [1, 2]);

        // Full
        buf.append(3).unwrap();
        assert_eq!(collect(&buf, Direction::Forward), [1, 2, 3]);

        // Delete and add
        buf.delete_oldest(1);
        buf.append(4).unwrap();
        assert_eq!(collect(&buf, Direction::Forward), [2, 3, 4]);
    }

    #[test]
    fn circ_buf_reverse_iterator() {
        let mut buf = CircBuf::new(3, 0u8);

        // Empty
        assert!(buf.iterator(Direction::Reverse).next().is_none());

        // Partially full
        buf.append(1).unwrap();
        buf.append(2).unwrap();
        assert_eq!(collect(&buf, Direction::Reverse), [2, 1]);

        // Full
        buf.append(3).unwrap();
        assert_eq!(collect(&buf, Direction::Reverse), [3, 2, 1]);

        // Delete and add
        buf.delete_oldest(1);
        buf.append(4).unwrap();
        assert_eq!(collect(&buf, Direction::Reverse), [4, 3, 2]);
    }

    #[test]
    fn circ_buf_iterator_seek() {
        let mut buf = CircBuf::new(4, 0u8);
        buf.append_slice_assume_capacity(&[1, 2, 3, 4]);
        let mut it = buf.iterator(Direction::Forward);
        it.seek_by(2);
        assert_eq!(it.next(), Some(&3));
        it.seek_by(-100);
        assert_eq!(it.next(), Some(&1));
        it.seek_by(isize::MAX);
        assert_eq!(it.next(), None);
        it.reset();
        assert_eq!(it.next(), Some(&1));
    }

    #[test]
    fn circ_buf_first_last() {
        let mut buf = CircBuf::new(3, 0u8);

        buf.append(1).unwrap();
        buf.append(2).unwrap();
        buf.append(3).unwrap();
        assert_eq!(buf.last(), Some(&3));
        assert_eq!(buf.first(), Some(&1));
        *buf.last_mut().unwrap() = 9;
        *buf.first_mut().unwrap() = 7;
        assert_eq!(collect(&buf, Direction::Forward), [7, 2, 9]);
    }

    #[test]
    fn circ_buf_first_last_empty() {
        let buf = CircBuf::new(0, 0u8);

        assert!(buf.first().is_none());
        assert!(buf.last().is_none());
    }

    #[test]
    fn circ_buf_first_last_empty_with_cap() {
        let mut buf = CircBuf::new(3, 0u8);

        assert!(buf.first().is_none());
        assert!(buf.last().is_none());
        assert!(buf.first_mut().is_none());
        assert!(buf.last_mut().is_none());
    }

    #[test]
    fn circ_buf_append_slice() {
        let mut buf = CircBuf::new(5, 0u8);

        buf.append_slice_assume_capacity(b"hello");
        assert_eq!(collect(&buf, Direction::Forward), b"hello");
    }

    #[test]
    fn circ_buf_append_slice_with_wrap() {
        let mut buf = CircBuf::new(4, 0u8);

        // Fill the buffer
        let cap = buf.capacity();
        let _ = buf.get_ptr_slice(0, cap);
        assert!(buf.full);
        assert_eq!(buf.len(), 4);

        // Delete
        buf.delete_oldest(2);
        assert!(!buf.full);
        assert_eq!(buf.len(), 2);

        buf.append_slice_assume_capacity(b"AB");
        assert_eq!(collect(&buf, Direction::Forward), [0, 0, b'A', b'B']);
    }

    #[test]
    fn circ_buf_get_ptr_slice_fits() {
        let mut buf = CircBuf::new(12, 0u8);

        let slices = buf.get_ptr_slice(0, 11);
        assert_eq!(slices[0].len(), 11);
        assert_eq!(slices[1].len(), 0);
        assert_eq!(buf.len(), 11);
    }

    #[test]
    fn circ_buf_get_ptr_slice_wraps() {
        let mut buf = CircBuf::new(4, 0u8);

        // Fill the buffer
        let cap = buf.capacity();
        let _ = buf.get_ptr_slice(0, cap);
        assert!(buf.full);
        assert_eq!(buf.len(), 4);

        // Delete
        buf.delete_oldest(2);
        assert!(!buf.full);
        assert_eq!(buf.len(), 2);

        // Get a slice that doesn't grow
        {
            let slices = buf.get_ptr_slice(0, 2);
            assert_eq!(slices[0].len(), 2);
            assert_eq!(slices[1].len(), 0);
            slices[0][0] = 1;
            slices[0][1] = 2;
        }
        assert_eq!(buf.len(), 2);

        // Get a slice that does grow, and forces wrap
        {
            let slices = buf.get_ptr_slice(2, 2);
            assert_eq!(slices[0].len(), 2);
            assert_eq!(slices[1].len(), 0);

            // should be empty
            assert_eq!(slices[0][0], 0);
            assert_eq!(slices[0][1], 0);
            slices[0][0] = 3;
            slices[0][1] = 4;
        }
        assert_eq!(buf.len(), 4);

        // Get a slice across boundaries
        {
            let slices = buf.get_ptr_slice(0, 4);
            assert_eq!(slices[0].len(), 2);
            assert_eq!(slices[1].len(), 2);

            assert_eq!(slices[0][0], 1);
            assert_eq!(slices[0][1], 2);
            assert_eq!(slices[1][0], 3);
            assert_eq!(slices[1][1], 4);
        }
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn circ_buf_rotate_to_zero() {
        let mut buf = CircBuf::new(12, 0u8);

        let _ = buf.get_ptr_slice(0, 11);
        buf.rotate_to_zero();
    }

    #[test]
    fn circ_buf_rotate_to_zero_offset() {
        let mut buf = CircBuf::new(4, 0u8);

        // Fill the buffer
        let _ = buf.get_ptr_slice(0, 3);
        assert_eq!(buf.len(), 3);

        // Delete
        buf.delete_oldest(2);
        assert!(!buf.full);
        assert_eq!(buf.len(), 1);
        assert!(buf.tail > 0 && buf.head >= buf.tail);

        // Rotate to zero
        buf.rotate_to_zero();
        assert_eq!(buf.tail, 0);
        assert_eq!(buf.head, 1);
    }

    #[test]
    fn circ_buf_rotate_to_zero_wraps() {
        let mut buf = CircBuf::new(4, 0u8);

        // Fill the buffer
        let _ = buf.get_ptr_slice(0, 3);
        assert_eq!(buf.len(), 3);
        assert!(buf.tail == 0 && buf.head == 3);

        // Delete all
        buf.delete_oldest(3);
        assert_eq!(buf.len(), 0);
        assert!(buf.tail == 3 && buf.head == 3);

        // Refill to force a wrap
        {
            let slices = buf.get_ptr_slice(0, 3);
            slices[0][0] = 1;
            slices[1][0] = 2;
            slices[1][1] = 3;
        }
        assert_eq!(buf.len(), 3);
        assert!(buf.tail == 3 && buf.head == 2);

        // Rotate to zero
        buf.rotate_to_zero();
        assert_eq!(buf.tail, 0);
        assert_eq!(buf.head, 3);
        {
            let slices = buf.get_ptr_slice(0, 3);
            assert_eq!(slices[0][0], 1);
            assert_eq!(slices[0][1], 2);
            assert_eq!(slices[0][2], 3);
        }
    }

    #[test]
    fn circ_buf_rotate_to_zero_full_no_wrap() {
        let mut buf = CircBuf::new(4, 0u8);

        // Fill the buffer
        let _ = buf.get_ptr_slice(0, 3);

        // Delete all
        buf.delete_oldest(3);

        // Refill to force a wrap
        {
            let slices = buf.get_ptr_slice(0, 4);
            slices[0][0] = 1;
            slices[1][0] = 2;
            slices[1][1] = 3;
            slices[1][2] = 4;
        }
        assert!(buf.full);

        // Rotate to zero
        buf.rotate_to_zero();
        assert!(buf.full);
        assert_eq!(buf.tail, 0);
        assert_eq!(buf.head, 0);
        {
            let slices = buf.get_ptr_slice(0, 4);
            assert_eq!(slices[0], [1, 2, 3, 4]);
        }
    }

    #[test]
    fn circ_buf_resize_grow_from_zero() {
        let mut buf = CircBuf::new(0, 0u8);
        assert!(buf.full);

        // Resize
        buf.resize(2);
        assert!(!buf.full);
        assert_eq!(buf.len(), 0);
        assert_eq!(buf.capacity(), 2);

        buf.append(1).unwrap();
        buf.append(2).unwrap();

        let slices = buf.get_ptr_slice(0, 2);
        assert_eq!(slices[0][0], 1);
        assert_eq!(slices[0][1], 2);
    }

    #[test]
    fn circ_buf_resize_grow() {
        let mut buf = CircBuf::new(4, 0u8);

        // Fill and write
        {
            let slices = buf.get_ptr_slice(0, 4);
            slices[0].copy_from_slice(&[1, 2, 3, 4]);
        }
        assert!(buf.full);

        // Resize
        buf.resize(6);
        assert!(!buf.full);
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.capacity(), 6);

        let slices = buf.get_ptr_slice(0, 4);
        assert_eq!(slices[0], [1, 2, 3, 4]);
    }

    #[test]
    fn circ_buf_resize_shrink() {
        let mut buf = CircBuf::new(4, 0u8);

        // Fill and write
        {
            let slices = buf.get_ptr_slice(0, 4);
            slices[0].copy_from_slice(&[1, 2, 3, 4]);
        }
        assert!(buf.full);

        // Resize
        buf.resize(3);
        assert!(buf.full);
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.capacity(), 3);

        let slices = buf.get_ptr_slice(0, 3);
        assert_eq!(slices[0], [1, 2, 3]);
    }

    #[test]
    fn circ_buf_append_empty_slice() {
        let mut buf = CircBuf::new(5, 0u8);

        // Appending an empty slice to empty buffer should be a no-op
        buf.append_slice_assume_capacity(b"");
        assert_eq!(buf.len(), 0);
        assert!(!buf.full);

        // Buffer should still work normally after appending empty slice
        buf.append_slice_assume_capacity(b"hi");
        assert_eq!(buf.len(), 2);

        // Appending an empty slice to non-empty buffer should also be a no-op
        buf.append_slice_assume_capacity(b"");
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn circ_buf_get_ptr_slice_zero_length() {
        let mut buf = CircBuf::new(5, 0u8);

        // zero length on empty buffer should return empty slices
        let slices = buf.get_ptr_slice(0, 0);
        assert_eq!(slices[0].len(), 0);
        assert_eq!(slices[1].len(), 0);
        assert_eq!(buf.len(), 0);

        // Fill buffer partially
        buf.append_slice_assume_capacity(b"abc");
        assert_eq!(buf.len(), 3);

        // zero length on non-empty buffer should also work
        let slices2 = buf.get_ptr_slice(0, 0);
        assert_eq!(slices2[0].len(), 0);
        assert_eq!(slices2[1].len(), 0);
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn circ_buf_delete_oldest_zero() {
        let mut buf = CircBuf::new(5, 0u8);

        // delete_oldest(0) on empty buffer should be a no-op
        buf.delete_oldest(0);
        assert_eq!(buf.len(), 0);

        // Fill buffer
        buf.append_slice_assume_capacity(b"hello");
        assert_eq!(buf.len(), 5);

        // delete_oldest(0) on non-empty buffer should be a no-op
        buf.delete_oldest(0);
        assert_eq!(buf.len(), 5);

        // Verify data is unchanged
        assert_eq!(buf.iterator(Direction::Forward).next(), Some(&b'h'));
    }

    #[test]
    fn circ_buf_ensure_unused_capacity() {
        let mut buf = CircBuf::new(2, 0u8);
        buf.append(1).unwrap();
        buf.ensure_unused_capacity(1);
        assert_eq!(buf.capacity(), 2);
        buf.ensure_unused_capacity(5);
        assert_eq!(buf.capacity(), 6);
        assert_eq!(collect(&buf, Direction::Forward), [1]);
    }
}
