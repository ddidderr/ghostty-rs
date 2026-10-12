//! A hash map with least-recently-used eviction (`lru.zig`).
//!
//! The Zig version threads an intrusive `std.DoublyLinkedList` through
//! allocated entries; here the entries live in a slab and are linked by
//! index.

use std::{
    collections::HashMap as StdHashMap,
    hash::{BuildHasher, Hash, RandomState},
};

/// Create a HashMap for a key type that can be automatically hashed.
/// If you want finer-grained control, use [`HashMap`] directly.
pub type AutoHashMap<K, V> = HashMap<K, V, RandomState>;

/// A key/value pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kv<K, V> {
    pub key: K,
    pub value: V,
}

/// The result of a `get_or_put` operation.
#[derive(Debug)]
pub struct GetOrPutResult<'a, K, V> {
    /// The entry that was retrieved. If `found_existing` is false,
    /// then this is a reference to space to store a V (initialized to
    /// `V::default()`). If `found_existing` is true, the value is valid, but
    /// can be overwritten.
    pub value_ptr: &'a mut V,

    /// Whether an existing value was found or not.
    pub found_existing: bool,

    /// If another entry had to be evicted to make space for this
    /// put operation, then this is the value that was evicted.
    pub evicted: Option<Kv<K, V>>,
}

#[derive(Debug)]
struct Entry<K, V> {
    key: K,
    value: V,
    prev: Option<usize>,
    next: Option<usize>,
}

/// HashMap implementation that supports least-recently-used eviction.
///
/// Note: This is a really elementary CS101 version of an LRU right now.
/// This is done initially to get something working. Once we have it working,
/// we can benchmark and improve if this ends up being a source of slowness.
#[derive(Debug)]
pub struct HashMap<K, V, S = RandomState> {
    /// Map to maintain our entries (key -> slab index).
    map: StdHashMap<K, usize, S>,

    /// Slab of entries.
    entries: Vec<Option<Entry<K, V>>>,
    free: Vec<usize>,

    /// Queue to maintain LRU order: `first` is the least recently used.
    first: Option<usize>,
    last: Option<usize>,

    /// The capacity of our map. If this capacity is reached, cache
    /// misses will begin evicting entries.
    capacity: usize,
}

impl<K: Hash + Eq + Clone, V: Default> HashMap<K, V, RandomState> {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self::with_hasher(capacity, RandomState::new())
    }
}

impl<K: Hash + Eq + Clone, V: Default, S: BuildHasher> HashMap<K, V, S> {
    #[must_use]
    pub fn with_hasher(capacity: usize, hasher: S) -> Self {
        Self {
            map: StdHashMap::with_hasher(hasher),
            entries: Vec::new(),
            free: Vec::new(),
            first: None,
            last: None,
            capacity,
        }
    }

