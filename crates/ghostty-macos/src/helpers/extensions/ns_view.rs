//! `Helpers/Extensions/NSView+Extension.swift` (the SwiftUI `Image`
//! screenshot variant is unportable).

// SAFETY: every `unsafe` block in this module carries its own justification (FFI / objc2 calls).
#![allow(unsafe_code, clippy::borrow_as_ptr, clippy::ref_as_ptr)]

use std::fmt::Write as _;

use objc2::{Message, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSImage, NSResponder, NSUserInterfaceItemIdentification, NSView};
use objc2_foundation::NSString;

fn class_name(view: &NSView) -> String {
    view.class().name().to_string_lossy().into_owned()
}

pub trait NSViewExt {
    /// Returns true if this view is currently in the responder chain.
    fn is_in_responder_chain(&self) -> bool;
    /// Take a screenshot of just this view.
    fn screenshot(&self) -> Option<Retained<NSImage>>;
    /// Returns the absolute root view by walking up the superview chain.
    fn root_view(&self) -> Retained<NSView>;
    /// Checks if a view contains another view in its hierarchy.
    fn contains_view(&self, view: &NSView) -> bool;
    /// Checks if the view contains the given class in its hierarchy.
    fn contains_class_name(&self, name: &str) -> bool;
    /// Finds the superview with the given class name.
    fn first_superview_with_class_name(&self, name: &str) -> Option<Retained<NSView>>;
    /// Recursively finds and returns the first descendant view that has the given class name.
    fn first_descendant_with_class_name(&self, name: &str) -> Option<Retained<NSView>>;
    /// Recursively finds and returns descendant views that have the given class name.
    fn descendants_with_class_name(&self, name: &str) -> Vec<Retained<NSView>>;
    /// Recursively finds and returns the first descendant view that has the given identifier.
    fn first_descendant_with_id(&self, id: &str) -> Option<Retained<NSView>>;
    /// Finds and returns the first view with the given class name starting
    /// from the absolute root of the view hierarchy. This includes private
    /// views like title bar views.
    fn first_view_from_root_with_class_name(&self, name: &str) -> Option<Retained<NSView>>;
    /// Prints the view hierarchy from the root in a tree-like ASCII format.
    ///
    /// I need this because the "Capture View Hierarchy" was broken under some
    /// scenarios in Xcode 26 (FB17912569). But, I kept it around because it
    /// might be useful to print out the view hierarchy without halting the
    /// program.
    fn print_view_hierarchy(&self);
    /// Returns a string representation of the view hierarchy in a tree-like format.
    fn view_hierarchy_description(&self, indent: &str, is_last: bool) -> String;
}

impl NSViewExt for NSView {
    fn is_in_responder_chain(&self) -> bool {
        let mut responder: Option<Retained<NSResponder>> =
            self.window().and_then(|w| w.firstResponder());
        while let Some(current) = responder {
            if std::ptr::eq(
                (&*current as *const NSResponder).cast::<AnyObject>(),
                (self as *const NSView).cast::<AnyObject>(),
            ) {
                return true;
            }
            // SAFETY: plain getter on a live responder.
            responder = unsafe { current.nextResponder() };
        }
        false
    }

