//! `Helpers/ExpiringUndoManager.swift`.
//!
//! An `NSUndoManager` subclass that supports registering undo operations that
//! automatically expire after a specified duration.
//!
//! When an undo operation expires, it is automatically removed from the undo
//! stack and cannot be invoked.
//!
//! ```ignore
//! let undo_manager = ExpiringUndoManager::new(mtm);
//! undo_manager.register_undo(&my_object, Duration::from_secs(30), |target| {
//!     // Undo operation that expires after 30 seconds
//! });
//! ```
//!
//! Deviation: Swift's `ExpiringTarget.deinit` calls `expire()`, which messages
//! the undo manager with `self`; that is not possible while the Objective-C
//! object is being deallocated, so the drop only invalidates the timer. The
//! manager owns its targets (strongly) until they expire or are removed, so a
//! target is never deallocated while registered.

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code, clippy::borrow_as_ptr, clippy::ref_as_ptr)]

use std::{cell::RefCell, ptr::NonNull, time::Duration};

use block2::RcBlock;
use objc2::{
    AnyThread,
    DefinedClass,
    MainThreadOnly,
    define_class,
    msg_send,
    rc::{Retained, Weak},
    runtime::AnyObject,
};
use objc2_foundation::{NSObject, NSTimer, NSUndoManager};

use super::extensions::duration::DurationExt;

/// The instance variables of an [`ExpiringTarget`].
pub struct TargetIvars {
    /// The actual target object for the undo operation, held weakly to avoid
    /// retain cycles.
    target: RefCell<Option<Weak<AnyObject>>>,
    /// Timer that triggers expiration after the specified duration.
    timer: RefCell<Option<Retained<NSTimer>>>,
    /// The undo manager from which to remove actions when this target expires.
    undo_manager: RefCell<Option<Weak<NSUndoManager>>>,
}

impl Drop for TargetIvars {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.get_mut().take() {
            timer.invalidate();
        }
    }
}

define_class!(
    /// A target object for ExpiringUndoManager that removes itself from the
    /// undo manager after it expires.
    ///
    /// This class acts as a proxy for the real target object in undo
    /// operations. It holds a weak reference to the actual target and
    /// automatically removes all associated undo operations when either the
    /// specified duration expires or `expire()` is called manually.
    // SAFETY: NSObject has no subclassing requirements; the ivars' Drop is the
    // only destructor logic.
    #[unsafe(super(NSObject))]
    #[name = "GhosttyExpiringTarget"]
    #[ivars = TargetIvars]
    struct ExpiringTarget;
);

impl ExpiringTarget {
    fn new(target: &AnyObject, duration: Duration, undo_manager: &NSUndoManager) -> Retained<Self> {
        let this = Self::alloc().set_ivars(TargetIvars {
            target: RefCell::new(Some(Weak::new(target))),
            timer: RefCell::new(None),
            undo_manager: RefCell::new(Some(Weak::new(undo_manager))),
        });
        // SAFETY: NSObject's init is the designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };

        let weak = Weak::new(&*this);
        // SAFETY: the block is a valid `void (^)(NSTimer *)`.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(
                duration.time_interval(),
                false,
                &RcBlock::new(move |_timer: NonNull<NSTimer>| {
                    if let Some(this) = weak.load() {
                        this.expire();
                    }
                }),
            )
        };
        *this.ivars().timer.borrow_mut() = Some(timer);
        this
    }

    /// The actual target object, if it is still alive.
    fn target(&self) -> Option<Retained<AnyObject>> {
        self.ivars().target.borrow().as_ref().and_then(Weak::load)
    }

    /// Manually expires the target, removing all associated undo actions and
    /// invalidating the timer.
    ///
    /// This method is called automatically when the timer fires, but can also
    /// be called manually to expire the target before the timer duration has
    /// elapsed.
    fn expire(&self) {
        *self.ivars().target.borrow_mut() = None;
        let manager = self
            .ivars()
            .undo_manager
            .borrow()
            .as_ref()
            .and_then(Weak::load);
        if let Some(manager) = manager {
            // SAFETY: `self` is a valid target object.
            #[allow(unsafe_code)]
            unsafe {
                manager.removeAllActionsWithTarget(self);
            }
        }
        if let Some(timer) = self.ivars().timer.borrow_mut().take() {
            timer.invalidate();
        }
    }
}

/// Instance variables of the manager.
#[derive(Default)]
pub struct ManagerIvars {
    /// The set of expiring targets so we can properly clean them up when
    /// removeAllActions is called with the real target.
    expiring_targets: RefCell<Vec<Retained<ExpiringTarget>>>,
}

