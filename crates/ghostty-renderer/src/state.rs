//! The render state that is given to a renderer (`renderer/State.zig`).

use std::sync::{Arc, Mutex};

use ghostty_input::Mods;
use ghostty_terminal::point::Coordinate;
use ghostty_terminal::size::CellCountInt;
use ghostty_terminal::terminal::Terminal;

use crate::overlay::Feature as OverlayFeature;

/// The state shared between the IO side, the surface and the renderer.
///
/// Zig passes a `*Mutex` plus pointers to the terminal and friends. Rust
/// folds them into one value behind one mutex: everything in here must only
/// be read or written while holding [`SharedState`]'s lock (the "renderer
/// mutex" of Ghostty). The termio thread parses into `terminal`, the surface
/// updates `preedit`/`mouse`, and the renderer thread reads all of it in
/// `update_frame`.
#[derive(Debug)]
pub struct State {
    /// The terminal data.
    pub terminal: Terminal,

    /// The terminal inspector, if any (Zig: `inspector: ?*Inspector`). This
    /// is `None` while the inspector is not active. It is accessed under the
    /// same lock as the terminal (lock order: this state, then the
    /// inspector's own lock).
    pub inspector: Option<Arc<dyn InspectorHook>>,

    /// Dead key state. This will render the current dead key preedit text
    /// over the cursor. This currently only ever renders a single codepoint.
    /// Preedit can in theory be multiple codepoints long but that is left as
    /// a future exercise.
    pub preedit: Option<Preedit>,

    /// Mouse state. This only contains state relevant to what renderers
    /// need about the mouse.
    pub mouse: Mouse,
}

/// The view of the terminal inspector that the renderer and termio need
/// (the inspector crate itself depends on this crate, so the Zig
/// `*Inspector` pointer is expressed as a trait object).
pub trait InspectorHook: Send + Sync + std::fmt::Debug {
    /// The overlay features requested in the inspector's renderer window
    /// (Zig: `inspector.rendererInfo().overlayFeatures()`).
    fn overlay_features(&self) -> Vec<OverlayFeature>;

    /// Record data read from the pty before it is processed by the terminal
    /// (Zig: `Inspector.recordPtyRead`). Termio calls this one byte at a
    /// time while an inspector is active.
    fn record_pty_read(&self, t: &Terminal, data: &[u8]);
}

/// The shared, mutex-protected renderer state.
pub type SharedState = Arc<Mutex<State>>;

impl State {
    pub fn new(terminal: Terminal) -> State {
        State {
            terminal,
            inspector: None,
            preedit: None,
            mouse: Mouse::default(),
        }
    }

    /// Wrap this state into the shared form used by the renderer thread.
    pub fn into_shared(self) -> SharedState {
        Arc::new(Mutex::new(self))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Mouse {
    /// The point on the viewport where the mouse currently is. We use
    /// viewport points to avoid the complexity of mapping the mouse to
    /// the renderer state.
    pub point: Option<Coordinate>,

    /// The mods that are currently active for the last mouse event.
    /// This could really just be mods in general and we probably will
    /// move it out of mouse state at some point.
    pub mods: Mods,
}

/// The pre-edit state. See Surface.preeditCallback for more information.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preedit {
    /// The codepoints to render as preedit text.
    pub codepoints: Vec<PreeditCodepoint>,
}

/// A single codepoint to render as preedit text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreeditCodepoint {
    pub codepoint: u32,
    pub wide: bool,
}

/// The result of [`Preedit::range`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreeditRange {
    pub start: CellCountInt,
    pub end: CellCountInt,
    pub cp_offset: usize,
}

impl Preedit {
    /// The width in cells of all codepoints in the preedit.
    pub fn width(&self) -> usize {
        self.codepoints
            .iter()
            .map(|cp| if cp.wide { 2 } else { 1 })
            .sum()
    }

    /// Range returns the start and end x position of the preedit text
    /// along with any codepoint offset necessary to fit the preedit
    /// into the available space.
    pub fn range(&self, start: CellCountInt, max: CellCountInt) -> PreeditRange {
        // If our width is greater than the number of cells we have
        // then we need to adjust our codepoint start to a point where
        // our width would be less than the number of cells we have.
        let (w, cp_offset) = 'width: {
            // max is inclusive, so we need to add 1 to it.
            let max_width = max - start + 1;

            // Rebuild our width in reverse order. This is because we want
            // to offset by the end cells, not the start cells (if we have to).
            let mut w: CellCountInt = 0;
            for i in 0..self.codepoints.len() {
                let reverse_i = self.codepoints.len() - i - 1;
                let cp = self.codepoints[reverse_i];
                w += if cp.wide { 2 } else { 1 };
                if w > max_width {
                    break 'width (w, reverse_i);
                }
            }

            // Width fit in the max width so no offset necessary.
            (w, 0)
        };

        // If our preedit goes off the end of the screen, we adjust it so
        // that it shifts left.
        let end = start + w;
        let start_offset = end.saturating_sub(max);
        PreeditRange {
            start: start.saturating_sub(start_offset),
            end: end.saturating_sub(start_offset),
            cp_offset,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cp(c: char, wide: bool) -> PreeditCodepoint {
        PreeditCodepoint {
            codepoint: c as u32,
            wide,
        }
    }

    #[test]
    fn preedit_width() {
        let p = Preedit {
            codepoints: vec![cp('a', false), cp('b', true)],
        };
        assert_eq!(p.width(), 3);
        assert_eq!(Preedit::default().width(), 0);
    }

    #[test]
    fn preedit_range_fits() {
        let p = Preedit {
            codepoints: vec![cp('a', false), cp('b', true)],
        };
        let r = p.range(2, 9);
        assert_eq!(
            r,
            PreeditRange {
                start: 2,
                end: 5,
                cp_offset: 0
            }
        );
    }

    #[test]
    fn preedit_range_shifts_left_at_edge() {
        let p = Preedit {
            codepoints: vec![cp('a', false), cp('b', false), cp('c', false)],
        };
        // Start at the last column of a 10 column screen (max = 9).
        let r = p.range(8, 9);
        // Width is 3 > max_width (2): the span shifts left so it ends at max.
        assert_eq!(
            r,
            PreeditRange {
                start: 6,
                end: 9,
                cp_offset: 0
            }
        );
    }

    #[test]
    fn preedit_range_too_wide_for_screen() {
        let p = Preedit {
            codepoints: vec![cp('a', true), cp('b', true), cp('c', true)],
        };
        // 4 columns available starting at 0 (max = 3): width 6 > 4.
        let r = p.range(0, 3);
        // Walking in reverse: c(2) b(4) a(6>4) breaks at index 0.
        assert_eq!(r.cp_offset, 0);
        assert_eq!(r.end, 3);
        assert_eq!(r.start, 0);
    }
}