    fn screenshot(&self) -> Option<Retained<NSImage>> {
        use objc2::AnyThread;
        let bounds = self.bounds();
        let bitmap_rep = self.bitmapImageRepForCachingDisplayInRect(bounds)?;
        self.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap_rep);
        let image = NSImage::initWithSize(NSImage::alloc(), bounds.size);
        image.addRepresentation(&bitmap_rep);
        Some(image)
    }

    fn root_view(&self) -> Retained<NSView> {
        let mut root = self.retain();
        // SAFETY: plain getter on a live view.
        while let Some(superview) = unsafe { root.superview() } {
            root = superview;
        }
        root
    }

    fn contains_view(&self, view: &NSView) -> bool {
        if std::ptr::eq(self, view) {
            return true;
        }
        self.subviews().iter().any(|s| s.contains_view(view))
    }

    fn contains_class_name(&self, name: &str) -> bool {
        if class_name(self) == name {
            return true;
        }
        self.subviews().iter().any(|s| s.contains_class_name(name))
    }

    fn first_superview_with_class_name(&self, name: &str) -> Option<Retained<NSView>> {
        // SAFETY: plain getter on a live view.
        let superview = unsafe { self.superview() }?;
        if class_name(&superview) == name {
            return Some(superview);
        }
        superview.first_superview_with_class_name(name)
    }

    fn first_descendant_with_class_name(&self, name: &str) -> Option<Retained<NSView>> {
        for subview in self.subviews() {
            if class_name(&subview) == name {
                return Some(subview);
            } else if let Some(found) = subview.first_descendant_with_class_name(name) {
                return Some(found);
            }
        }
        None
    }

    fn descendants_with_class_name(&self, name: &str) -> Vec<Retained<NSView>> {
        let mut result = Vec::new();
        for subview in self.subviews() {
            if class_name(&subview) == name {
                result.push(subview.clone());
            }
            result.extend(subview.descendants_with_class_name(name));
        }
        result
    }

    fn first_descendant_with_id(&self, id: &str) -> Option<Retained<NSView>> {
        let want = NSString::from_str(id);
        for subview in self.subviews() {
            if subview.identifier().is_some_and(|i| *i == *want) {
                return Some(subview);
            } else if let Some(found) = subview.first_descendant_with_id(id) {
                return Some(found);
            }
        }
        None
    }

    fn first_view_from_root_with_class_name(&self, name: &str) -> Option<Retained<NSView>> {
        let root = self.root_view();

        // Check if the root view itself matches
        if class_name(&root) == name {
            return Some(root);
        }

        // Otherwise search descendants
        root.first_descendant_with_class_name(name)
    }

    fn print_view_hierarchy(&self) {
        let root = self.root_view();
        println!("View Hierarchy from Root:");
        println!("{}", root.view_hierarchy_description("", true));
    }

    fn view_hierarchy_description(&self, indent: &str, is_last: bool) -> String {
        let mut result = String::new();

        // Add the tree branch characters
        result.push_str(indent);
        if !indent.is_empty() {
            result.push_str(if is_last { "└── " } else { "├── " });
        }

        // Add the class name and optional identifier
        result.push_str(&class_name(self));

        // Add identifier if present
        if let Some(identifier) = self.identifier() {
            let _ = write!(result, " (id: {identifier})");
        }

        // Add frame info
        let f = self.frame();
        let _ = write!(
            result,
            " [frame: (({}, {}), ({}, {}))]",
            f.origin.x, f.origin.y, f.size.width, f.size.height
        );

        // Add visual properties
        let mut properties: Vec<String> = Vec::new();
        if self.isHidden() {
            properties.push("hidden".into());
        }
        properties.push(
            if self.isOpaque() {
                "opaque"
            } else {
                "transparent"
            }
            .into(),
        );

        if self.wantsLayer() {
            properties.push("layer-backed".into());
            if let Some(bg) = self.layer().and_then(|l| l.backgroundColor()) {
                {
                    let color = objc2_app_kit::NSColor::colorWithCGColor(&bg);
                    let rgb = color.and_then(|c| {
                        c.colorUsingColorSpace(&objc2_app_kit::NSColorSpace::deviceRGBColorSpace())
                    });
                    if let Some(rgb) = rgb {
                        properties.push(format!(
                            "bg:rgba({:.0},{:.0},{:.0},{:.2})",
                            rgb.redComponent() * 255.0,
                            rgb.greenComponent() * 255.0,
                            rgb.blueComponent() * 255.0,
                            rgb.alphaComponent()
                        ));
                    } else {
                        properties.push(format!("bg:{bg:?}"));
                    }
                }
            }
        }

        let _ = writeln!(result, " [{}]", properties.join(", "));

        // Process subviews
        let subviews = self.subviews();
        let count = subviews.len();
        for (index, subview) in subviews.iter().enumerate() {
            let is_last_subview = index == count - 1;
            let new_indent = format!("{indent}{}", if is_last { "    " } else { "│   " });
            result.push_str(&subview.view_hierarchy_description(&new_indent, is_last_subview));
        }

        result
    }
}
