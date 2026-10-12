//! `Helpers/Extensions/UndoManager+Extension.swift`.

use objc2_foundation::NSUndoManager;

pub trait UndoManagerExt {
    /// A Boolean value that indicates whether the undo manager is currently
    /// performing either an undo or redo operation.
    fn is_undoing_or_redoing(&self) -> bool;

    /// Temporarily disables undo registration while executing the provided
    /// handler, so operations aren't recorded in the undo stack.
    fn disable_undo_registration_during(&self, handler: impl FnOnce());
}

impl UndoManagerExt for NSUndoManager {
    fn is_undoing_or_redoing(&self) -> bool {
        self.isUndoing() || self.isRedoing()
    }

    fn disable_undo_registration_during(&self, handler: impl FnOnce()) {
        self.disableUndoRegistration();
        handler();
        self.enableUndoRegistration();
    }
}
