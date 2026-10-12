//! The in-process equivalent of Foundation's `NotificationCenter.default` for
//! Ghostty's *own* notifications (`Notification.Name.ghosttyConfigDidChange`,
//! `Ghostty.Notification.ghosttyNewWindow`, ...).
//!
//! The Swift app posts `Foundation.Notification`s whose `object` is the sender
//! (a surface view, a controller) and whose `userInfo` is an untyped
//! dictionary of Swift values. Both ends are Rust here, so the payload keeps
//! its Rust type: [`UserInfo`] is a string-keyed map of `Rc<dyn Any>` and the
//! sender is a [`Sender`] (identity + a type-erased strong handle that
//! [`Sender::downcast_ref`] turns back into e.g. `Retained<SurfaceView>`).
//!
//! Semantics follow `NotificationCenter`:
//! * delivery is synchronous, on the posting thread (always the main thread:
//!   nothing here is `Send`);
//! * an observer registered with a sender only receives notifications posted
//!   with that same sender; one registered without a sender receives every
//!   notification of that name;
//! * observers registered or removed while a notification is being delivered do
//!   not disturb the delivery (the observer list is snapshotted; an observer
//!   removed meanwhile is skipped);
//! * dropping the [`ObserverToken`] removes the observer (the equivalent of
//!   `removeObserver`, and of storing a Combine `AnyCancellable`).
//!
//! AppKit's own notifications (`NSWindow.didBecomeKeyNotification`, ...) go
//! through `helpers::extensions::ns_notification::observe` instead.

use std::{any::Any, cell::RefCell, collections::HashMap, fmt, rc::Rc};

/// A notification name (`Notification.Name`); the string is the same as in the
/// Swift app (`"com.mitchellh.ghostty.configDidChange"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Name(pub &'static str);

impl Name {
    #[must_use]
    pub const fn raw_value(self) -> &'static str {
        self.0
    }
}

/// The sender of a notification (`Notification.object`).
#[derive(Clone)]
pub struct Sender {
    id: usize,
    handle: Rc<dyn Any>,
}

impl Sender {
    /// `id` is the object's identity (the address of the Objective-C object),
    /// `handle` keeps it alive and lets receivers downcast it.
    pub fn new<T: 'static>(id: usize, handle: T) -> Self {
        Self {
            id,
            handle: Rc::new(handle),
        }
    }

    #[must_use]
    pub fn id(&self) -> usize {
        self.id
    }

    /// The sender as `T` (the type passed to [`Sender::new`]).
    #[must_use]
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.handle.downcast_ref::<T>()
    }
}

impl fmt::Debug for Sender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sender({:#x})", self.id)
    }
}

/// `Notification.userInfo`.
#[derive(Clone, Default)]
pub struct UserInfo(HashMap<&'static str, Rc<dyn Any>>);

impl UserInfo {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder style insert.
    #[must_use]
    pub fn with<T: 'static>(mut self, key: &'static str, value: T) -> Self {
        self.insert(key, value);
        self
    }

    pub fn insert<T: 'static>(&mut self, key: &'static str, value: T) {
        self.0.insert(key, Rc::new(value));
    }

    /// `userInfo?[key] as? T`.
    #[must_use]
    pub fn get<T: 'static>(&self, key: &str) -> Option<&T> {
        self.0.get(key).and_then(|v| v.downcast_ref::<T>())
    }

    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `Foundation.Notification`.
#[derive(Clone)]
pub struct Notification {
    pub name: Name,
    pub object: Option<Sender>,
    pub user_info: UserInfo,
}

/// An observer callback.
pub type Handler = Rc<dyn Fn(&Notification)>;

struct Observer {
    id: u64,
    name: Name,
    object: Option<usize>,
    handler: Handler,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    observers: Vec<Observer>,
}

/// A notification center. Use [`NotificationCenter::default_center`] for the
/// process-wide one (thread-local, i.e. main thread).
#[derive(Clone, Default)]
pub struct NotificationCenter {
    inner: Rc<RefCell<Inner>>,
}

thread_local! {
    static DEFAULT: NotificationCenter = NotificationCenter::default();
}

/// Removes its observer when dropped.
#[must_use = "dropping the token removes the observer"]
pub struct ObserverToken {
    center: std::rc::Weak<RefCell<Inner>>,
    id: u64,
}

impl ObserverToken {
    /// Keep the observer registered for the life of the process
    /// (`addObserver` without a matching `removeObserver`).
    pub fn forget(self) {
        std::mem::forget(self);
    }
}

