//! Bounded, deduplicated values with stable nonzero identifiers.
use std::{
    borrow::Cow,
    hash::{DefaultHasher, Hash, Hasher},
};
/// Errors callers can recover from by rebuilding or growing a page.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AddError {
    /// The set has exhausted its logical capacity.
    OutOfMemory,
    /// Dead low IDs prevent allocating another sequential ID.
    NeedsRehash,
    /// Identifier does not refer to a live entry.
    InvalidId,
    /// A reference count would overflow or underflow.
    InvalidRefCount,
}
impl std::fmt::Display for AddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for AddError {}
/// Logical layout of the Zig set, independent of Rust collection overhead.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Slots including reserved ID zero.
    pub cap: usize,
    /// Power-of-two hash table capacity.
    pub table_cap: usize,
    /// Table byte offset.
    pub table_start: usize,
    /// Item byte offset.
    pub items_start: usize,
    /// Total byte requirement.
    pub total_size: usize,
}
impl Layout {
    /// Calculate a layout using the reference ABI's item size and alignment.
    pub fn init(cap: usize, item_size: usize, item_align: usize) -> Self {
        if cap == 0 {
            return Self::default();
        }
        let table_cap = cap.next_power_of_two();
        let items_start = (table_cap * 2).next_multiple_of(item_align);
        let cap = table_cap * 13 / 16;
        Self {
            cap,
            table_cap,
            table_start: 0,
            items_start,
            total_size: items_start + cap * item_size,
        }
    }
}
#[derive(Debug, Clone)]
struct Item<T> {
    value: T,
    refs: u16,
    bucket: usize,
    psl: usize,
}
/// A bounded Robin Hood table with stable nonzero IDs. Dead items remain in
/// their buckets until trimming, explicit ID reuse, or insertion reaps them.
/// Probe statistics enforce the same 32-probe insertion limit as Zig.
#[derive(Debug, Clone)]
pub struct RefCountedSet<T> {
    slots: Vec<Option<Item<T>>>,
    table: Vec<u16>,
    hash: fn(&T) -> u64,
    psl_stats: [usize; 32],
    max_psl: usize,
    next_id: usize,
    living: usize,
}
impl<T: Eq + Hash + Clone> RefCountedSet<T> {
    /// Construct with Zig's 13/16 load factor and power-of-two rounding.
    pub fn init(cap: usize) -> Result<Self, AddError> {
        Self::init_with_hasher(cap, |value| {
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        })
    }
    /// Construct with the caller's deterministic hash context. Page styles and
    /// hyperlinks use the reference hashes so probe-dependent ID reuse agrees.
    pub fn init_with_hasher(cap: usize, hash: fn(&T) -> u64) -> Result<Self, AddError> {
        if cap > 65536 {
            return Err(AddError::OutOfMemory);
        }
        let slots_len = if cap == 0 {
            0
        } else {
            cap.next_power_of_two() * 13 / 16
        };
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(slots_len)
            .map_err(|_| AddError::OutOfMemory)?;
        slots.resize_with(slots_len, || None);
        let mut table = Vec::new();
        let table_cap = if cap == 0 { 0 } else { cap.next_power_of_two() };
        table
            .try_reserve_exact(table_cap)
            .map_err(|_| AddError::OutOfMemory)?;
        table.resize(table_cap, 0);
        Ok(Self {
            slots,
            table,
            hash,
            psl_stats: [0; 32],
            max_psl: 0,
            next_id: 1,
            living: 0,
        })
    }
    /// Release all values while retaining the table, slots, and hash context.
    pub fn clear(&mut self) {
        self.slots.fill_with(|| None);
        self.table.fill(0);
        self.psl_stats.fill(0);
        self.max_psl = 0;
        self.next_id = 1;
        self.living = 0;
    }
    /// Minimum table capacity for a given number of live values.
    pub fn capacity_for_count(n: usize) -> usize {
        if n == 0 {
            0
        } else {
            ((n + 1) * 16).div_ceil(13)
        }
    }
    /// Maximum number of live values, excluding ID zero.
    pub fn capacity(&self) -> usize {
        self.slots.len().saturating_sub(1)
    }
    /// Number of values with nonzero reference counts.
    pub fn count(&self) -> usize {
        self.living
    }
    /// Look up a live value without retaining it.
    pub fn lookup(&self, value: &T) -> Option<u16> {
        if self.table.is_empty() {
            return None;
        }
        let hash = (self.hash)(value) as usize;
        for probe in 0..=self.max_psl {
            let id = self.table[hash.wrapping_add(probe) & (self.table.len() - 1)];
            if id == 0 {
                return None;
            }
            let item = self.slots[usize::from(id)].as_ref()?;
            if item.psl < probe {
                return None;
            }
            if item.psl == probe && item.refs > 0 && item.value == *value {
                return Some(id);
            }
        }
        None
    }
    /// Add or retain a value. Trim trailing dead IDs before lookup, then check
    /// the probe ceiling before deciding between NeedsRehash and OutOfMemory.
    pub fn add(&mut self, value: T) -> Result<u16, AddError> {
        self.add_value(Cow::Owned(value))
    }
    fn add_value(&mut self, value: Cow<'_, T>) -> Result<u16, AddError> {
        while self.next_id > 1 && self.ref_count((self.next_id - 1) as u16) == 0 {
            self.next_id -= 1;
            self.delete_item(self.next_id);
        }
        if let Some(id) = self.lookup(&value) {
            self.r#use(id)?;
            return Ok(id);
        }
        if self.psl_stats[31] > 0 {
            return Err(AddError::OutOfMemory);
        }
        if self.next_id >= self.slots.len() {
            return Err(if self.living < self.slots.len() * 9 / 10 {
                AddError::NeedsRehash
            } else {
                AddError::OutOfMemory
            });
        }
        let id = self.insert(value.into_owned(), self.next_id);
        self.slots[id].as_mut().expect("inserted item").refs = 1;
        self.living += 1;
        if id == self.next_id {
            self.next_id += 1;
        }
        Ok(id as u16)
    }
    /// Prefer an allocated ID. A dead preferred ID is deleted before lookup,
    /// including the backwards shift of its following probe sequence.
    pub fn add_with_id(&mut self, value: T, id: u16) -> Result<Option<u16>, AddError> {
        self.add_with_id_value(Cow::Owned(value), id)
    }
    /// Borrow managed values during page copies; only a newly inserted value
    /// needs its persistent storage duplicated.
    pub(crate) fn add_ref_with_id(&mut self, value: &T, id: u16) -> Result<Option<u16>, AddError> {
        self.add_with_id_value(Cow::Borrowed(value), id)
    }
    fn add_with_id_value(&mut self, value: Cow<'_, T>, id: u16) -> Result<Option<u16>, AddError> {
        if id == 0 {
            return Err(AddError::InvalidId);
        }
        if usize::from(id) < self.next_id {
            if self.ref_count(id) == 0 {
                if self.psl_stats[31] > 0 {
                    return Err(AddError::OutOfMemory);
                }
                self.delete_item(usize::from(id));
                let actual = self.lookup(&value).map_or_else(
                    || self.insert(value.into_owned(), usize::from(id)),
                    usize::from,
                );
                let item = self.slots[actual].as_mut().expect("upserted item");
                item.refs = item.refs.checked_add(1).ok_or(AddError::InvalidRefCount)?;
                self.living += 1;
                return Ok((actual != usize::from(id)).then_some(actual as u16));
            }
            if self.get(id) == Some(value.as_ref()) {
                self.r#use(id)?;
                return Ok(None);
            }
        }
        self.add_value(value).map(Some)
    }
    fn delete_item(&mut self, id: usize) {
        let Some(item) = self.slots[id].take() else {
            return;
        };
        self.psl_stats[item.psl] -= 1;
        let mut bucket = item.bucket;
        let mask = self.table.len() - 1;
        loop {
            let next = (bucket + 1) & mask;
            let next_id = usize::from(self.table[next]);
            if next_id == 0 {
                break;
            }
            let next_item = self.slots[next_id].as_mut().expect("occupied bucket");
            if next_item.psl == 0 {
                break;
            }
            next_item.bucket = bucket;
            self.psl_stats[next_item.psl] -= 1;
            next_item.psl -= 1;
            self.psl_stats[next_item.psl] += 1;
            self.table[bucket] = next_id as u16;
            bucket = next;
        }
        self.table[bucket] = 0;
        while self.max_psl > 0 && self.psl_stats[self.max_psl] == 0 {
            self.max_psl -= 1;
        }
    }
    fn insert(&mut self, value: T, new_id: usize) -> usize {
        let hash = (self.hash)(&value) as usize;
        let mask = self.table.len() - 1;
        let mut held = Item {
            value,
            refs: 0,
            bucket: 0,
            psl: 0,
        };
        let mut held_id = new_id;
        let mut chosen_id = new_id;
        for probe in 0..self.table.len() - 1 {
            let bucket = hash.wrapping_add(probe) & mask;
            let id = usize::from(self.table[bucket]);
            let dead = id != 0 && self.slots[id].as_ref().expect("occupied bucket").refs == 0;
            if id == 0 || dead {
                if dead {
                    let item = self.slots[id].take().expect("dead item");
                    self.psl_stats[item.psl] -= 1;
                    if id < new_id {
                        chosen_id = id;
                    }
                }
                held.bucket = bucket;
                self.psl_stats[held.psl] += 1;
                self.max_psl = self.max_psl.max(held.psl);
                self.table[bucket] = held_id as u16;
                self.slots[held_id] = Some(held);
                break;
            }
            let item = self.slots[id].as_ref().expect("occupied bucket");
            if item.psl < held.psl || (item.psl == held.psl && item.refs < held.refs) {
                let displaced = self.slots[id].take().expect("displaced item");
                held.bucket = bucket;
                self.psl_stats[held.psl] += 1;
                self.max_psl = self.max_psl.max(held.psl);
                self.table[bucket] = held_id as u16;
                self.slots[held_id] = Some(held);
                self.psl_stats[displaced.psl] -= 1;
                held = displaced;
                held_id = id;
            }
            held.psl += 1;
        }
        if chosen_id != new_id {
            let item = self.slots[new_id].take().expect("newly inserted item");
            self.table[item.bucket] = chosen_id as u16;
            self.slots[chosen_id] = Some(item);
        }
        chosen_id
    }
    /// Retain a live value once.
    pub fn r#use(&mut self, id: u16) -> Result<(), AddError> {
        self.use_multiple(id, 1)
    }
    /// Retain a live value multiple times.
    pub fn use_multiple(&mut self, id: u16, n: u16) -> Result<(), AddError> {
        let item = self
            .slots
            .get_mut(usize::from(id))
            .and_then(Option::as_mut)
            .filter(|i| i.refs > 0)
            .ok_or(AddError::InvalidId)?;
        item.refs = item.refs.checked_add(n).ok_or(AddError::InvalidRefCount)?;
        Ok(())
    }
    /// Obtain a live value without retaining it.
    pub fn get(&self, id: u16) -> Option<&T> {
        self.slots
            .get(usize::from(id))?
            .as_ref()
            .filter(|i| i.refs > 0)
            .map(|i| &i.value)
    }
    /// Release one reference. Dead values remain in their slots until reuse.
    pub fn release(&mut self, id: u16) -> Result<(), AddError> {
        self.release_multiple(id, 1)
    }
    /// Release several references.
    pub fn release_multiple(&mut self, id: u16, n: u16) -> Result<(), AddError> {
        let item = self
            .slots
            .get_mut(usize::from(id))
            .and_then(Option::as_mut)
            .filter(|i| i.refs > 0)
            .ok_or(AddError::InvalidId)?;
        item.refs = item.refs.checked_sub(n).ok_or(AddError::InvalidRefCount)?;
        if item.refs == 0 {
            self.living -= 1;
        }
        Ok(())
    }
    /// Read the reference count, or zero for an absent ID.
    pub fn ref_count(&self, id: u16) -> u16 {
        self.slots
            .get(usize::from(id))
            .and_then(Option::as_ref)
            .map_or(0, |i| i.refs)
    }
    pub(crate) fn contains_slot(&self, id: u16) -> bool {
        self.slots.get(usize::from(id)).is_some_and(Option::is_some)
    }
    /// Iterate live IDs, values, and their reference counts.
    pub fn entries(&self) -> impl Iterator<Item = (u16, &T, u16)> {
        self.slots.iter().enumerate().filter_map(|(id, i)| {
            i.as_ref()
                .filter(|i| i.refs > 0)
                .map(|i| (id as u16, &i.value, i.refs))
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capacity_and_rehash() {
        let mut s = RefCountedSet::init(16).unwrap();
        let ids: Vec<_> = (0..12).map(|n| s.add(n).unwrap()).collect();
        assert_eq!(s.add(99), Err(AddError::OutOfMemory));
        s.release(ids[0]).unwrap();
        s.release(ids[1]).unwrap();
        assert_eq!(s.add(99), Err(AddError::NeedsRehash));
        assert_eq!(s.add_with_id(99, ids[0]).unwrap(), None);
        assert_eq!(s.count(), 11);
    }
    #[test]
    fn trailing_reuse_and_dedup() {
        let mut s = RefCountedSet::init(4).unwrap();
        let a = s.add(1).unwrap();
        let b = s.add(2).unwrap();
        assert_eq!(s.add(1).unwrap(), a);
        s.release(b).unwrap();
        assert_eq!(s.add(3).unwrap(), b);
        assert_eq!(s.ref_count(a), 2);
        assert_eq!(s.r#use(0), Err(AddError::InvalidId));
    }
    #[test]
    fn probe_ceiling_precedes_capacity_and_dead_id_reuse() {
        let mut set = RefCountedSet::init_with_hasher(128, |_: &u64| 0).unwrap();
        for n in 0..32 {
            assert_eq!(set.add(n).unwrap(), n as u16 + 1);
        }
        assert!(set.count() < set.capacity());
        assert_eq!(set.add(32), Err(AddError::OutOfMemory));
        // A live duplicate still succeeds before the probe-limit check.
        assert_eq!(set.add(0), Ok(1));
        set.release(2).unwrap();
        assert_eq!(set.add_with_id(100, 2), Err(AddError::OutOfMemory));
        // Trimming the trailing ID reduces the longest probe before insertion.
        set.release(32).unwrap();
        assert_eq!(set.add(100), Ok(2));
        assert_eq!(set.lookup(&100), Some(2));
        assert_eq!(set.lookup(&31), None);
    }

    #[test]
    fn robin_hood_displacement_reuses_encountered_dead_id() {
        let mut set = RefCountedSet::init_with_hasher(16, |v: &u64| *v).unwrap();
        assert_eq!(set.add(0), Ok(1));
        assert_eq!(set.add(1), Ok(2));
        assert_eq!(set.add(16), Ok(3));
        assert_eq!(&set.table[..3], &[1, 3, 2]);
        set.release(2).unwrap();
        assert_eq!(set.add(32), Ok(2));
        assert_eq!(set.next_id, 4);
        for (value, id) in [(0, 1), (16, 3), (32, 2)] {
            assert_eq!(set.lookup(&value), Some(id));
        }
        assert_eq!(set.lookup(&1), None);
    }

    #[test]
    fn preferred_id_deletion_backshifts_probe_sequence() {
        let mut set = RefCountedSet::init_with_hasher(16, |_: &u64| 0).unwrap();
        for n in 0..4 {
            set.add(n).unwrap();
        }
        set.release(2).unwrap();
        assert_eq!(set.add_with_id(99, 2), Ok(None));
        assert_eq!(&set.table[..4], &[1, 3, 4, 2]);
        assert_eq!(set.max_psl, 3);
        for (value, id) in [(0, 1), (2, 3), (3, 4), (99, 2)] {
            assert_eq!(set.lookup(&value), Some(id));
        }
    }

    #[test]
    fn dead_preferred_id_upsert_preserves_reference_count_semantics() {
        let mut set = RefCountedSet::init_with_hasher(16, |v: &u64| *v).unwrap();
        let dead = set.add(1).unwrap();
        let live = set.add(2).unwrap();
        set.release(dead).unwrap();
        assert_eq!(set.add_with_id(2, dead), Ok(Some(live)));
        assert_eq!(set.ref_count(live), 2);
        assert!(!set.contains_slot(dead));
        // Zig counts this upsert as living even when it finds a live value.
        assert_eq!(set.count(), 2);
    }

    #[test]
    fn clear_retains_storage_and_hash_context() {
        let mut set = RefCountedSet::init_with_hasher(16, |_: &u64| 7).unwrap();
        set.add(123).unwrap();
        let slots = set.slots.as_ptr();
        let table = set.table.as_ptr();
        set.clear();
        assert_eq!(set.slots.as_ptr(), slots);
        assert_eq!(set.table.as_ptr(), table);
        assert_eq!(set.count(), 0);
        assert_eq!(set.add(456), Ok(1));
        assert_eq!(set.table[7], 1);
    }
}
