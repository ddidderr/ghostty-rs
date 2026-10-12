//! `Helpers/AnySortKey.swift`.

use std::{any::Any, cmp::Ordering, rc::Rc};

/// Type-erased wrapper for any Comparable type to use as a sort key.
///
/// Like the Swift version, keys of different underlying types compare as
/// equal (`guard let l = lhs as? T, let r = rhs as? T else { return .orderedSame }`).
type Comparator = Rc<dyn Fn(&dyn Any, &dyn Any) -> Ordering>;

#[derive(Clone)]
pub struct AnySortKey {
    value: Rc<dyn Any>,
    comparator: Comparator,
}

impl AnySortKey {
    pub fn new<T: Ord + 'static>(value: T) -> Self {
        Self {
            value: Rc::new(value),
            comparator: Rc::new(|lhs, rhs| {
                let (Some(l), Some(r)) = (lhs.downcast_ref::<T>(), rhs.downcast_ref::<T>()) else {
                    return Ordering::Equal;
                };
                l.cmp(r)
            }),
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        (self.comparator)(self.value.as_ref(), other.value.as_ref())
    }
}

impl PartialEq for AnySortKey {
    fn eq(&self, other: &Self) -> bool {
        self.compare(other) == Ordering::Equal
    }
}

impl Eq for AnySortKey {}

impl PartialOrd for AnySortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AnySortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_same_type() {
        assert!(AnySortKey::new(1) < AnySortKey::new(2));
        assert!(AnySortKey::new("b".to_string()) > AnySortKey::new("a".to_string()));
        assert!(AnySortKey::new(3) == AnySortKey::new(3));
    }

    #[test]
    fn mixed_types_compare_equal() {
        assert!(AnySortKey::new(1) == AnySortKey::new("x".to_string()));
    }

    #[test]
    fn sorts_a_heterogeneous_vec_stably() {
        let mut v = [AnySortKey::new(3), AnySortKey::new(1), AnySortKey::new(2)];
        v.sort();
        assert!(v[0] == AnySortKey::new(1) && v[2] == AnySortKey::new(3));
    }
}
