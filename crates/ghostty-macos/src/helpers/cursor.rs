//! `Helpers/Cursor.swift`.

// The pre-macOS-15 fallbacks are deprecated but used on older systems, like the Swift.
#![allow(deprecated)]

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2_app_kit::{NSCursor, NSHorizontalDirections, NSVerticalDirections};

use super::{cursor_counter::CursorCounter, extensions::macos_at_least};

thread_local! {
    static COUNTER: RefCell<CursorCounter> = const { RefCell::new(CursorCounter::new()) };
}

/// This helps manage the stateful nature of NSCursor hiding and unhiding.
pub struct Cursor;

impl Cursor {
    #[must_use]
    pub fn is_visible() -> bool {
        COUNTER.with(|c| c.borrow().is_visible())
    }

    pub fn hide() {
        COUNTER.with(|c| c.borrow_mut().hide());
        NSCursor::hide();
    }

    /// Unhide the cursor. Returns true if the cursor was previously hidden.
    pub fn unhide() -> bool {
        // Its always safe to call unhide when the counter is zero because it
        // won't go negative.
        NSCursor::unhide();
        COUNTER.with(|c| c.borrow_mut().unhide())
    }

    pub fn unhide_completely() -> u64 {
        let counter = COUNTER.with(|c| c.borrow().count());
        for _ in 0..counter {
            let was_hidden = Self::unhide();
            debug_assert!(was_hidden);
        }
        debug_assert_eq!(COUNTER.with(|c| c.borrow().count()), 0);
        counter
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Default,
    GrabIdle,
    GrabActive,
    HorizontalText,
    VerticalText,
    Link,
    ResizeLeft,
    ResizeRight,
    ResizeUp,
    ResizeDown,
    ResizeUpDown,
    ResizeLeftRight,
    ContextMenu,
    Crosshair,
    OperationNotAllowed,
}

impl CursorStyle {
    #[must_use]
    pub fn cursor(self) -> Retained<NSCursor> {
        // macOS 15 introduced the directional resize cursors.
        let new_resize = macos_at_least(15);
        match self {
            Self::Default => NSCursor::arrowCursor(),
            Self::GrabIdle => NSCursor::openHandCursor(),
            Self::GrabActive => NSCursor::closedHandCursor(),
            Self::HorizontalText => NSCursor::IBeamCursor(),
            Self::VerticalText => NSCursor::IBeamCursorForVerticalLayout(),
            Self::Link => NSCursor::pointingHandCursor(),
            Self::ResizeLeft if new_resize => {
                NSCursor::columnResizeCursorInDirections(NSHorizontalDirections::Left)
            }
            Self::ResizeLeft => NSCursor::resizeLeftCursor(),
            Self::ResizeRight if new_resize => {
                NSCursor::columnResizeCursorInDirections(NSHorizontalDirections::Right)
            }
            Self::ResizeRight => NSCursor::resizeRightCursor(),
            Self::ResizeUp if new_resize => {
                NSCursor::rowResizeCursorInDirections(NSVerticalDirections::Up)
            }
            Self::ResizeUp => NSCursor::resizeUpCursor(),
            Self::ResizeDown if new_resize => {
                NSCursor::rowResizeCursorInDirections(NSVerticalDirections::Down)
            }
            Self::ResizeDown => NSCursor::resizeDownCursor(),
            Self::ResizeUpDown if new_resize => NSCursor::rowResizeCursor(),
            Self::ResizeUpDown => NSCursor::resizeUpDownCursor(),
            Self::ResizeLeftRight if new_resize => NSCursor::columnResizeCursor(),
            Self::ResizeLeftRight => NSCursor::resizeLeftRightCursor(),
            Self::ContextMenu => NSCursor::contextualMenuCursor(),
            Self::Crosshair => NSCursor::crosshairCursor(),
            Self::OperationNotAllowed => NSCursor::operationNotAllowedCursor(),
        }
    }
}