impl Drop for ObserverToken {
    fn drop(&mut self) {
        if let Some(inner) = self.center.upgrade() {
            inner.borrow_mut().observers.retain(|o| o.id != self.id);
        }
    }
}

impl NotificationCenter {
    /// `NotificationCenter.default`.
    #[must_use]
    pub fn default_center() -> Self {
        DEFAULT.with(Clone::clone)
    }

    /// `addObserver(forName:object:queue:using:)` (always delivered inline).
    pub fn add_observer(
        &self,
        name: Name,
        object: Option<&Sender>,
        handler: impl Fn(&Notification) + 'static,
    ) -> ObserverToken {
        let mut inner = self.inner.borrow_mut();
        let id = inner.next_id;
        inner.next_id += 1;
        inner.observers.push(Observer {
            id,
            name,
            object: object.map(Sender::id),
            handler: Rc::new(handler),
        });
        ObserverToken {
            center: Rc::downgrade(&self.inner),
            id,
        }
    }

    /// `post(name:object:userInfo:)`.
    pub fn post(&self, name: Name, object: Option<Sender>, user_info: UserInfo) {
        self.post_notification(&Notification {
            name,
            object,
            user_info,
        });
    }

    pub fn post_notification(&self, n: &Notification) {
        let snapshot: Vec<(u64, Handler)> = self
            .inner
            .borrow()
            .observers
            .iter()
            .filter(|o| {
                o.name == n.name
                    && match (o.object, &n.object) {
                        (None, _) => true,
                        (Some(want), Some(got)) => want == got.id(),
                        (Some(_), None) => false,
                    }
            })
            .map(|o| (o.id, o.handler.clone()))
            .collect();
        for (id, handler) in snapshot {
            // Skip observers removed by an earlier handler of this delivery.
            if self.inner.borrow().observers.iter().any(|o| o.id == id) {
                handler(n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    const A: Name = Name("test.a");
    const B: Name = Name("test.b");

    #[test]
    fn delivers_by_name_and_sender() {
        let nc = NotificationCenter::default();
        let any = Rc::new(Cell::new(0));
        let only = Rc::new(Cell::new(0));
        let s1 = Sender::new(1, 1u32);
        let s2 = Sender::new(2, 2u32);
        let _t1 = nc.add_observer(A, None, {
            let any = any.clone();
            move |_| any.set(any.get() + 1)
        });
        let _t2 = nc.add_observer(A, Some(&s1), {
            let only = only.clone();
            move |n| {
                assert_eq!(
                    n.object.as_ref().and_then(|o| o.downcast_ref::<u32>()),
                    Some(&1)
                );
                only.set(only.get() + 1);
            }
        });
        nc.post(A, Some(s1.clone()), UserInfo::new());
        nc.post(A, Some(s2), UserInfo::new());
        nc.post(A, None, UserInfo::new());
        nc.post(B, Some(s1), UserInfo::new());
        assert_eq!(any.get(), 3);
        assert_eq!(only.get(), 1);
    }

    #[test]
    fn dropping_token_removes_observer() {
        let nc = NotificationCenter::default();
        let hits = Rc::new(Cell::new(0));
        let t = nc.add_observer(A, None, {
            let hits = hits.clone();
            move |_| hits.set(hits.get() + 1)
        });
        nc.post(A, None, UserInfo::new());
        drop(t);
        nc.post(A, None, UserInfo::new());
        assert_eq!(hits.get(), 1);
    }

    #[test]
    fn user_info_is_typed() {
        let info = UserInfo::new().with("k", String::from("v")).with("n", 5u64);
        assert_eq!(info.get::<String>("k").map(String::as_str), Some("v"));
        assert_eq!(info.get::<u64>("n"), Some(&5));
        assert!(info.get::<u32>("n").is_none());
        assert!(info.get::<u64>("missing").is_none());
    }

    #[test]
    fn handler_may_remove_a_later_observer_during_delivery() {
        let nc = NotificationCenter::default();
        let hits = Rc::new(Cell::new(0));
        let victim: Rc<RefCell<Option<ObserverToken>>> = Rc::new(RefCell::new(None));
        let _first = nc.add_observer(A, None, {
            let victim = victim.clone();
            move |_| drop(victim.borrow_mut().take())
        });
        *victim.borrow_mut() = Some(nc.add_observer(A, None, {
            let hits = hits.clone();
            move |_| hits.set(hits.get() + 1)
        }));
        nc.post(A, None, UserInfo::new());
        assert_eq!(hits.get(), 0);
    }
}