define_class!(
    /// An UndoManager subclass that supports registering undo operations that
    /// automatically expire after a specified duration.
    // SAFETY: NSUndoManager may be subclassed; we override only documented
    // methods and call super; no Drop.
    #[unsafe(super(NSUndoManager))]
    #[thread_kind = MainThreadOnly]
    #[name = "GhosttyExpiringUndoManager"]
    #[ivars = ManagerIvars]
    pub struct ExpiringUndoManager;

    impl ExpiringUndoManager {
        /// Removes all undo and redo operations from the undo manager.
        ///
        /// This override ensures that all expiring targets are also cleared
        /// when the undo manager is reset.
        #[unsafe(method(removeAllActions))]
        fn remove_all_actions(&self) {
            // SAFETY: calling the superclass implementation.
            let _: () = unsafe { msg_send![super(self), removeAllActions] };
            self.ivars().expiring_targets.borrow_mut().clear();
        }

        /// Removes all undo and redo operations involving the specified target.
        ///
        /// This override ensures that when actions are removed for a target,
        /// any associated expiring targets are also properly cleaned up.
        #[unsafe(method(removeAllActionsWithTarget:))]
        fn remove_all_actions_with_target(&self, target: &AnyObject) {
            // Call super to handle standard removal
            // SAFETY: calling the superclass implementation.
            let _: () = unsafe { msg_send![super(self), removeAllActionsWithTarget: target] };

            let target_ptr: *const AnyObject = target;
            // If the target is an expiring target, remove it.
            let is_expiring = self
                .ivars()
                .expiring_targets
                .borrow()
                .iter()
                .any(|e| std::ptr::eq((&**e as *const ExpiringTarget).cast::<AnyObject>(), target_ptr));
            if is_expiring {
                self.ivars()
                    .expiring_targets
                    .borrow_mut()
                    .retain(|e| !std::ptr::eq((&**e as *const ExpiringTarget).cast::<AnyObject>(), target_ptr));
            } else {
                // Find and remove any ExpiringTarget instances that wrap this target.
                let matching: Vec<Retained<ExpiringTarget>> = self
                    .ivars()
                    .expiring_targets
                    .borrow()
                    .iter()
                    .filter(|e| e.target().is_none_or(|t| std::ptr::eq(&*t as *const AnyObject, target_ptr)))
                    .cloned()
                    .collect();
                for e in matching {
                    // Technically they'll always expire when they get
                    // deinitialized but we want to make sure it happens right now.
                    e.expire();
                    self.ivars().expiring_targets.borrow_mut().retain(|x| !std::ptr::eq(Retained::as_ptr(x), Retained::as_ptr(&e)));
                }
            }
        }
    }
);

impl ExpiringUndoManager {
    #[must_use]
    pub fn new(mtm: objc2::MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ManagerIvars::default());
        // SAFETY: NSUndoManager's init is the designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// Registers an undo operation that automatically expires after the
    /// specified duration.
    ///
    /// - `target`: the target object for the undo operation. The undo
    ///   operation will be removed if this object is deallocated before the
    ///   operation is invoked.
    /// - `duration`: the duration after which the undo operation should expire
    ///   and be removed from the undo stack.
    /// - `handler`: the closure to execute when the undo operation is invoked.
    ///   It receives the target object as its parameter.
    pub fn register_undo(
        &self,
        target: &AnyObject,
        expires_after: Duration,
        handler: impl Fn(&AnyObject) + 'static,
    ) {
        // Ignore instantly expiring undos
        if expires_after.time_interval() <= 0.0 {
            return;
        }

        // Ignore when undo registration is disabled. UndoManager still lets
        // registration happen then cancels later but I was seeing some
        // weird behavior with this so let's just guard on it.
        if !self.isUndoRegistrationEnabled() {
            return;
        }

        let expiring_target = ExpiringTarget::new(target, expires_after, self);
        self.ivars()
            .expiring_targets
            .borrow_mut()
            .push(expiring_target.clone());

        let weak_self = Weak::new(self);
        let block = RcBlock::new(move |raw: NonNull<AnyObject>| {
            // SAFETY: the undo manager passes back the registered target.
            #[allow(unsafe_code)]
            let raw = unsafe { raw.as_ref() };
            if let Some(this) = weak_self.load() {
                this.ivars().expiring_targets.borrow_mut().retain(|e| {
                    !std::ptr::eq(
                        (&**e as *const ExpiringTarget).cast::<AnyObject>(),
                        raw as *const AnyObject,
                    )
                });
            }
            // SAFETY: `raw` is the ExpiringTarget we registered.
            #[allow(unsafe_code)]
            let expiring: &ExpiringTarget =
                unsafe { &*(raw as *const AnyObject).cast::<ExpiringTarget>() };
            let Some(target) = expiring.target() else {
                return;
            };
            handler(&target);
        });
        // SAFETY: the target is an object and the handler block matches
        // `void (^)(id)`.
        #[allow(unsafe_code)]
        unsafe {
            self.registerUndoWithTarget_handler(&expiring_target, &block);
        }
    }
}
