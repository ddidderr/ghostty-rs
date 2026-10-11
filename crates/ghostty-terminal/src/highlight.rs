//! Highlights are any contiguous sequences of cells that should
//! be called out in some way, most commonly for text selection but
//! also search results or any other purpose.
//!
//! Within the terminal package, a highlight is a generic concept
//! that represents a range of cells.
//!
//! Port of `ZIG/src/terminal/highlight.zig`.

// NOTE: The plan is for highlights to ultimately replace Selection
// completely. Selection is deeply tied to various parts of the Ghostty
// internals so this may take some time.

use crate::{
    page::Error,
    page_list::{NodeId, PageList, Pin, TrackedPin},
    screen::Screen,
    size,
};

/// An untracked highlight is a highlight that stores its highlighted
/// area as a top-left and bottom-right screen pin. Since it is untracked,
/// the pins are only valid for the current terminal state and may not
/// be safe to use after any terminal modifications.
///
/// For rectangle highlights/selections, the downstream consumer of this
/// code is expected to interpret the pins in whatever shape they want.
/// For example, a rectangular selection would interpret the pins as
/// setting the x bounds for each row between start.y and end.y.
///
/// To simplify all operations, start MUST be before or equal to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Untracked {
    pub start: Pin,
    pub end: Pin,
}

impl Untracked {
    pub fn track(&self, screen: &mut Screen) -> Result<Tracked, Error> {
        Tracked::init(screen, self.start, self.end)
    }

    pub fn eql(self, other: Untracked) -> bool {
        self.start.eql(other.start) && self.end.eql(other.end)
    }
}

/// A tracked highlight is a highlight that stores its highlighted
/// area as tracked pins within a screen.
///
/// A tracked highlight ensures that the pins remain valid even as
/// the terminal state changes. Because of this, tracked highlights
/// have more operations available to them.
///
/// There is more overhead to creating and maintaining tracked highlights.
/// If you're manipulating highlights that are untracked and you're sure
/// that the terminal state won't change, you can use the `init_assume`
/// function.
///
/// Unlike Zig (which stores `*Pin`), the tracked pins are handles into the
/// owning [`PageList`]; read them with `pages.tracked_pin(h.start)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tracked {
    pub start: TrackedPin,
    pub end: TrackedPin,
}

impl Tracked {
    pub fn init(screen: &mut Screen, start: Pin, end: Pin) -> Result<Tracked, Error> {
        let start_tracked = screen.pages.track_pin(start)?;
        let end_tracked = match screen.pages.track_pin(end) {
            Ok(v) => v,
            Err(e) => {
                screen.pages.untrack_pin(start_tracked);
                return Err(e);
            }
        };
        Ok(Tracked {
            start: start_tracked,
            end: end_tracked,
        })
    }

    /// Initializes a tracked highlight by assuming that the provided
    /// pins are already tracked. This allows callers to perform tracked
    /// operations without the overhead of tracking the pins, if the
    /// caller can guarantee that the pins are already tracked or that
    /// the terminal state will not change.
    ///
    /// Do not call deinit on highlights created with this function.
    pub fn init_assume(start: TrackedPin, end: TrackedPin) -> Tracked {
        Tracked { start, end }
    }

    pub fn deinit(self, screen: &mut Screen) {
        screen.pages.untrack_pin(self.start);
        screen.pages.untrack_pin(self.end);
    }

    /// Current (untracked) value of the highlight.
    pub fn untracked(&self, pages: &PageList) -> Option<Untracked> {
        Some(Untracked {
            start: pages.tracked_pin(self.start)?,
            end: pages.tracked_pin(self.end)?,
        })
    }
}

/// A flattened highlight is a highlight that stores its highlighted
/// area as a list of page chunks. This representation allows for
/// traversing the entire highlighted area without needing to read any
/// terminal state or dereference any page nodes (which may have been
/// pruned).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Flattened {
    /// The page chunks that make up this highlight. This handles the
    /// y bounds since chunks[0].start is the first highlighted row
    /// and chunks[len - 1].end is the last highlighted row (exclusive).
    pub chunks: Vec<Chunk>,

    /// The x bounds of the highlight. `bot_x` may be less than `top_x`
    /// for typical left-to-right highlights: can start the selection right
    /// of the end on a higher row.
    pub top_x: size::CellCountInt,
    pub bot_x: size::CellCountInt,
}

/// A flattened chunk is almost identical to a PageList.Chunk but
/// we also flatten the serial number. This lets the flattened
/// highlight more robust for comparisons and validity checks with
/// the PageList.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub node: NodeId,
    pub serial: u64,
    pub start: size::CellCountInt,
    pub end: size::CellCountInt,
}

impl Flattened {
    pub const EMPTY: Flattened = Flattened {
        chunks: Vec::new(),
        top_x: 0,
        bot_x: 0,
    };

    pub fn empty() -> Flattened {
        Self::EMPTY
    }

    pub fn init(pages: &PageList, start: Pin, end: Pin) -> Result<Flattened, Error> {
        let mut result = Vec::new();
        for chunk in pin_page_iterator(pages, start, end) {
            let serial = pages
                .node(chunk.node)
                .ok_or(Error::InvalidCoordinate)?
                .serial;
            result.push(Chunk {
                node: chunk.node,
                serial,
                start: chunk.start,
                end: chunk.end,
            });
        }
        // NOTE: The Zig code assigns `.end_x = end.x` here (a typo for
        // `bot_x`, never compiled since the function is unused).
        Ok(Flattened {
            chunks: result,
            top_x: start.x,
            bot_x: end.x,
        })
    }

    pub fn start_pin(&self) -> Pin {
        let first = self.chunks[0];
        Pin {
            node: first.node,
            x: self.top_x,
            y: first.start,
            garbage: false,
        }
    }

    pub fn end_pin(&self) -> Pin {
        let last = self.chunks[self.chunks.len() - 1];
        Pin {
            node: last.node,
            x: self.bot_x,
            y: last.end - 1,
            garbage: false,
        }
    }

    /// Convert to an Untracked highlight.
    pub fn untracked(&self) -> Untracked {
        Untracked {
            start: self.start_pin(),
            end: self.end_pin(),
        }
    }
}

/// Equivalent of Zig `Pin.pageIterator(.right_down, limit)`: iterate the
/// page chunks (inclusive of `end`'s row) starting at `start`.
///
/// Like Zig this does not validate the pins against the page sizes (the end
/// row of the last chunk is `end.y + 1` even if the page is smaller), so
/// consumers must clamp.
pub fn pin_page_iterator(
    pages: &PageList,
    start: Pin,
    end: Pin,
) -> impl Iterator<Item = crate::page_list::Chunk> + '_ {
    let mut cur = Some(start.node);
    std::iter::from_fn(move || {
        let node = cur?;
        let n = pages.node(node)?;
        let first = node == start.node;
        let last = node == end.node;
        let chunk = crate::page_list::Chunk {
            node,
            start: if first { start.y } else { 0 },
            end: if last { end.y + 1 } else { n.data.size.rows },
        };
        cur = if last { None } else { n.next };
        Some(chunk)
    })
}
