//! Bounded, deduplicated values with stable nonzero identifiers.
use std::{collections::HashMap, hash::Hash};
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
}
/// A slot array and deduplication map. ID zero is always reserved.
#[derive(Debug, Clone)]
pub struct RefCountedSet<T> {
    slots: Vec<Option<Item<T>>>,
    lookup: HashMap<T, u16>,
    next_id: usize,
    living: usize,
}
impl<T: Eq + Hash + Clone> RefCountedSet<T> {
    /// Construct with Zig's 13/16 load factor and power-of-two rounding.
    pub fn init(cap: usize) -> Result<Self, AddError> {
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
        Ok(Self {
            slots,
            lookup: HashMap::new(),
            next_id: 1,
            living: 0,
        })
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
        self.lookup.get(value).copied()
    }
    /// Add or retain a value, trimming unused trailing IDs first.
    pub fn add(&mut self, value: T) -> Result<u16, AddError> {
        while self.next_id > 1
            && self.slots[self.next_id - 1]
                .as_ref()
                .is_none_or(|v| v.refs == 0)
        {
            self.next_id -= 1;
            self.slots[self.next_id] = None;
        }
        if let Some(id) = self.lookup(&value) {
            self.r#use(id)?;
            return Ok(id);
        }
        if self.next_id >= self.slots.len() {
            return Err(if self.living < self.slots.len() * 9 / 10 {
                AddError::NeedsRehash
            } else {
                AddError::OutOfMemory
            });
        }
        self.lookup
            .try_reserve(1)
            .map_err(|_| AddError::OutOfMemory)?;
        let id = self.next_id as u16;
        self.lookup.insert(value.clone(), id);
        self.slots[self.next_id] = Some(Item { value, refs: 1 });
        self.next_id += 1;
        self.living += 1;
        Ok(id)
    }
    /// Prefer an already allocated ID; return None when that ID was retained.
    pub fn add_with_id(&mut self, value: T, id: u16) -> Result<Option<u16>, AddError> {
        if id == 0 {
            return Err(AddError::InvalidId);
        }
        if usize::from(id) < self.next_id {
            if self.get(id) == Some(&value) {
                self.r#use(id)?;
                return Ok(None);
            }
            if self.ref_count(id) == 0 {
                if let Some(other) = self.lookup(&value) {
                    self.r#use(other)?;
                    return Ok(Some(other));
                }
                self.lookup
                    .try_reserve(1)
                    .map_err(|_| AddError::OutOfMemory)?;
                self.lookup.insert(value.clone(), id);
                self.slots[usize::from(id)] = Some(Item { value, refs: 1 });
                self.living += 1;
                return Ok(None);
            }
        }
        self.add(value).map(Some)
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
            self.lookup.remove(&item.value);
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
}
