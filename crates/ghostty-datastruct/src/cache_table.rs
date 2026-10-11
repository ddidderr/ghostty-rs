//! A fixed-size-bucket hash cache with LRU replacement (`cache_table.zig`).

/// Behaviours for a [`CacheTable`] (the Zig `Context`).
pub trait CacheContext<K, V> {
    /// Return a hash for a key.
    fn hash(&mut self, key: &K) -> u64;

    /// Check two keys for equality.
    fn eql(&mut self, a: &K, b: &K) -> bool;

    /// [OPTIONAL] Eviction callback. Called whenever an item is evicted
    /// from the cache.
    fn evicted(&mut self, _key: K, _value: V) {}
}

/// A key/value pair stored in the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kv<K, V> {
    pub key: K,
    pub value: V,
}

/// An associative data structure used for efficiently storing and
/// retrieving values which are able to be recomputed if necessary.
///
/// This structure is effectively a hash table with fixed-sized buckets.
///
/// When inserting an item in to a full bucket, the least recently used
/// item is replaced.
///
/// To achieve this, when an item is accessed, it's moved to the end of
/// the bucket, and the rest of the items are moved over to fill the gap.
///
/// This should provide very good query performance and keep frequently
/// accessed items cached indefinitely.
///
/// Parameters:
///
/// `C`
///   A [`CacheContext`] defining hash/eql (and optionally an eviction hook).
///
/// `BUCKET_COUNT`
///   Should ideally be close to the median number of important items that
///   you expect to be cached at any given point. This is required to be a
///   power of 2 since performance suffers if it's not and there's no good
///   reason to allow it to be anything else.
///
/// `BUCKET_SIZE`
///   should be larger if you expect a large number of unimportant items to
///   enter the cache at a time. Having larger buckets will avoid important
///   items being dropped from the cache prematurely.
#[derive(Debug)]
pub struct CacheTable<K, V, C, const BUCKET_COUNT: usize, const BUCKET_SIZE: usize> {
    /// `BUCKET_COUNT` buckets containing up to `BUCKET_SIZE` KV pairs each;
    /// oldest first.
    buckets: Vec<Vec<Kv<K, V>>>,

    /// An instance of the context structure.
    pub context: C,
}

impl<K, V, C, const BUCKET_COUNT: usize, const BUCKET_SIZE: usize>
    CacheTable<K, V, C, BUCKET_COUNT, BUCKET_SIZE>
where
    C: CacheContext<K, V>,
{
    const ASSERT_POW2: () = assert!(BUCKET_COUNT.is_power_of_two());
    const ASSERT_SIZE: () = assert!(BUCKET_SIZE > 0 && BUCKET_SIZE <= u8::MAX as usize);

    #[must_use]
    pub fn new(context: C) -> Self {
        let () = Self::ASSERT_POW2;
        let () = Self::ASSERT_SIZE;
        let mut buckets = Vec::with_capacity(BUCKET_COUNT);
        buckets.resize_with(BUCKET_COUNT, Vec::new);
        Self { buckets, context }
    }

    fn index(&mut self, key: &K) -> usize {
        (self.context.hash(key) % BUCKET_COUNT as u64) as usize
    }

    /// Adds an item to the cache table. If an old value was removed to
    /// make room then it is returned in a struct with its key and value.
    pub fn put(&mut self, key: K, value: V) -> Option<Kv<K, V>>
    where
        K: Clone,
        V: Clone,
    {
        let idx = self.index(&key);
        let bucket = &mut self.buckets[idx];

        // If we have space available in the bucket then we just append
        if bucket.len() < BUCKET_SIZE {
            bucket.push(Kv { key, value });
            return None;
        }
        debug_assert_eq!(bucket.len(), BUCKET_SIZE);

        // Append our new item and return the oldest
        let evicted = bucket.remove(0);
        bucket.push(Kv { key, value });

        // The Context is allowed to register an eviction hook.
        self.context
            .evicted(evicted.key.clone(), evicted.value.clone());

        Some(evicted)
    }

    /// Retrieves an item from the cache table (marking it most recently
    /// used).
    ///
    /// Returns `None` if no item is found with the provided key.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        let idx = self.index(key);
        let len = self.buckets[idx].len();
        let mut i = len;
        while i > 0 {
            i -= 1;
            if self.context.eql(key, &self.buckets[idx][i].key) {
                // Rotate the item to the end of the bucket.
                self.buckets[idx][i..len].rotate_left(1);
                return Some(&self.buckets[idx][len - 1].value);
            }
        }

        None
    }

    /// Removes all items from the cache table.
    ///
    /// If your `Context` has an `evicted` method,
    /// it will be called with all removed items.
    pub fn clear(&mut self) {
        for b in &mut self.buckets {
            for kv in b.drain(..) {
                self.context.evicted(kv.key, kv.value);
            }
        }
    }
}

/// A context for any `K: Hash + Eq` using the std default hasher (the Zig
/// `AutoContext`).
#[derive(Debug, Default, Clone, Copy)]
pub struct AutoContext;

impl<K: std::hash::Hash + Eq, V> CacheContext<K, V> for AutoContext {
    fn hash(&mut self, key: &K) -> u64 {
        use std::hash::Hasher;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        h.finish()
    }

    fn eql(&mut self, a: &K, b: &K) -> bool {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A context that has a predictable hash so we can test all edge cases.
    #[derive(Default)]
    struct Ctx {
        evicted: Vec<(u32, u32)>,
    }

    impl CacheContext<u32, u32> for Ctx {
        fn hash(&mut self, key: &u32) -> u64 {
            u64::from(*key)
        }
        fn eql(&mut self, a: &u32, b: &u32) -> bool {
            a == b
        }
        fn evicted(&mut self, key: u32, value: u32) {
            self.evicted.push((key, value));
        }
    }

    #[test]
    fn cache_table() {
        let mut t: CacheTable<u32, u32, Ctx, 2, 2> = CacheTable::new(Ctx::default());

        // Fill the table
        assert!(t.put(0, 0).is_none());
        assert!(t.put(1, 0).is_none());
        assert!(t.put(2, 0).is_none());
        assert!(t.put(3, 0).is_none());

        // It should now be full, so any insert should evict the oldest item.
        // NOTE: For the sake of this test, we're assuming that the first item
        // is evicted but we don't need to promise this.
        assert_eq!(t.put(4, 0).unwrap(), Kv { key: 0, value: 0 });

        // The first item should now be gone
        assert!(t.get(&0).is_none());
        assert_eq!(t.context.evicted, vec![(0, 0)]);
    }

    #[test]
    fn cache_table_get_refreshes_and_clear() {
        let mut t: CacheTable<u32, u32, Ctx, 1, 3> = CacheTable::new(Ctx::default());
        t.put(1, 10);
        t.put(2, 20);
        t.put(3, 30);
        // Touch 1 so 2 becomes the oldest.
        assert_eq!(t.get(&1), Some(&10));
        assert_eq!(t.put(4, 40).unwrap().key, 2);
        assert_eq!(t.get(&3), Some(&30));
        t.clear();
        assert!(t.get(&1).is_none());
        assert_eq!(t.context.evicted.len(), 1 + 3);
    }

    #[test]
    fn cache_table_auto_context() {
        let mut t: CacheTable<String, u8, AutoContext, 4, 2> = CacheTable::new(AutoContext);
        t.put("a".to_string(), 1);
        assert_eq!(t.get(&"a".to_string()), Some(&1));
        assert_eq!(t.get(&"b".to_string()), None);
    }
}