    /// The capacity of the map.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn entry(&self, idx: usize) -> &Entry<K, V> {
        self.entries[idx].as_ref().expect("live lru entry")
    }

    fn entry_mut(&mut self, idx: usize) -> &mut Entry<K, V> {
        self.entries[idx].as_mut().expect("live lru entry")
    }

    /// Unlink an entry from the queue.
    fn unlink(&mut self, idx: usize) {
        let (prev, next) = {
            let e = self.entry(idx);
            (e.prev, e.next)
        };
        match prev {
            Some(p) => self.entry_mut(p).next = next,
            None => self.first = next,
        }
        match next {
            Some(n) => self.entry_mut(n).prev = prev,
            None => self.last = prev,
        }
        let e = self.entry_mut(idx);
        e.prev = None;
        e.next = None;
    }

    /// Append an (unlinked) entry to the end (most recently used).
    fn append(&mut self, idx: usize) {
        let last = self.last;
        {
            let e = self.entry_mut(idx);
            e.prev = last;
            e.next = None;
        }
        match last {
            Some(l) => self.entry_mut(l).next = Some(idx),
            None => self.first = Some(idx),
        }
        self.last = Some(idx);
    }

    /// Get or put a value for a key. See [`GetOrPutResult`] on how to check
    /// if an existing value was found, if an existing value was evicted,
    /// etc.
    pub fn get_or_put(&mut self, key: K) -> GetOrPutResult<'_, K, V> {
        if let Some(&idx) = self.map.get(&key) {
            // Move to end to mark as most recently used
            self.unlink(idx);
            self.append(idx);

            return GetOrPutResult {
                found_existing: true,
                value_ptr: &mut self.entry_mut(idx).value,
                evicted: None,
            };
        }

        // We're evicting if our map insertion would increase our size
        // beyond capacity.
        let evict = self.map.len() + 1 > self.capacity;

        let (idx, evicted) = if evict {
            // Our first node is the least recently used. We avoid
            // allocation by reusing the entry we evict.
            let lru = self.first.expect("capacity must be non-zero to evict");
            self.unlink(lru);

            // Remove the least used from the map
            let old_key = self.entry(lru).key.clone();
            self.map.remove(&old_key);

            let e = self.entry_mut(lru);
            let old_key = std::mem::replace(&mut e.key, key.clone());
            let old_value = std::mem::take(&mut e.value);
            (
                lru,
                Some(Kv {
                    key: old_key,
                    value: old_value,
                }),
            )
        } else {
            let entry = Entry {
                key: key.clone(),
                value: V::default(),
                prev: None,
                next: None,
            };
            let idx = if let Some(i) = self.free.pop() {
                self.entries[i] = Some(entry);
                i
            } else {
                self.entries.push(Some(entry));
                self.entries.len() - 1
            };
            (idx, None)
        };

        // Store our entry in the map and mark it as most recently used.
        self.map.insert(key, idx);
        self.append(idx);

        GetOrPutResult {
            found_existing: false,
            value_ptr: &mut self.entry_mut(idx).value,
            evicted,
        }
    }

    /// Get a value for a key. This does not change the LRU order.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<&V> {
        let idx = *self.map.get(key)?;
        Some(&self.entry(idx).value)
    }

    /// Resize the LRU. If this shrinks the LRU then LRU items will be
    /// removed. The removed values are returned (oldest first).
    pub fn resize(&mut self, capacity: usize) -> Option<Vec<V>> {
        // Fastest
        if capacity >= self.capacity {
            self.capacity = capacity;
            return None;
        }

        // If we're shrinking but we're smaller than the new capacity,
        // then we don't have to do anything.
        if self.map.len() <= capacity {
            self.capacity = capacity;
            return None;
        }

        // We're shrinking and we have more items than the new capacity
        let delta = self.map.len() - capacity;
        let mut evicted = Vec::with_capacity(delta);
        for _ in 0..delta {
            let idx = self.first.expect("non-empty queue");
            self.unlink(idx);
            let entry = self.entries[idx].take().expect("live lru entry");
            self.free.push(idx);
            self.map.remove(&entry.key);
            evicted.push(entry.value);
        }

        self.capacity = capacity;
        assert_eq!(self.map.len(), capacity);

        Some(evicted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_or_put() {
        let mut m = AutoHashMap::<u32, u8>::new(2);

        // Insert cap values, should be hits
        {
            let gop = m.get_or_put(1);
            assert!(!gop.found_existing);
            assert!(gop.evicted.is_none());
            *gop.value_ptr = 1;
        }
        {
            let gop = m.get_or_put(2);
            assert!(!gop.found_existing);
            assert!(gop.evicted.is_none());
            *gop.value_ptr = 2;
        }

        // 1 is LRU
        assert!(m.get_or_put(1).found_existing);
        assert!(m.get_or_put(2).found_existing);

        // Next should evict
        {
            let gop = m.get_or_put(3);
            assert!(!gop.found_existing);
            assert!(gop.evicted.is_some());
            assert_eq!(gop.evicted.unwrap().value, 1);
            *gop.value_ptr = 3;
        }

        // Currently: 2 is LRU, let's make 3 LRU
        assert!(m.get_or_put(2).found_existing);

        // Next should evict
        {
            let gop = m.get_or_put(4);
            assert!(!gop.found_existing);
            let ev = gop.evicted.unwrap();
            assert_eq!(ev.value, 3);
            assert_eq!(ev.key, 3);
            *gop.value_ptr = 4;
        }
    }

    #[test]
    fn get() {
        let mut m = AutoHashMap::<u32, u8>::new(2);

        // Insert cap values, should be hits
        {
            let gop = m.get_or_put(1);
            assert!(!gop.found_existing);
            assert!(gop.evicted.is_none());
            *gop.value_ptr = 1;
        }

        assert!(m.get(&1).is_some());
        assert_eq!(*m.get(&1).unwrap(), 1);
        assert!(m.get(&2).is_none());
    }

    #[test]
    fn resize_shrink_without_removal() {
        let mut m = AutoHashMap::<u32, u8>::new(2);

        // Insert cap values, LRU is 1
        {
            let gop = m.get_or_put(1);
            assert!(!gop.found_existing);
            assert!(gop.evicted.is_none());
            *gop.value_ptr = 1;
        }

        // Shrink
        let evicted = m.resize(1);
        assert!(evicted.is_none());
        {
            let gop = m.get_or_put(1);
            assert!(gop.found_existing);
        }
    }

    #[test]
    fn resize_shrink_and_remove() {
        let mut m = AutoHashMap::<u32, u8>::new(2);

        // Insert cap values, LRU is 1
        {
            let gop = m.get_or_put(1);
            assert!(!gop.found_existing);
            assert!(gop.evicted.is_none());
            *gop.value_ptr = 1;
        }
        {
            let gop = m.get_or_put(2);
            assert!(!gop.found_existing);
            assert!(gop.evicted.is_none());
            *gop.value_ptr = 2;
        }

        // Shrink
        let evicted = m.resize(1).unwrap();
        assert_eq!(evicted.len(), 1);
        {
            let gop = m.get_or_put(1);
            assert!(!gop.found_existing);
            assert_eq!(gop.evicted.as_ref().unwrap().value, 2);
            *gop.value_ptr = 1;
        }
    }

    #[test]
    fn resize_grow_and_slot_reuse() {
        let mut m = AutoHashMap::<u32, u8>::new(1);
        *m.get_or_put(1).value_ptr = 1;
        assert!(m.resize(3).is_none());
        *m.get_or_put(2).value_ptr = 2;
        *m.get_or_put(3).value_ptr = 3;
        assert_eq!(m.len(), 3);
        // Shrink to 1 evicts oldest-first and frees slots for reuse.
        assert_eq!(m.resize(1).unwrap(), vec![1, 2]);
        *m.get_or_put(4).value_ptr = 4;
        assert_eq!(m.len(), 1);
        assert_eq!(m.get(&4), Some(&4));
        assert_eq!(m.get(&3), None);
    }
}
