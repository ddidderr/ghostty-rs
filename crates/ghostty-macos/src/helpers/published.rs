//! The Rust equivalent of a SwiftUI `@Published` property / Combine
//! `CurrentValueSubject`: a value cell that remembers its latest value and
//! notifies subscribers after each change with the new value (Combine's
//! `$property` publisher emits in `willSet`, but the Swift call sites only use
//! the value they are handed, which is the new one).
//!
//! Single-threaded (main thread), like everything that owns AppKit objects.
//! Dropping the returned [`Subscription`] cancels it (an `AnyCancellable`).

use std::{cell::RefCell, rc::Rc};

type Callback<T> = Rc<dyn Fn(&T)>;

struct Inner<T> {
    value: T,
    next_id: u64,
    subscribers: Vec<(u64, Callback<T>)>,
}

/// `@Published var value: T`. Cloning shares the same cell.
pub struct Published<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Clone for Published<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Cancels the subscription when dropped.
#[must_use = "dropping the subscription cancels it"]
pub struct Subscription {
    cancel: Option<Box<dyn FnOnce()>>,
}

impl Subscription {
    /// Keep the subscription for the life of the cell.
    pub fn forget(mut self) {
        self.cancel = None;
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

impl<T: Clone + 'static> Published<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner {
                value,
                next_id: 0,
                subscribers: Vec::new(),
            })),
        }
    }

    /// The current value.
    #[must_use]
    pub fn get(&self) -> T {
        self.inner.borrow().value.clone()
    }

    /// Assign a new value and notify every subscriber with it.
    pub fn set(&self, value: T) {
        let subscribers: Vec<Callback<T>> = {
            let mut inner = self.inner.borrow_mut();
            inner.value = value.clone();
            inner.subscribers.iter().map(|(_, cb)| cb.clone()).collect()
        };
        for cb in subscribers {
            cb(&value);
        }
    }

    /// Mutate in place (`value.field = x` on a `@Published` struct) and notify.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let mut value = self.get();
        f(&mut value);
        self.set(value);
    }

    /// Subscribe (`$property.sink`). Like Combine's `@Published` publisher the
    /// callback is first invoked with the current value.
    pub fn sink(&self, f: impl Fn(&T) + 'static) -> Subscription {
        let cb: Callback<T> = Rc::new(f);
        let (id, current) = {
            let mut inner = self.inner.borrow_mut();
            let id = inner.next_id;
            inner.next_id += 1;
            inner.subscribers.push((id, cb.clone()));
            (id, inner.value.clone())
        };
        cb(&current);
        let weak = Rc::downgrade(&self.inner);
        Subscription {
            cancel: Some(Box::new(move || {
                if let Some(inner) = weak.upgrade() {
                    inner.borrow_mut().subscribers.retain(|(i, _)| *i != id);
                }
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn sink_gets_current_then_changes() {
        let p = Published::new(1);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let _s = p.sink({
            let seen = seen.clone();
            move |v| seen.borrow_mut().push(*v)
        });
        p.set(2);
        p.update(|v| *v += 5);
        assert_eq!(*seen.borrow(), vec![1, 2, 7]);
        assert_eq!(p.get(), 7);
    }

    #[test]
    fn cancel_on_drop() {
        let p = Published::new(0);
        let n = Rc::new(Cell::new(0));
        let s = p.sink({
            let n = n.clone();
            move |_| n.set(n.get() + 1)
        });
        drop(s);
        p.set(1);
        assert_eq!(n.get(), 1);
    }
}
