//! `Helpers/Extensions/NSMenu+Extension.swift`.

use std::collections::HashSet;

use objc2::runtime::Sel;
use objc2_app_kit::{NSMenu, NSMenuItem, NSUserInterfaceItemIdentification};

pub trait NSMenuExt {
    /// Inserts a menu item after an existing item with the specified action
    /// selector.
    ///
    /// If an item with the same identifier already exists, it is removed first
    /// to avoid duplicates. This is useful when menus are cached and reused
    /// across different targets.
    ///
    /// Returns the index where the item was inserted, or `None` if the action
    /// was not found and the item was not inserted.
    fn insert_item_after(&self, item: &NSMenuItem, action: Sel) -> Option<usize>;

    /// Removes all menu items whose identifier is in the given set.
    fn remove_items_with_identifiers(&self, identifiers: &HashSet<String>);
}

impl NSMenuExt for NSMenu {
    fn insert_item_after(&self, item: &NSMenuItem, action: Sel) -> Option<usize> {
        if let Some(identifier) = item.identifier() {
            let existing = self
                .itemArray()
                .iter()
                .find(|i| i.identifier().is_some_and(|id| id == identifier));
            if let Some(existing) = existing {
                self.removeItem(&existing);
            }
        }

        let idx = self
            .itemArray()
            .iter()
            .position(|i| i.action() == Some(action))?;

        let insertion_index = idx + 1;
        self.insertItem_atIndex(item, insertion_index as isize);
        Some(insertion_index)
    }

    fn remove_items_with_identifiers(&self, identifiers: &HashSet<String>) {
        let items = self.itemArray();
        for (index, item) in items.to_vec().into_iter().enumerate().rev() {
            let Some(identifier) = item.identifier() else {
                continue;
            };
            if identifiers.contains(&identifier.to_string()) {
                self.removeItemAtIndex(index as isize);
            }
        }
    }
}
