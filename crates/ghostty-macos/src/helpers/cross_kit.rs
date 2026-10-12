//! `Helpers/CrossKit.swift`: types that are effectively identical between
//! AppKit and UIKit. Only the AppKit side exists in this port; the
//! `OSViewRepresentable` protocol is SwiftUI-only (unportable).

pub type OSView = objc2_app_kit::NSView;
pub type OSColor = objc2_app_kit::NSColor;
pub type OSSize = objc2_foundation::NSSize;
pub type OSPasteboard = objc2_app_kit::NSPasteboard;
