//! A collection of Vecs with methods for bulk operations
//! (`array_list_collection.zig`).

/// A collection of Vecs with methods for bulk operations.
#[derive(Debug, Clone, Default)]
pub struct ArrayListCollection<T> {
    /// The lists that belong to this collection.
    pub lists: Vec<Vec<T>>,
}

impl<T> ArrayListCollection<T> {
    /// The collection will be initialized with empty lists.
    #[must_use]
    pub fn new(list_count: usize, initial_capacity: usize) -> Self {
        Self {
            lists: (0..list_count)
                .map(|_| Vec::with_capacity(initial_capacity))
                .collect(),
        }
    }

    /// Clear all lists in the collection, retaining capacity.
    pub fn reset(&mut self) {
        for list in &mut self.lists {
            list.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_and_reset() {
        let mut c: ArrayListCollection<u32> = ArrayListCollection::new(3, 8);
        assert_eq!(c.lists.len(), 3);
        for l in &c.lists {
            assert!(l.is_empty());
            assert!(l.capacity() >= 8);
        }
        c.lists[1].extend([1, 2, 3]);
        c.reset();
        assert!(c.lists.iter().all(Vec::is_empty));
        assert!(c.lists[1].capacity() >= 8);
    }
}
