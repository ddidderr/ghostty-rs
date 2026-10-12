//! Linked terminal pages, byte-bounded history, and stable tracked coordinates.
use std::{
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    page::{
        self,
        Adjustment,
        Capacity,
        Cell,
        ContentTag,
        Error,
        Page,
        Row,
        SemanticContent,
        SemanticPrompt,
        Wide,
    },
    point::{Coordinate, Point, Tag},
};
/// Slab index and generation; stale references never alias a reused node.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct NodeId {
    index: usize,
    generation: u64,
}
impl NodeId {
    /// Index in the owning list's slab.
    pub fn index(self) -> usize {
        self.index
    }
    /// Generation assigned when the node was allocated.
    pub fn generation(self) -> u64 {
        self.generation
    }
}
/// One node in the page list.
#[derive(Debug)]
pub struct Node {
    /// Previous page.
    pub prev: Option<NodeId>,
    /// Next page.
    pub next: Option<NodeId>,
    /// Owned terminal page.
    pub data: Page,
    /// Monotonically increasing allocation serial.
    pub serial: u64,
}
/// Untracked cell location, valid until the referenced content is moved.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Pin {
    /// Page node.
    pub node: NodeId,
    /// Row in the page.
    pub y: u16,
    /// Column in the page.
    pub x: u16,
    /// History containing this pin was pruned or reset.
    pub garbage: bool,
}
/// Handle into the list-owned tracked-pin slab.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct TrackedPin {
    index: usize,
    generation: u64,
}
#[derive(Debug)]
struct PinSlot {
    generation: u64,
    pin: Option<Pin>,
}
/// Viewport anchor.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Viewport {
    /// Follow the bottom active area.
    Active,
    /// Follow the oldest history row.
    Top,
    /// Follow the list's internal tracked viewport pin.
    Pin,
}
/// Traversal direction.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Direction {
    /// Earlier cells, rows, or pages.
    LeftUp,
    /// Later cells, rows, or pages.
    RightDown,
}
/// Viewport scroll request.
#[derive(Debug, Copy, Clone)]
pub enum Scroll {
    /// Show the active area.
    Active,
    /// Show the oldest history.
    Top,
    /// Show an absolute screen row.
    Row(usize),
    /// Move by a signed row count.
    DeltaRow(isize),
    /// Jump by a signed number of prompts.
    DeltaPrompt(isize),
    /// Show a specific row, ignoring its column.
    Pin(Pin),
}
/// Scrollbar geometry in rows.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct Scrollbar {
    /// Total scrollable rows.
    pub total: usize,
    /// Viewport's offset from the oldest row.
    pub offset: usize,
    /// Visible row count.
    pub len: usize,
}
/// Current cursor position in the active area for resize behavior.
#[derive(Debug, Copy, Clone)]
pub struct Cursor {
    /// Cursor column.
    pub x: u16,
    /// Cursor row.
    pub y: u16,
}
/// Resize dimensions and text reflow policy.
#[derive(Debug, Copy, Clone)]
pub struct Resize {
    /// New column count, or unchanged.
    pub cols: Option<u16>,
    /// New row count, or unchanged.
    pub rows: Option<u16>,
    /// Rewrap logical lines when changing columns.
    pub reflow: bool,
    /// Cursor position whose surrounding rows should be preserved.
    pub cursor: Option<Cursor>,
}
impl Default for Resize {
    fn default() -> Self {
        Self {
            cols: None,
            rows: None,
            reflow: true,
            cursor: None,
        }
    }
}
/// Auxiliary capacity dimension to double.
#[derive(Debug, Copy, Clone)]
pub enum IncreaseCapacity {
    /// Style hash-table slots.
    Styles,
    /// Grapheme allocation bytes and map entries.
    GraphemeBytes,
    /// Hyperlink set and map capacity.
    HyperlinkBytes,
    /// Shared string allocation bytes.
    StringBytes,
}
/// Linked pages backed by a reusable node slab and a standard-page pool.
#[derive(Debug)]
pub struct PageList {
    nodes: Vec<Option<Node>>,
    free_nodes: Vec<usize>,
    page_pool: Vec<Page>,
    /// Never-written pages (all cells zero) that need no `reinit` memset.
    clean_pool: Vec<Page>,
    /// Oldest page.
    pub first: Option<NodeId>,
    /// Last active page.
    pub last: Option<NodeId>,
    /// Next page serial.
    pub page_serial: u64,
    /// Lower bound for still-valid serial numbers.
    pub page_serial_min: u64,
    /// Logical allocated page bytes, including standard pool rounding.
    pub page_size: usize,
    /// Configured scrollback byte limit.
    pub explicit_max_size: usize,
    /// Minimum byte limit needed for the active area and one extra page.
    pub min_max_size: usize,
    /// Total allocated visible rows across all pages.
    pub total_rows: usize,
    /// Current grid columns.
    pub cols: u16,
    /// Active grid rows.
    pub rows: u16,
    /// Current viewport anchor.
    pub viewport: Viewport,
    pins: Vec<PinSlot>,
    free_pins: Vec<usize>,
    viewport_pin: Option<TrackedPin>,
    /// Absolute viewport offset, or usize::MAX when invalidated. Atomic interior
    /// mutability preserves the read-only coordinate API and Send/Sync traits.
    viewport_pin_row_offset: AtomicUsize,
    pin_serial: u64,
    pause_checks: usize,
}
/// Failure points of [`PageList::init`] (Zig: `init_tw`). The Zig
/// `init_memory_pool` point has no counterpart: Rust has no up-front memory
/// pool, the slabs start empty and grow with the fallible reserves below.
/// `viewport_pin` and `viewport_pin_track` are two allocations in Zig (pool
/// create + set insert) that are one reserve in `track_pin` here; both points
/// are checked before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitPoint {
    InitPages,
    ViewportPin,
    ViewportPinTrack,
}
crate::tripwire_module!(init_tw, InitPoint, Error);

/// Failure points of the page creation done by `init` (Zig: `initPages_tw`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitPagesPoint {
    Node,
    BufStd,
    BufNonStd,
}
crate::tripwire_module!(init_pages_tw, InitPagesPoint, Error);

fn std_size() -> usize {
    Page::layout(page::STD_CAPACITY).total_size
}
fn allocated_size(cap: Capacity) -> usize {
    Page::layout(cap).total_size.max(std_size())
}
fn initial_capacity(cols: u16) -> Capacity {
    page::STD_CAPACITY
        .adjust(Adjustment { cols: Some(cols) })
        .unwrap_or(Capacity {
            cols,
            ..page::STD_CAPACITY
        })
}
fn min_max_size(cols: u16, rows: u16) -> usize {
    (usize::from(rows).div_ceil(usize::from(initial_capacity(cols).rows)) + 1) * std_size()
}
impl PageList {
    /// Create an empty active screen, optionally limiting history in bytes.
    pub fn init(cols: u16, rows: u16, max_size: Option<usize>) -> Result<Self, Error> {
        if cols == 0 || rows == 0 {
            return Err(Error::InvalidDimensions);
        }
        let mut s = Self {
            nodes: Vec::new(),
            free_nodes: Vec::new(),
            page_pool: Vec::new(),
            clean_pool: Vec::new(),
            first: None,
            last: None,
            page_serial: 0,
            page_serial_min: 0,
            page_size: 0,
            explicit_max_size: max_size.unwrap_or(usize::MAX),
            min_max_size: min_max_size(cols, rows),
            total_rows: 0,
            cols,
            rows,
            viewport: Viewport::Active,
            pins: Vec::new(),
            free_pins: Vec::new(),
            viewport_pin: None,
            viewport_pin_row_offset: AtomicUsize::new(usize::MAX),
            pin_serial: 0,
            pause_checks: 0,
        };
        init_tw::check(InitPoint::InitPages)?;
        while s.total_rows < usize::from(rows) {
            s.grow()?;
        }
        let pin = s
            .get_top_left(Tag::Screen)
            .ok_or(Error::InvalidCoordinate)?;
        init_tw::check(InitPoint::ViewportPin)?;
        init_tw::check(InitPoint::ViewportPinTrack)?;
        s.viewport_pin = Some(s.track_pin(pin)?);
        Ok(s)
    }
    /// Retrieve a live node, rejecting stale generations.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes
            .get(id.index)?
            .as_ref()
            .filter(|n| n.serial == id.generation)
    }
    /// Retrieve a mutable live node, rejecting stale generations.
    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes
            .get_mut(id.index)?
            .as_mut()
            .filter(|n| n.serial == id.generation)
    }
    fn create_node(&mut self, cap: Capacity) -> Result<NodeId, Error> {
        if Page::layout(cap).total_size > u32::MAX as usize {
            return Err(Error::OutOfSpace);
        }
        init_pages_tw::check(InitPagesPoint::Node)?;
        self.nodes.try_reserve(1).map_err(|_| Error::OutOfMemory)?;
        let mut data = if let Some(mut p) = self.clean_pool.pop().filter(|p| p.capacity == cap) {
            p.size = page::Size {
                cols: cap.cols,
                rows: cap.rows,
            };
            p
        } else if let Some(pos) = self.page_pool.iter().position(|p| p.capacity == cap) {
            let mut p = self.page_pool.swap_remove(pos);
            p.reinit()?;
            p
        } else {
            if Page::layout(cap).total_size <= std_size() {
                init_pages_tw::check(InitPagesPoint::BufStd)?;
            } else {
                init_pages_tw::check(InitPagesPoint::BufNonStd)?;
            }
            Page::init(cap)?
        };
        data.size.rows = 0;
        let index = self.free_nodes.pop().unwrap_or(self.nodes.len());
        let serial = self.page_serial;
        self.page_serial = self.page_serial.wrapping_add(1);
        let id = NodeId {
            index,
            generation: serial,
        };
        let node = Node {
            prev: None,
            next: None,
            data,
            serial,
        };
        if index == self.nodes.len() {
            self.nodes.push(Some(node));
        } else {
            self.nodes[index] = Some(node);
        }
        self.page_size += allocated_size(cap);
        Ok(id)
    }
    fn insert_after(&mut self, prev: Option<NodeId>, id: NodeId) {
        let next = prev
            .and_then(|p| self.node(p).and_then(|n| n.next))
            .or(if prev.is_none() { self.first } else { None });
        if let Some(n) = self.node_mut(id) {
            n.prev = prev;
            n.next = next;
        }
        if let Some(p) = prev {
            if let Some(n) = self.node_mut(p) {
                n.next = Some(id);
            }
        } else {
            self.first = Some(id);
        }
        if let Some(p) = next {
            if let Some(n) = self.node_mut(p) {
                n.prev = Some(id);
            }
        } else {
            self.last = Some(id);
        }
    }
    fn detach(&mut self, id: NodeId) -> Option<Node> {
        let n = self.node(id)?;
        let (prev, next) = (n.prev, n.next);
        if let Some(p) = prev {
            self.node_mut(p)?.next = next;
        } else {
            self.first = next;
        }
        if let Some(p) = next {
            self.node_mut(p)?.prev = prev;
        } else {
            self.last = prev;
        }
        let n = self.nodes[id.index].take()?;
        self.page_size -= allocated_size(n.data.capacity);
        self.free_nodes.push(id.index);
        Some(n)
    }
    fn recycle(&mut self, node: Node) {
        if Page::layout(node.data.capacity).total_size <= std_size() {
            self.page_pool.push(node.data);
        }
    }
    fn active_offset(&self) -> usize {
        self.total_rows.saturating_sub(usize::from(self.rows))
    }
    fn screen_row(&self, p: Pin) -> Option<usize> {
        if !self.pin_is_valid(p) {
            return None;
        }
        let mut id = self.first;
        let mut y = 0;
        while let Some(i) = id {
            let n = self.node(i)?;
            if i == p.node {
                return Some(y + usize::from(p.y));
            }
            y += usize::from(n.data.size.rows);
            id = n.next;
        }
        None
    }
    fn pin_at(&self, mut y: usize, x: u16) -> Option<Pin> {
        if x >= self.cols {
            return None;
        }
        let mut id = self.first;
        while let Some(i) = id {
            let n = self.node(i)?;
            if y < usize::from(n.data.size.rows) {
                return Some(Pin {
                    node: i,
                    y: y as u16,
                    x,
                    garbage: false,
                });
            }
            y -= usize::from(n.data.size.rows);
            id = n.next;
        }
        None
    }
    /// Current viewport's absolute row offset. A pinned viewport caches the
    /// result; the first lookup counts backwards from the active end.
    pub fn viewport_row_offset(&self) -> usize {
        match self.viewport {
            Viewport::Active => self.active_offset(),
            Viewport::Top => 0,
            Viewport::Pin => {
                let cached = self.viewport_pin_row_offset.load(Ordering::Relaxed);
                if cached != usize::MAX {
                    return cached;
                }
                let Some(pin) = self.viewport_pin.and_then(|h| self.tracked_pin(h)) else {
                    return 0;
                };
                let mut offset = self.total_rows;
                let mut node = self.last;
                while let Some(id) = node {
                    let n = self.node(id).expect("live viewport node");
                    offset -= usize::from(n.data.size.rows);
                    if id == pin.node {
                        offset += usize::from(pin.y);
                        self.viewport_pin_row_offset
                            .store(offset, Ordering::Relaxed);
                        return offset;
                    }
                    node = n.prev;
                }
                0
            }
        }
    }
    fn invalidate_viewport_offset(&self) {
        self.viewport_pin_row_offset
            .store(usize::MAX, Ordering::Relaxed);
    }
    fn bounds(&self, tag: Tag) -> (usize, usize) {
        match tag {
            Tag::Screen => (0, self.total_rows),
            Tag::History => (0, self.active_offset()),
            Tag::Active => (self.active_offset(), self.total_rows),
            Tag::Viewport => {
                let start = self.viewport_row_offset();
                (start, (start + usize::from(self.rows)).min(self.total_rows))
            }
        }
    }
    /// Map a tagged coordinate to an untracked pin. Active and viewport
    /// coordinates start at their anchors, independent of history length.
    pub fn pin(&self, pt: Point) -> Option<Pin> {
        let c = pt.coord();
        if c.x >= self.cols {
            return None;
        }
        let mut p = self.get_top_left(pt.tag())?.down(self, c.y as usize)?;
        p.x = c.x;
        Some(p)
    }
    /// Convert a pin to a coordinate by traversing only its reference frame.
    pub fn point_from_pin(&self, tag: Tag, p: Pin) -> Option<Point> {
        if !self.pin_is_valid(p) {
            return None;
        }
        let top = self.get_top_left(tag)?;
        let mut node = top.node;
        let mut y = 0;
        let mut start = top.y;
        loop {
            if node == p.node {
                y += u32::from(p.y.checked_sub(start)?);
                return Some(Point::new(tag, Coordinate { x: p.x, y }));
            }
            let n = self.node(node)?;
            y += u32::from(n.data.size.rows - start);
            node = n.next?;
            start = 0;
        }
    }
    /// Top-left coordinate in a reference frame. The active area is always
    /// at the tail, so locating it never walks through scrollback.
    pub fn get_top_left(&self, tag: Tag) -> Option<Pin> {
        match tag {
            Tag::Screen | Tag::History => Some(Pin {
                node: self.first?,
                x: 0,
                y: 0,
                garbage: false,
            }),
            Tag::Active => {
                let node = self.last?;
                Pin {
                    node,
                    x: 0,
                    y: self.node(node)?.data.size.rows - 1,
                    garbage: false,
                }
                .up(self, usize::from(self.rows) - 1)
            }
            Tag::Viewport => match self.viewport {
                Viewport::Active => self.get_top_left(Tag::Active),
                Viewport::Top => self.get_top_left(Tag::Screen),
                Viewport::Pin => self.viewport_pin.and_then(|h| self.tracked_pin(h)),
            },
        }
    }
    /// Bottom-right coordinate in a reference frame, if the region exists.
    pub fn get_bottom_right(&self, tag: Tag) -> Option<Pin> {
        let mut p = match tag {
            Tag::Screen | Tag::Active => {
                let node = self.last?;
                Pin {
                    node,
                    x: 0,
                    y: self.node(node)?.data.size.rows - 1,
                    garbage: false,
                }
            }
            Tag::Viewport => self
                .get_top_left(tag)?
                .down(self, usize::from(self.rows) - 1)?,
            Tag::History => self.get_top_left(Tag::Active)?.up(self, 1)?,
        };
        p.x = self.cols - 1;
        Some(p)
    }
    fn pin_is_active(&self, p: Pin) -> bool {
        self.point_from_pin(Tag::Active, p).is_some()
    }
    /// Check the generation and logical coordinates of a pin.
    pub fn pin_is_valid(&self, p: Pin) -> bool {
        self.node(p.node)
            .is_some_and(|n| p.x < n.data.size.cols && p.y < n.data.size.rows)
    }
    /// Register an independently owned location updated by every mutation.
    pub fn track_pin(&mut self, p: Pin) -> Result<TrackedPin, Error> {
        if !self.pin_is_valid(p) {
            return Err(Error::InvalidCoordinate);
        }
        self.pins.try_reserve(1).map_err(|_| Error::OutOfMemory)?;
        let index = self.free_pins.pop().unwrap_or(self.pins.len());
        let generation = self.pin_serial;
        self.pin_serial = self.pin_serial.wrapping_add(1);
        let slot = PinSlot {
            generation,
            pin: Some(p),
        };
        if index == self.pins.len() {
            self.pins.push(slot);
        } else {
            self.pins[index] = slot;
        }
        Ok(TrackedPin { index, generation })
    }
    /// Read a tracked location. Stale handles return None.
    pub fn tracked_pin(&self, h: TrackedPin) -> Option<Pin> {
        self.pins
            .get(h.index)
            .filter(|s| s.generation == h.generation)?
            .pin
    }
    /// Update a tracked location. The node must be live; row bounds may be
    /// temporarily exceeded while a caller repairs cursor state after splitting.
    pub fn set_tracked_pin(&mut self, h: TrackedPin, p: Pin) -> Result<(), Error> {
        if self.node(p.node).is_none() {
            return Err(Error::InvalidId);
        }
        let slot = self
            .pins
            .get_mut(h.index)
            .filter(|s| s.generation == h.generation && s.pin.is_some())
            .ok_or(Error::InvalidId)?;
        slot.pin = Some(p);
        if self.viewport_pin == Some(h) {
            self.invalidate_viewport_offset();
        }
        Ok(())
    }
    /// Mutable access to a tracked pin's location for hot paths that only
    /// change `x` within the same row (cursor movement while printing).
    /// Unlike [`PageList::set_tracked_pin`] this performs no node validation
    /// and does not invalidate the cached viewport offset, so it must not be
    /// used to move the pin to another row or node.
    #[inline]
    pub fn tracked_pin_x_mut(&mut self, h: TrackedPin) -> Option<&mut Pin> {
        self.pins
            .get_mut(h.index)
            .filter(|s| s.generation == h.generation)?
            .pin
            .as_mut()
    }
    /// Release a tracked handle. The internal viewport handle remains owned.
    pub fn untrack_pin(&mut self, h: TrackedPin) {
        if self.viewport_pin == Some(h) {
            return;
        }
        if let Some(s) = self
            .pins
            .get_mut(h.index)
            .filter(|s| s.generation == h.generation && s.pin.is_some())
        {
            s.pin = None;
            self.free_pins.push(h.index);
        }
    }
    /// Count all tracked pins, including the internal viewport pin.
    pub fn count_tracked_pins(&self) -> usize {
        self.pins.iter().filter(|s| s.pin.is_some()).count()
    }
    /// Iterate tracked handles and their current locations.
    pub fn tracked_pins(&self) -> impl Iterator<Item = (TrackedPin, Pin)> + '_ {
        self.pins.iter().enumerate().filter_map(|(index, s)| {
            s.pin.map(|p| {
                (
                    TrackedPin {
                        index,
                        generation: s.generation,
                    },
                    p,
                )
            })
        })
    }
    /// Actual byte ceiling heuristic, including the minimum active allocation.
    pub fn max_size(&self) -> usize {
        self.explicit_max_size.max(self.min_max_size)
    }
    /// Add exactly one row, pruning one whole history page when necessary.
    pub fn grow(&mut self) -> Result<Option<NodeId>, Error> {
        if let Some(last) = self.last
            && let Some(n) = self.node_mut(last)
            && n.data.size.rows < n.data.capacity.rows
        {
            n.data.size.rows += 1;
            self.total_rows += 1;
            return Ok(None);
        }
        let cap = initial_capacity(self.cols);
        if let (Some(first), Some(last)) = (self.first, self.last)
            && first != last
            && self.page_size.saturating_add(std_size()) > self.max_size()
        {
            let n = self.node(first).ok_or(Error::InvalidId)?;
            let removed = usize::from(n.data.size.rows);
            let next = n.next;
            if self.total_rows - removed + 1 >= usize::from(self.rows) {
                let cached = self.viewport_pin_row_offset.load(Ordering::Relaxed);
                let viewport =
                    (self.viewport == Viewport::Pin && cached != usize::MAX).then_some(cached);
                let n = self.detach(first).ok_or(Error::InvalidId)?;
                self.total_rows -= removed;
                if Page::layout(n.data.capacity).total_size <= std_size() {
                    self.page_serial_min = n.serial.wrapping_add(1);
                }
                self.recycle(n);
                if let Some(next) = next {
                    for slot in &mut self.pins {
                        if let Some(p) = &mut slot.pin
                            && p.node == first
                        {
                            *p = Pin {
                                node: next,
                                x: 0,
                                y: 0,
                                garbage: true,
                            };
                        }
                    }
                }
                if let Some(viewport) = viewport {
                    if viewport < removed {
                        self.viewport = Viewport::Top;
                    } else {
                        self.viewport_pin_row_offset
                            .store(viewport - removed, Ordering::Relaxed);
                    }
                }
                if let Some(h) = self.viewport_pin
                    && let Some(p) = self.pins[h.index].pin.as_mut()
                {
                    p.garbage = false;
                }
            }
        }
        let id = self.create_node(cap)?;
        self.insert_after(self.last, id);
        self.node_mut(id).ok_or(Error::InvalidId)?.data.size.rows = 1;
        self.total_rows += 1;
        Ok(Some(id))
    }
    /// Return scrollbar geometry, hiding history when scrollback is disabled.
    pub fn scrollbar(&self) -> Scrollbar {
        if self.explicit_max_size == 0 {
            Scrollbar {
                total: usize::from(self.rows),
                offset: 0,
                len: usize::from(self.rows),
            }
        } else {
            Scrollbar {
                total: self.total_rows,
                offset: self.viewport_row_offset(),
                len: usize::from(self.rows),
            }
        }
    }
    /// Move the viewport without allocating pages or creating history.
    pub fn scroll(&mut self, behavior: Scroll) {
        if self.explicit_max_size == 0 {
            self.viewport = Viewport::Active;
            return;
        }
        match behavior {
            Scroll::Active => self.viewport = Viewport::Active,
            Scroll::Top => self.viewport = Viewport::Top,
            Scroll::Pin(p) => {
                if self.pin_is_active(p) {
                    self.viewport = Viewport::Active;
                } else if Some(p.node) == self.first && p.y == 0 {
                    self.viewport = Viewport::Top;
                } else if let Some(h) = self.viewport_pin {
                    self.pins[h.index].pin = Some(p);
                    self.viewport = Viewport::Pin;
                    self.invalidate_viewport_offset();
                }
            }
            Scroll::Row(n) => {
                if n == 0 {
                    self.viewport = Viewport::Top;
                    return;
                }
                if n >= self.active_offset() {
                    self.viewport = Viewport::Active;
                    return;
                }
                let cached = self.viewport_pin_row_offset.load(Ordering::Relaxed);
                if self.viewport == Viewport::Pin && cached != usize::MAX {
                    self.scroll(Scroll::DeltaRow(n as isize - cached as isize));
                    return;
                }
                let p = if n < self.total_rows / 2 {
                    self.pin_at(n, 0)
                } else {
                    self.get_bottom_right(Tag::Screen)
                        .and_then(|p| p.up(self, self.total_rows - n - 1))
                        .map(|p| Pin { x: 0, ..p })
                };
                if let (Some(p), Some(h)) = (p, self.viewport_pin) {
                    self.pins[h.index].pin = Some(p);
                    self.viewport = Viewport::Pin;
                    self.viewport_pin_row_offset.store(n, Ordering::Relaxed);
                }
            }
            Scroll::DeltaRow(n) => {
                if n == 0
                    || (self.viewport == Viewport::Active && n > 0)
                    || (self.viewport == Viewport::Top && n < 0)
                {
                    return;
                }
                let Some(top) = self.get_top_left(Tag::Viewport) else {
                    return;
                };
                let moved = if n < 0 {
                    top.up(self, n.unsigned_abs())
                } else {
                    top.down(self, n as usize)
                };
                let was_pin = self.viewport == Viewport::Pin;
                let cached = self.viewport_pin_row_offset.load(Ordering::Relaxed);
                let Some(p) = moved else {
                    self.viewport = if n < 0 {
                        if self.active_offset() == 0 {
                            Viewport::Active
                        } else {
                            Viewport::Top
                        }
                    } else {
                        Viewport::Active
                    };
                    return;
                };
                // A pinned upward delta stays pinned even at row zero.
                if was_pin && (n < 0 || !self.pin_is_active(p)) {
                    let h = self.viewport_pin.expect("viewport handle");
                    self.pins[h.index].pin = Some(p);
                    if cached != usize::MAX {
                        self.viewport_pin_row_offset
                            .store(cached.saturating_add_signed(n), Ordering::Relaxed);
                    }
                } else {
                    self.scroll(Scroll::Pin(p));
                }
            }
            Scroll::DeltaPrompt(delta) => {
                if delta == 0 {
                    return;
                }
                let Some(top) = self.get_top_left(Tag::Viewport) else {
                    return;
                };
                let start = if delta < 0 {
                    top.up(self, 1)
                } else {
                    let mut next = top.down(self, 1);
                    if top
                        .row_and_cell(self)
                        .is_some_and(|(r, _)| r.semantic_prompt() != SemanticPrompt::None)
                    {
                        while let Some(p) = next {
                            if p.row_and_cell(self).is_none_or(|(r, _)| {
                                r.semantic_prompt() != SemanticPrompt::PromptContinuation
                            }) {
                                break;
                            }
                            let following = p.down(self, 1);
                            if following.is_none() {
                                break;
                            }
                            next = following;
                        }
                    }
                    next
                };
                let Some(start) = start else {
                    return;
                };
                let direction = if delta < 0 {
                    Direction::LeftUp
                } else {
                    Direction::RightDown
                };
                let mut it = PromptIterator {
                    list: self,
                    current: Some(start),
                    limit: None,
                    direction,
                };
                let mut found = None;
                for _ in 0..delta.unsigned_abs() {
                    let Some(p) = it.next() else {
                        break;
                    };
                    found = Some(p);
                }
                if let Some(p) = found {
                    if self.pin_is_active(p) {
                        self.viewport = Viewport::Active;
                    } else if let Some(h) = self.viewport_pin {
                        self.pins[h.index].pin = Some(p);
                        self.viewport = Viewport::Pin;
                        self.invalidate_viewport_offset();
                    }
                }
            }
        }
    }
    fn fixup_viewport(&mut self) {
        let pin = match self.viewport {
            Viewport::Active => return,
            Viewport::Top => self.get_top_left(Tag::Screen),
            Viewport::Pin => self.viewport_pin.and_then(|h| self.tracked_pin(h)),
        };
        if pin.is_some_and(|p| self.pin_is_active(p)) {
            self.viewport = Viewport::Active;
        }
    }
    /// Read a cell without copying its managed auxiliary values.
    pub fn get_cell(&self, pt: Point) -> Option<Cell> {
        let p = self.pin(pt)?;
        Some(
            self.node(p.node)?
                .data
                .get_row_and_cell(usize::from(p.x), usize::from(p.y))?
                .1
                .to_owned(),
        )
    }
    /// Access a cell mutably; callers maintain managed metadata and dirty flags.
    pub fn get_cell_mut(&mut self, pt: Point) -> Option<&mut Cell> {
        let p = self.pin(pt)?;
        let page = &mut self.node_mut(p.node)?.data;
        let i = page.cell_index(usize::from(p.x), usize::from(p.y)).ok()?;
        page.cells.get_mut(i)
    }
    /// The handle of the internal tracked viewport pin, if any (used by the
    /// inspector to label it).
    pub fn viewport_pin_handle(&self) -> Option<TrackedPin> {
        self.viewport_pin
    }
    /// Number of live pages in list order.
    pub fn total_pages(&self) -> usize {
        self.nodes.iter().filter(|n| n.is_some()).count()
    }
    /// Clear full-page and individual-row dirty flags.
    pub fn clear_dirty(&mut self) {
        for n in self.nodes.iter_mut().flatten() {
            n.data.dirty = false;
            for r in &mut n.data.rows {
                r.set_dirty(false);
            }
        }
    }
    /// Whether a coordinate's page or row needs redraw.
    pub fn is_dirty(&self, pt: Point) -> bool {
        self.pin(pt).is_some_and(|p| p.is_dirty(self))
    }
    /// Split before the pin's row, preserving metadata and remapping tracked pins.
    pub fn split(&mut self, p: Pin) -> Result<(), Error> {
        let page = &self.node(p.node).ok_or(Error::InvalidId)?.data;
        if page.size.rows <= 1 {
            return Err(Error::OutOfSpace);
        }
        if p.y == 0 {
            return Ok(());
        }
        if p.y >= page.size.rows {
            return Err(Error::InvalidCoordinate);
        }
        let id = self.clone_node_rows(p.node, p.y, page.size.rows, page.capacity)?;
        self.insert_after(Some(p.node), id);
        let src = &mut self.node_mut(p.node).ok_or(Error::InvalidId)?.data;
        for y in usize::from(p.y)..usize::from(src.size.rows) {
            src.clear_cells(y, 0, usize::from(src.size.cols), false)?;
        }
        src.size.rows = p.y;
        for s in &mut self.pins {
            if let Some(pin) = &mut s.pin
                && pin.node == p.node
                && pin.y >= p.y
            {
                pin.node = id;
                pin.y -= p.y;
            }
        }
        Ok(())
    }
    /// Clone a range directly into a pooled destination, without allocating
    /// another temporary Page with the same backing storage.
    fn clone_node_rows(
        &mut self,
        source: NodeId,
        start: u16,
        end: u16,
        cap: Capacity,
    ) -> Result<NodeId, Error> {
        let src = &self.node(source).ok_or(Error::InvalidId)?.data;
        let cols = src.size.cols;
        let dirty = src.dirty;
        let id = self.create_node(cap)?;
        let page = &mut self.node_mut(id).ok_or(Error::InvalidId)?.data;
        page.size.cols = cols;
        page.size.rows = end - start;
        page.dirty = dirty;
        for y in start..end {
            if let Err(error) = self.copy_row(source, y, id, y - start) {
                self.destroy_unlinked(id);
                return Err(error);
            }
        }
        Ok(id)
    }
    fn destroy_unlinked(&mut self, id: NodeId) {
        let node = self.nodes[id.index].take().expect("unlinked live node");
        self.page_size -= allocated_size(node.data.capacity);
        self.free_nodes.push(id.index);
        self.recycle(node);
    }
    fn replace_page(&mut self, old: NodeId, id: NodeId) -> Result<NodeId, Error> {
        let prev = self.node(old).ok_or(Error::InvalidId)?.prev;
        self.insert_after(prev, id);
        let n = self.detach(old).ok_or(Error::InvalidId)?;
        self.recycle(n);
        for slot in &mut self.pins {
            if let Some(p) = &mut slot.pin
                && p.node == old
            {
                p.node = id;
            }
        }
        Ok(id)
    }
    /// Double an auxiliary capacity, or rebuild unchanged to reclaim dead IDs.
    pub fn increase_capacity(
        &mut self,
        id: NodeId,
        adjustment: Option<IncreaseCapacity>,
    ) -> Result<NodeId, Error> {
        let old = &self.node(id).ok_or(Error::InvalidId)?.data;
        let mut cap = old.capacity;

        match adjustment {
            Some(IncreaseCapacity::Styles) => cap.styles = double16(cap.styles)?,
            Some(IncreaseCapacity::GraphemeBytes) => {
                cap.grapheme_bytes = double32(cap.grapheme_bytes)?;
            }
            Some(IncreaseCapacity::HyperlinkBytes) => {
                cap.hyperlink_bytes = double16(cap.hyperlink_bytes)?;
            }
            Some(IncreaseCapacity::StringBytes) => cap.string_bytes = double32(cap.string_bytes)?,
            None => {}
        }
        if Page::layout(cap).total_size > u32::MAX as usize {
            return Err(Error::OutOfSpace);
        }
        let new = self.clone_node_rows(id, 0, old.size.rows, cap)?;
        self.replace_page(id, new)
    }
    /// Alias for capacity adjustment used by higher-level screen code.
    pub fn adjust_capacity(
        &mut self,
        id: NodeId,
        adjustment: Option<IncreaseCapacity>,
    ) -> Result<NodeId, Error> {
        self.increase_capacity(id, adjustment)
    }
    /// Compact oversized pages when a smaller logical allocation is possible.
    pub fn compact(&mut self, id: NodeId) -> Result<Option<NodeId>, Error> {
        let old = &self.node(id).ok_or(Error::InvalidId)?.data;
        if Page::layout(old.capacity).total_size <= std_size() {
            return Ok(None);
        }
        let cap = old.exact_row_capacity(0, usize::from(old.size.rows))?;
        if Page::layout(cap).total_size >= Page::layout(old.capacity).total_size {
            return Ok(None);
        }
        let new = match self.clone_node_rows(id, 0, old.size.rows, cap) {
            Ok(new) => new,
            Err(Error::OutOfMemory) => return Err(Error::OutOfMemory),
            Err(_) => return Ok(None),
        };
        self.replace_page(id, new).map(Some)
    }
    fn erase_range(&mut self, start: usize, end: usize) -> Result<(), Error> {
        if start > end || end > self.total_rows {
            return Err(Error::InvalidCoordinate);
        }
        if start == end {
            return Ok(());
        }
        self.invalidate_viewport_offset();
        let mut node = self.first;
        let mut offset = 0;
        while let Some(id) = node {
            let current = self.node(id).ok_or(Error::InvalidId)?;
            node = current.next;
            let rows = usize::from(current.data.size.rows);
            let a = start.saturating_sub(offset).min(rows);
            let b = end.saturating_sub(offset).min(rows);
            offset += rows;
            if a == b {
                continue;
            }
            if a == 0 && b == rows && (current.prev.is_some() || current.next.is_some()) {
                let dest = current.prev.or(current.next).ok_or(Error::InvalidId)?;
                for slot in &mut self.pins {
                    if let Some(p) = &mut slot.pin
                        && p.node == id
                    {
                        p.node = dest;
                        p.x = 0;
                        p.y = 0;
                    }
                }
                let old = self.detach(id).ok_or(Error::InvalidId)?;
                self.recycle(old);
            } else if a == 0 && b == rows {
                let page = &mut self.node_mut(id).ok_or(Error::InvalidId)?.data;
                let cols = page.size.cols;
                page.reinit()?;
                page.size.cols = cols;
                page.size.rows = 0;
            } else {
                let page = &mut self.node_mut(id).ok_or(Error::InvalidId)?.data;
                for y in a..b {
                    page.clear_cells(y, 0, usize::from(page.size.cols), false)?;
                }
                page.rows[a..rows].rotate_left(b - a);
                page.size.rows -= (b - a) as u16;
                for row in &mut page.rows[a..usize::from(page.size.rows)] {
                    row.set_dirty(true);
                }
                for slot in &mut self.pins {
                    if let Some(p) = &mut slot.pin
                        && p.node == id
                    {
                        if usize::from(p.y) >= b {
                            p.y -= (b - a) as u16;
                        } else if usize::from(p.y) >= a {
                            p.y = 0;
                            p.x = 0;
                        }
                    }
                }
            }
            if offset >= end {
                break;
            }
        }
        self.total_rows -= end - start;
        Ok(())
    }
    /// Remove an inclusive row range, regrowing erased active rows.
    pub fn erase_rows(&mut self, start: Point, end: Option<Point>) -> Result<(), Error> {
        // An empty history produces no chunks in the reference iterator.
        if start.tag() == Tag::History && self.active_offset() == 0 && end.is_none() {
            return Ok(());
        }
        let a = self
            .pin(start)
            .and_then(|p| self.screen_row(p))
            .ok_or(Error::InvalidCoordinate)?;
        let b = if let Some(end) = end {
            self.pin(end)
                .and_then(|p| self.screen_row(p))
                .ok_or(Error::InvalidCoordinate)?
                + 1
        } else {
            self.bounds(start.tag()).1
        };
        let cached = self.viewport_pin_row_offset.load(Ordering::Relaxed);
        self.erase_range(a, b)?;
        let regrow = if start.tag() == Tag::Active {
            b - a
        } else {
            usize::from(self.rows).saturating_sub(self.total_rows)
        };
        for _ in 0..regrow {
            self.grow()?;
        }
        self.fixup_viewport();
        if a == 0 && self.viewport == Viewport::Pin && cached != usize::MAX {
            if cached < b - a {
                self.viewport = Viewport::Top;
            } else {
                self.viewport_pin_row_offset
                    .store(cached - (b - a), Ordering::Relaxed);
            }
        }
        self.fixup_viewport();
        Ok(())
    }
    /// Remove one row, shifting following rows without changing page dimensions.
    pub fn erase_row(&mut self, pt: Point) -> Result<(), Error> {
        self.erase_row_bounded_impl(pt, usize::MAX, true)?;
        self.invalidate_viewport_offset();
        self.fixup_viewport();
        Ok(())
    }
    /// Remove a row and shift at most `limit` following rows, blanking the last.
    pub fn erase_row_bounded(&mut self, pt: Point, limit: usize) -> Result<(), Error> {
        self.erase_row_bounded_impl(pt, limit, false)
    }
    fn erase_row_bounded_impl(
        &mut self,
        pt: Point,
        limit: usize,
        keep_erased_pin: bool,
    ) -> Result<(), Error> {
        let first = self.pin(pt).ok_or(Error::InvalidCoordinate)?;
        let mut node = first.node;
        let mut start = first.y;
        let mut remaining = limit;
        loop {
            let current = self.node(node).ok_or(Error::InvalidId)?;
            let rows = current.data.size.rows;
            let next = current.next;
            let previous = current.prev;
            let previous_rows = previous
                .and_then(|p| self.node(p))
                .map(|n| n.data.size.rows);
            let count = (remaining.saturating_add(1)).min(usize::from(rows - start));
            let end = start + count as u16;
            let stop = usize::from(rows - start) > remaining;
            let page = &mut self.node_mut(node).ok_or(Error::InvalidId)?.data;
            if stop {
                page.clear_cells(usize::from(start), 0, usize::from(page.size.cols), false)?;
            }
            page.rows[usize::from(start)..usize::from(end)].rotate_left(1);
            page.dirty = true;
            if self.viewport == Viewport::Pin
                && let Some(p) = self.viewport_pin.and_then(|h| self.tracked_pin(h))
                && p.node == node
                && p.y >= start
                && p.y < end
                && (node != first.node || p.y > 0)
            {
                let cached = self.viewport_pin_row_offset.load(Ordering::Relaxed);
                if cached != usize::MAX {
                    self.viewport_pin_row_offset
                        .store(cached - 1, Ordering::Relaxed);
                }
            }
            for slot in &mut self.pins {
                if let Some(p) = &mut slot.pin
                    && p.node == node
                    && p.y >= start
                    && p.y < end
                {
                    if keep_erased_pin && node == first.node && p.y == first.y {
                        continue;
                    }
                    if p.y > 0 {
                        p.y -= 1;
                    } else if node == first.node {
                        p.x = 0;
                    } else {
                        p.node = previous.ok_or(Error::InvalidId)?;
                        p.y = previous_rows.ok_or(Error::InvalidId)? - 1;
                    }
                }
            }
            if stop {
                return Ok(());
            }
            if let Some(next) = next {
                self.copy_row(next, 0, node, rows - 1)?;
                remaining -= usize::from(rows - start);
                node = next;
                start = 0;
            } else {
                let page = &mut self.node_mut(node).ok_or(Error::InvalidId)?.data;
                page.clear_cells(usize::from(rows - 1), 0, usize::from(page.size.cols), false)?;
                return Ok(());
            }
        }
    }
    /// Remove cells within a rectangular row region, shifting rows upward.
    pub fn erase_cells_bounded(
        &mut self,
        start: Pin,
        end: Pin,
        left: u16,
        right: u16,
    ) -> Result<(), Error> {
        let a = self.screen_row(start).ok_or(Error::InvalidCoordinate)?;
        let b = self.screen_row(end).ok_or(Error::InvalidCoordinate)?;
        if a > b || left > right || right >= self.cols {
            return Err(Error::InvalidCoordinate);
        }
        for y in a..b {
            let src = self.pin_at(y + 1, left).ok_or(Error::InvalidCoordinate)?;
            let dst = self.pin_at(y, left).ok_or(Error::InvalidCoordinate)?;
            if src.node == dst.node {
                self.node_mut(src.node)
                    .ok_or(Error::InvalidId)?
                    .data
                    .clone_cells_within(
                        usize::from(src.y),
                        usize::from(dst.y),
                        usize::from(left),
                        usize::from(left),
                        usize::from(right - left + 1),
                    )?;
            } else {
                let (source, dest) = self.pages_for_copy(src.node, dst.node)?;
                let base = source.rows[usize::from(src.y)].cells();
                for x in left..=right {
                    dest.clone_cell_from(
                        source,
                        base + usize::from(x),
                        usize::from(x),
                        usize::from(dst.y),
                    )?;
                }
            }
        }
        let p = self.pin_at(b, left).ok_or(Error::InvalidCoordinate)?;
        self.node_mut(p.node)
            .ok_or(Error::InvalidId)?
            .data
            .clear_cells(
                usize::from(p.y),
                usize::from(left),
                usize::from(right) + 1,
                false,
            )?;
        let changes: Vec<_> = self
            .tracked_pins()
            .filter_map(|(h, p)| {
                self.screen_row(p)
                    .filter(|y| *y >= a && *y <= b && p.x >= left && p.x <= right)
                    .map(|y| (h, p, y))
            })
            .collect();
        for (h, p, y) in changes {
            if let Some(mut new) = self.pin_at(y.saturating_sub(1).max(a), p.x) {
                new.garbage = p.garbage;
                self.pins[h.index].pin = Some(new);
            }
        }
        self.fixup_viewport();
        Ok(())
    }
    /// Reset to empty active rows and mark every external tracked pin garbage.
    pub fn reset(&mut self) -> Result<(), Error> {
        let handles: Vec<_> = self.tracked_pins().map(|(h, _)| h).collect();
        let mut id = self.first;
        while let Some(i) = id {
            let n = self.node(i).ok_or(Error::InvalidId)?;
            id = n.next;
            let n = self.detach(i).ok_or(Error::InvalidId)?;
            self.recycle(n);
        }
        let cap = initial_capacity(self.cols);
        let pages = usize::from(self.rows).div_ceil(usize::from(cap.rows));
        self.page_pool.retain(|page| page.capacity == cap);
        self.page_pool.truncate(pages);
        self.nodes.clear();
        self.free_nodes.clear();
        self.nodes.shrink_to(pages);
        self.free_nodes.shrink_to(pages);
        self.total_rows = 0;
        self.viewport = Viewport::Active;
        while self.total_rows < usize::from(self.rows) {
            self.grow()?;
        }
        let p = self
            .get_top_left(Tag::Screen)
            .ok_or(Error::InvalidCoordinate)?;
        for h in handles {
            self.pins[h.index].pin = Some(Pin {
                garbage: self.viewport_pin != Some(h),
                ..p
            });
        }
        Ok(())
    }
    /// Scroll the nonempty portion of the active area into history.
    pub fn scroll_clear(&mut self) -> Result<(), Error> {
        let count = self
            .row_iterator(
                Direction::LeftUp,
                Point::Active(Coordinate::default()),
                None,
            )
            .find(|p| {
                self.node(p.node)
                    .and_then(|n| n.data.get_cells(usize::from(p.y)))
                    .is_some_and(|cells| cells.iter().any(|c| !c.is_empty()))
            })
            .and_then(|p| self.point_from_pin(Tag::Active, p))
            .map_or(0, |p| p.coord().y as usize + 1);
        for _ in 0..count {
            self.grow()?;
        }
        Ok(())
    }
    /// Erase all scrollback while preserving the active area.
    pub fn clear_history(&mut self) -> Result<(), Error> {
        let end = self.active_offset();
        self.erase_range(0, end)?;
        self.viewport = Viewport::Active;
        Ok(())
    }
    /// Nestable suspension of debug integrity checks.
    pub fn pause_integrity_checks(&mut self, pause: bool) {
        if pause {
            self.pause_checks += 1;
        } else {
            self.pause_checks = self.pause_checks.saturating_sub(1);
        }
    }
}
/// Result of moving vertically past a list boundary.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Overflow {
    /// Movement remained in the screen.
    Offset(Pin),
    /// Movement reached an edge with rows still remaining.
    Overflow {
        /// Last valid position, retaining the original column.
        end: Pin,
        /// Number of rows beyond the boundary.
        remaining: usize,
    },
}
/// Which part of a pin's row to expose.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CellSubset {
    /// Entire visible row.
    All,
    /// Columns up to and including the pin.
    Left,
    /// Pin column and all columns to its right.
    Right,
}
impl Pin {
    /// Read a visible row subset without copying or allocating.
    pub fn cells(self, list: &PageList, subset: CellSubset) -> Option<&[Cell]> {
        let cells = list.node(self.node)?.data.get_cells(usize::from(self.y))?;
        match subset {
            CellSubset::All => Some(cells),
            CellSubset::Left => cells.get(..=usize::from(self.x)),
            CellSubset::Right => cells.get(usize::from(self.x)..),
        }
    }
    /// Move upward, returning the edge and unconsumed distance on overflow.
    pub fn up_overflow(self, list: &PageList, n: usize) -> Option<Overflow> {
        let mut p = self;
        let mut remaining = n;
        loop {
            if remaining <= usize::from(p.y) {
                p.y -= remaining as u16;
                return Some(Overflow::Offset(p));
            }
            remaining -= usize::from(p.y);
            let Some(prev) = list.node(p.node)?.prev else {
                p.y = 0;
                return Some(Overflow::Overflow { end: p, remaining });
            };
            remaining -= 1;
            p.node = prev;
            p.y = list.node(prev)?.data.size.rows - 1;
        }
    }
    /// Move downward, returning the edge and unconsumed distance on overflow.
    pub fn down_overflow(self, list: &PageList, n: usize) -> Option<Overflow> {
        let mut p = self;
        let mut remaining = n;
        loop {
            let node = list.node(p.node)?;
            let available = usize::from(node.data.size.rows - p.y - 1);
            if remaining <= available {
                p.y += remaining as u16;
                return Some(Overflow::Offset(p));
            }
            remaining -= available;
            let Some(next) = node.next else {
                p.y = node.data.size.rows - 1;
                return Some(Overflow::Overflow { end: p, remaining });
            };
            remaining -= 1;
            p.node = next;
            p.y = 0;
        }
    }
    /// Compare page and coordinates, ignoring garbage state.
    pub fn eql(self, other: Self) -> bool {
        self.node == other.node && self.x == other.x && self.y == other.y
    }
    /// Read the row and cell from the owning list.
    pub fn row_and_cell(self, list: &PageList) -> Option<(&Row, &Cell)> {
        list.node(self.node)?
            .data
            .get_row_and_cell(usize::from(self.x), usize::from(self.y))
    }
    /// Whether the row or page requires redraw.
    pub fn is_dirty(self, list: &PageList) -> bool {
        list.node(self.node).is_some_and(|n| {
            n.data.dirty
                || n.data
                    .get_row(usize::from(self.y))
                    .is_some_and(|r| r.dirty())
        })
    }
    /// Mark the referenced row dirty.
    pub fn mark_dirty(self, list: &mut PageList) {
        if let Some(r) = list
            .node_mut(self.node)
            .and_then(|n| n.data.get_row_mut(usize::from(self.y)))
        {
            r.set_dirty(true);
        }
    }
    /// Resolve the current cell's style.
    pub fn style(self, list: &PageList) -> Option<crate::style::Style> {
        let page = &list.node(self.node)?.data;
        Some(
            page.style(
                *page
                    .get_row_and_cell(usize::from(self.x), usize::from(self.y))?
                    .1,
            ),
        )
    }
    /// Resolve the current cell's extra codepoints.
    pub fn grapheme(self, list: &PageList) -> Option<&[u32]> {
        let page = &list.node(self.node)?.data;
        page.lookup_grapheme(
            page.cell_index(usize::from(self.x), usize::from(self.y))
                .ok()?,
        )
    }
    /// Move left within a row, rejecting out-of-range movement.
    pub fn left(self, n: usize) -> Option<Self> {
        Some(Self {
            x: self.x.checked_sub(u16::try_from(n).ok()?)?,
            ..self
        })
    }
    /// Move right within a row, rejecting out-of-range movement.
    pub fn right(self, list: &PageList, n: usize) -> Option<Self> {
        let x = usize::from(self.x).checked_add(n)?;
        if x >= usize::from(list.cols) {
            None
        } else {
            Some(Self {
                x: x as u16,
                ..self
            })
        }
    }
    /// Move left, clamping to column zero.
    pub fn left_clamp(self, n: u16) -> Self {
        Self {
            x: self.x.saturating_sub(n),
            ..self
        }
    }
    /// Move right, clamping to the last column.
    pub fn right_clamp(self, list: &PageList, n: u16) -> Self {
        Self {
            x: self.x.saturating_add(n).min(list.cols - 1),
            ..self
        }
    }
    /// Move up across page boundaries while preserving the column.
    pub fn up(self, list: &PageList, mut n: usize) -> Option<Self> {
        let mut p = self;
        loop {
            if n <= usize::from(p.y) {
                p.y -= n as u16;
                return Some(p);
            }
            n -= usize::from(p.y) + 1;
            p.node = list.node(p.node)?.prev?;
            p.y = list.node(p.node)?.data.size.rows.checked_sub(1)?;
        }
    }
    /// Move down across page boundaries while preserving the column.
    pub fn down(self, list: &PageList, mut n: usize) -> Option<Self> {
        let mut p = self;
        loop {
            let rem = usize::from(list.node(p.node)?.data.size.rows - p.y - 1);
            if n <= rem {
                p.y += n as u16;
                return Some(p);
            }
            n -= rem + 1;
            p.node = list.node(p.node)?.next?;
            p.y = 0;
        }
    }
    /// Move left in row-major order, wrapping to previous rows.
    pub fn left_wrap(self, list: &PageList, n: usize) -> Option<Self> {
        let cols = usize::from(list.cols);
        let x = usize::from(self.x);
        if n <= x {
            return self.left(n);
        }
        let delta = n - x;
        let rows = delta.div_ceil(cols);
        let mut p = self.up(list, rows)?;
        p.x = ((cols - delta % cols) % cols) as u16;
        Some(p)
    }
    /// Move right in row-major order, wrapping to following rows.
    pub fn right_wrap(self, list: &PageList, n: usize) -> Option<Self> {
        let x = usize::from(self.x).checked_add(n)?;
        let cols = usize::from(list.cols);
        let mut p = self.down(list, x / cols)?;
        p.x = (x % cols) as u16;
        Some(p)
    }
    /// Whether this pin precedes another pin in row-major screen order.
    pub fn before(self, list: &PageList, other: Self) -> bool {
        if self.node == other.node {
            return (self.y, self.x) < (other.y, other.x);
        }
        let mut node = list.node(self.node).and_then(|n| n.next);
        while let Some(id) = node {
            if id == other.node {
                return true;
            }
            node = list.node(id).and_then(|n| n.next);
        }
        false
    }
    /// Whether this pin lies between inclusive endpoints.
    pub fn is_between(self, list: &PageList, top: Self, bottom: Self) -> bool {
        (self.eql(top) || top.before(list, self)) && (self.eql(bottom) || self.before(list, bottom))
    }
    /// Move diagonally by signed row and column deltas, clamping columns.
    pub fn diagonal(self, list: &PageList, dx: isize, dy: isize) -> Option<Self> {
        let mut p = if dy < 0 {
            self.up(list, dy.unsigned_abs())?
        } else {
            self.down(list, dy as usize)?
        };
        p.x = usize::from(p.x)
            .saturating_add_signed(dx)
            .min(usize::from(list.cols - 1)) as u16;
        Some(p)
    }
}
impl Pin {
    /// Iterate cells toward an inclusive limit row. The limit column is ignored.
    pub fn cell_iterator(
        self,
        list: &PageList,
        direction: Direction,
        limit: Option<Pin>,
    ) -> CellIterator<'_> {
        let (front, back) = match direction {
            Direction::RightDown => (
                Some(self),
                limit
                    .or_else(|| list.get_bottom_right(Tag::Screen))
                    .map(|p| Pin {
                        x: list.cols - 1,
                        ..p
                    }),
            ),
            Direction::LeftUp => (
                limit
                    .or_else(|| list.get_top_left(Tag::Screen))
                    .map(|p| Pin { x: 0, ..p }),
                Some(self),
            ),
        };
        CellIterator {
            list,
            front,
            back,
            direction,
        }
    }
    /// Iterate rows directly from a pin, with an inclusive limit row.
    pub fn row_iterator(
        self,
        list: &PageList,
        direction: Direction,
        limit: Option<Pin>,
    ) -> RowIterator<'_> {
        let cells = self.cell_iterator(list, direction, limit);
        RowIterator {
            list,
            front: cells.front.map(|p| Pin { x: 0, ..p }),
            back: cells.back.map(|p| Pin { x: 0, ..p }),
            direction,
        }
    }
    /// Iterate maximal page chunks directly from a pin.
    pub fn page_iterator(
        self,
        list: &PageList,
        direction: Direction,
        limit: Option<Pin>,
    ) -> PageIterator<'_> {
        PageIterator {
            rows: self.row_iterator(list, direction, limit),
        }
    }
    /// Iterate prompt starts directly from a pin, with an inclusive limit row.
    pub fn prompt_iterator(
        self,
        list: &PageList,
        direction: Direction,
        limit: Option<Pin>,
    ) -> PromptIterator<'_> {
        PromptIterator {
            list,
            current: Some(self),
            limit,
            direction,
        }
    }
}
/// A double-ended cell iterator yielding stable untracked locations.
pub struct CellIterator<'a> {
    list: &'a PageList,
    front: Option<Pin>,
    back: Option<Pin>,
    direction: Direction,
}
impl CellIterator<'_> {
    fn take_front(&mut self) -> Option<Pin> {
        let p = self.front?;
        if self.back.is_some_and(|b| p.eql(b)) {
            self.front = None;
            self.back = None;
        } else {
            self.front = p.right_wrap(self.list, 1);
        }
        Some(p)
    }
    fn take_back(&mut self) -> Option<Pin> {
        let p = self.back?;
        if self.front.is_some_and(|a| p.eql(a)) {
            self.front = None;
            self.back = None;
        } else {
            self.back = p.left_wrap(self.list, 1);
        }
        Some(p)
    }
}
impl Iterator for CellIterator<'_> {
    type Item = Pin;
    fn next(&mut self) -> Option<Pin> {
        if self.direction == Direction::RightDown {
            self.take_front()
        } else {
            self.take_back()
        }
    }
}
impl DoubleEndedIterator for CellIterator<'_> {
    fn next_back(&mut self) -> Option<Pin> {
        if self.direction == Direction::RightDown {
            self.take_back()
        } else {
            self.take_front()
        }
    }
}
/// A double-ended row iterator yielding column-zero pins.
pub struct RowIterator<'a> {
    list: &'a PageList,
    front: Option<Pin>,
    back: Option<Pin>,
    direction: Direction,
}
impl RowIterator<'_> {
    fn take_front(&mut self) -> Option<Pin> {
        let p = self.front?;
        if self.back.is_some_and(|b| p.eql(b)) {
            self.front = None;
            self.back = None;
        } else {
            self.front = p.down(self.list, 1);
        }
        Some(p)
    }
    fn take_back(&mut self) -> Option<Pin> {
        let p = self.back?;
        if self.front.is_some_and(|a| p.eql(a)) {
            self.front = None;
            self.back = None;
        } else {
            self.back = p.up(self.list, 1);
        }
        Some(p)
    }
}
impl Iterator for RowIterator<'_> {
    type Item = Pin;
    fn next(&mut self) -> Option<Pin> {
        if self.direction == Direction::RightDown {
            self.take_front()
        } else {
            self.take_back()
        }
    }
}
impl DoubleEndedIterator for RowIterator<'_> {
    fn next_back(&mut self) -> Option<Pin> {
        if self.direction == Direction::RightDown {
            self.take_back()
        } else {
            self.take_front()
        }
    }
}
/// A contiguous half-open row range within one page.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Page node.
    pub node: NodeId,
    /// First row included.
    pub start: u16,
    /// First row excluded.
    pub end: u16,
}
impl Chunk {
    /// Row metadata covered by this chunk.
    pub fn rows(self, list: &PageList) -> Option<&[Row]> {
        list.node(self.node)?
            .data
            .rows
            .get(usize::from(self.start)..usize::from(self.end))
    }
    /// True if all logical rows in the page are covered.
    pub fn full_page(self, list: &PageList) -> bool {
        self.start == 0
            && list
                .node(self.node)
                .is_some_and(|n| self.end == n.data.size.rows)
    }
    /// True when two chunks overlap in the same page.
    pub fn overlaps(self, other: Self) -> bool {
        self.node == other.node && self.start < other.end && other.start < self.end
    }
}
/// A double-ended page iterator yielding maximal contiguous row chunks.
pub struct PageIterator<'a> {
    rows: RowIterator<'a>,
}
impl PageIterator<'_> {
    fn front(&mut self) -> Option<Chunk> {
        let a = self.rows.front?;
        let b = self.rows.back?;
        let end = if a.node == b.node {
            b.y + 1
        } else {
            self.rows.list.node(a.node)?.data.size.rows
        };
        let c = Chunk {
            node: a.node,
            start: a.y,
            end,
        };
        if a.node == b.node {
            self.rows.front = None;
            self.rows.back = None;
        } else {
            self.rows.front = Some(Pin {
                node: self.rows.list.node(a.node)?.next?,
                y: 0,
                x: 0,
                garbage: false,
            });
        }
        Some(c)
    }
    fn back(&mut self) -> Option<Chunk> {
        let a = self.rows.front?;
        let b = self.rows.back?;
        let start = if a.node == b.node { a.y } else { 0 };
        let c = Chunk {
            node: b.node,
            start,
            end: b.y + 1,
        };
        if a.node == b.node {
            self.rows.front = None;
            self.rows.back = None;
        } else {
            let node = self.rows.list.node(b.node)?.prev?;
            self.rows.back = Some(Pin {
                node,
                y: self.rows.list.node(node)?.data.size.rows - 1,
                x: 0,
                garbage: false,
            });
        }
        Some(c)
    }
}
impl Iterator for PageIterator<'_> {
    type Item = Chunk;
    fn next(&mut self) -> Option<Chunk> {
        if self.rows.direction == Direction::RightDown {
            self.front()
        } else {
            self.back()
        }
    }
}
impl DoubleEndedIterator for PageIterator<'_> {
    fn next_back(&mut self) -> Option<Chunk> {
        if self.rows.direction == Direction::RightDown {
            self.back()
        } else {
            self.front()
        }
    }
}
impl PageList {
    fn iter_bounds(&self, start: Point, end: Option<Point>) -> (Option<Pin>, Option<Pin>) {
        let a = self.pin(start);
        let b = end.map_or_else(
            || self.get_bottom_right(start.tag()),
            |p| {
                if p.tag() == start.tag() {
                    self.pin(p)
                } else {
                    None
                }
            },
        );
        if let (Some(a), Some(b)) = (a, b)
            && (a.eql(b) || a.before(self, b))
        {
            return (Some(a), Some(b));
        }
        (None, None)
    }
    /// Iterate cells in an inclusive tagged range; None means the region's end.
    pub fn cell_iterator(
        &self,
        direction: Direction,
        start: Point,
        end: Option<Point>,
    ) -> CellIterator<'_> {
        let (front, back) = self.iter_bounds(start, end);
        CellIterator {
            list: self,
            front,
            back,
            direction,
        }
    }
    /// Iterate rows in an inclusive tagged range, ignoring endpoint columns.
    pub fn row_iterator(
        &self,
        direction: Direction,
        start: Point,
        end: Option<Point>,
    ) -> RowIterator<'_> {
        let (front, back) = self.iter_bounds(start, end);
        RowIterator {
            list: self,
            front: front.map(|p| Pin { x: 0, ..p }),
            back: back.map(|p| Pin { x: 0, ..p }),
            direction,
        }
    }
    /// Iterate maximal page chunks in an inclusive tagged range.
    pub fn page_iterator(
        &self,
        direction: Direction,
        start: Point,
        end: Option<Point>,
    ) -> PageIterator<'_> {
        PageIterator {
            rows: self.row_iterator(direction, start, end),
        }
    }
    /// Iterate prompt starts, coalescing continuation rows in either direction.
    pub fn prompt_iterator(
        &self,
        direction: Direction,
        start: Point,
        end: Option<Point>,
    ) -> PromptIterator<'_> {
        let (front, back) = self.iter_bounds(start, end);
        PromptIterator {
            list: self,
            current: if direction == Direction::RightDown {
                front
            } else {
                back
            },
            limit: if direction == Direction::RightDown {
                back
            } else {
                front
            },
            direction,
        }
    }
    /// Locate prompt, input, or output bounds within the prompt starting at `at`.
    pub fn highlight_semantic_content(
        &self,
        at: Pin,
        content: SemanticContent,
    ) -> Option<(Pin, Pin)> {
        let rowstart = Pin { x: 0, ..at };
        let start_pt = self.point_from_pin(Tag::Screen, rowstart)?;
        let mut prompts = self.prompt_iterator(Direction::RightDown, start_pt, None);
        prompts.next()?;
        let end = prompts
            .next()
            .and_then(|p| p.up(self, 1))
            .map(|p| Pin {
                x: self.cols - 1,
                ..p
            })
            .or_else(|| self.get_bottom_right(Tag::Screen))?;
        let mut it = CellIterator {
            list: self,
            front: Some(at),
            back: Some(end),
            direction: Direction::RightDown,
        };
        match content {
            SemanticContent::Prompt => {
                let mut last = at;
                for p in it {
                    if p.row_and_cell(self)?.1.semantic_content() == SemanticContent::Output {
                        break;
                    }
                    last = p;
                }
                Some((rowstart, last))
            }
            SemanticContent::Input => {
                let first = loop {
                    let p = it.next()?;
                    match p.row_and_cell(self)?.1.semantic_content() {
                        SemanticContent::Prompt => {}
                        SemanticContent::Output => return None,
                        SemanticContent::Input => break p,
                    }
                };
                let mut last = first;
                for p in it {
                    match p.row_and_cell(self)?.1.semantic_content() {
                        SemanticContent::Prompt => {}
                        SemanticContent::Output => break,
                        SemanticContent::Input => last = p,
                    }
                }
                Some((first, last))
            }
            SemanticContent::Output => {
                let first = loop {
                    let p = it.next()?;
                    let c = p.row_and_cell(self)?.1;
                    if c.semantic_content() == SemanticContent::Output && c.has_text() {
                        break p;
                    }
                };
                let mut last = first;
                for p in it {
                    let c = p.row_and_cell(self)?.1;
                    if c.semantic_content() != SemanticContent::Output {
                        break;
                    }
                    if c.has_text() {
                        last = p;
                    }
                }
                Some((first, last))
            }
        }
    }
}
/// Iterator over primary prompts and orphaned prompt continuations.
pub struct PromptIterator<'a> {
    list: &'a PageList,
    current: Option<Pin>,
    limit: Option<Pin>,
    direction: Direction,
}
impl Iterator for PromptIterator<'_> {
    type Item = Pin;
    fn next(&mut self) -> Option<Pin> {
        let mut current = self.current?;
        loop {
            let at_limit = self
                .limit
                .is_some_and(|p| p.node == current.node && p.y == current.y);
            let kind = current.row_and_cell(self.list)?.0.semantic_prompt();
            if kind != SemanticPrompt::None {
                let result = Pin { x: 0, ..current };
                if at_limit {
                    self.current = None;
                    return Some(result);
                }
                if self.direction == Direction::RightDown {
                    let mut next = current.down(self.list, 1);
                    while let Some(p) = next {
                        if p.row_and_cell(self.list)?.0.semantic_prompt()
                            != SemanticPrompt::PromptContinuation
                        {
                            self.current = Some(p);
                            return Some(result);
                        }
                        if self.limit.is_some_and(|l| l.node == p.node && l.y == p.y) {
                            break;
                        }
                        next = p.down(self.list, 1);
                    }
                    self.current = None;
                    return Some(result);
                }
                if kind == SemanticPrompt::Prompt {
                    self.current = current.up(self.list, 1);
                    return Some(result);
                }
                let mut end = current;
                while let Some(prior) = end.up(self.list, 1) {
                    if self
                        .limit
                        .is_some_and(|p| p.node == prior.node && p.y == prior.y)
                    {
                        break;
                    }
                    match prior.row_and_cell(self.list)?.0.semantic_prompt() {
                        SemanticPrompt::None => {
                            self.current = Some(prior);
                            return Some(Pin { x: 0, ..end });
                        }
                        SemanticPrompt::Prompt => {
                            self.current = prior.up(self.list, 1);
                            return Some(Pin { x: 0, ..prior });
                        }
                        SemanticPrompt::PromptContinuation => end = prior,
                    }
                }
                self.current = None;
                return Some(result);
            }
            if at_limit {
                self.current = None;
                return None;
            }
            let next = if self.direction == Direction::RightDown {
                current.down(self.list, 1)
            } else {
                current.up(self.list, 1)
            };
            let Some(next) = next else {
                self.current = None;
                return None;
            };
            current = next;
        }
    }
}

/// Capacity inherited by a rewritten page. Oversized source pages cap their
/// inherited row count on reflow to avoid propagating massive empty grids.
fn resized_capacity(page: &Page, cols: u16, reflow: bool) -> Capacity {
    page.capacity
        .adjust(Adjustment { cols: Some(cols) })
        .unwrap_or(Capacity {
            cols,
            rows: page.size.rows.min(if reflow {
                page::STD_CAPACITY.rows
            } else {
                page.capacity.rows
            }),
            ..page.capacity
        })
}

/// Streaming source-to-destination cursor used exclusively during reflow.
/// Unlike a Screen cursor it owns no rendering or terminal state.
struct ReflowCursor {
    node: NodeId,
    x: u16,
    y: u16,
    pending_wrap: bool,
    new_rows: usize,
    total_rows: usize,
}
#[derive(Copy, Clone)]
enum WriteResult {
    Success,
    Repeat,
    SkipNext,
}
impl ReflowCursor {
    fn new(node: NodeId) -> Self {
        Self {
            node,
            x: 0,
            y: 0,
            pending_wrap: false,
            new_rows: 0,
            total_rows: 1,
        }
    }
    fn page<'a>(&self, list: &'a PageList) -> &'a Page {
        &list.node(self.node).expect("reflow destination").data
    }
    fn page_mut<'a>(&self, list: &'a mut PageList) -> &'a mut Page {
        &mut list.node_mut(self.node).expect("reflow destination").data
    }
    fn forward(&mut self, list: &PageList) {
        if self.x == self.page(list).size.cols - 1 {
            self.pending_wrap = true;
        } else {
            self.x += 1;
        }
    }
    fn scroll(&mut self, list: &mut PageList, cap: Capacity) -> Result<(), Error> {
        if self.y == self.page(list).capacity.rows - 1 {
            let node = list.create_node(cap)?;
            list.node_mut(node).ok_or(Error::InvalidId)?.data.size.rows = 1;
            list.insert_after(Some(self.node), node);
            self.node = node;
            self.y = 0;
        } else {
            self.page_mut(list).size.rows += 1;
            self.y += 1;
        }
        self.x = 0;
        self.pending_wrap = false;
        self.total_rows += 1;
        Ok(())
    }
    fn increase_capacity(
        &mut self,
        list: &mut PageList,
        adjustment: Option<IncreaseCapacity>,
    ) -> Result<(), Error> {
        self.node = list.increase_capacity(self.node, adjustment)?;
        self.pending_wrap = false;
        // Zig reinitializes the cursor after replacing the page.
        self.new_rows = 0;
        Ok(())
    }
    fn remap_cell(&self, list: &mut PageList, source: NodeId, y: u16, x: u16) {
        for slot in &mut list.pins {
            if let Some(p) = &mut slot.pin
                && p.node == source
                && p.y == y
                && p.x == x
            {
                p.node = self.node;
                p.x = self.x;
                p.y = self.y;
            }
        }
    }
    fn reflow_row(
        &mut self,
        list: &mut PageList,
        source: NodeId,
        src: &Page,
        y: u16,
    ) -> Result<(), Error> {
        let row = src.rows[usize::from(y)];
        let cells = src
            .get_cells(usize::from(y))
            .ok_or(Error::InvalidCoordinate)?;
        let mut len = src.size.cols;
        if !row.wrap() {
            while len > 0 && cells[usize::from(len - 1)].is_empty() {
                len -= 1;
            }
            if len == 0 && row.semantic_prompt() != SemanticPrompt::None {
                len = 1;
            }
        }
        let remaining = self.page(list).size.cols - 1 - self.x;
        for slot in &mut list.pins {
            if let Some(p) = &mut slot.pin
                && p.node == source
                && p.y == y
            {
                if p.x >= len {
                    p.x = p.x.min(remaining);
                }
                len = len.max(p.x + 1);
            }
        }
        if len == 0 {
            if !row.wrap_continuation() {
                self.new_rows += 1;
            }
            return Ok(());
        }
        let cap = resized_capacity(src, self.page(list).size.cols, true);
        while self.new_rows > 0 {
            self.scroll(list, cap)?;
            self.new_rows -= 1;
        }
        self.page_mut(list).rows[usize::from(self.y)].set_semantic_prompt(row.semantic_prompt());
        let mut x = 0;
        while x < len {
            if self.pending_wrap {
                self.page_mut(list).rows[usize::from(self.y)].set_wrap(true);
                self.scroll(list, cap)?;
                let dst = &mut self.page_mut(list).rows[usize::from(self.y)];
                dst.set_semantic_prompt(row.semantic_prompt());
                dst.set_wrap_continuation(true);
            }
            self.remap_cell(list, source, y, x);
            let index = row.cells() + usize::from(x);
            match self.write_cell(list, src, index) {
                Ok(WriteResult::Success) => x += 1,
                Ok(WriteResult::Repeat) => {}
                Ok(WriteResult::SkipNext) => {
                    self.remap_cell(list, source, y, x + 1);
                    x += 2;
                }
                Err(Error::OutOfSpace) if self.y == 0 => {
                    x += 1;
                    self.forward(list);
                }
                Err(Error::OutOfSpace) => self.move_last_row(list, cap)?,
                Err(error) => return Err(error),
            }
        }
        if !row.wrap() {
            self.new_rows += 1;
        }
        Ok(())
    }
    /// Install unmanaged content first. A capacity failure leaves a valid cell
    /// whose managed parts can be copied to a new page before retrying.
    fn write_cell(
        &mut self,
        list: &mut PageList,
        src: &Page,
        index: usize,
    ) -> Result<WriteResult, Error> {
        let cell = src.cells[index];
        let cols = self.page(list).size.cols;
        let x = usize::from(self.x);
        let y = usize::from(self.y);
        let dst = self.page(list).cell_index(x, y)?;
        if matches!(
            cell.content_tag(),
            ContentTag::BgColorPalette | ContentTag::BgColorRgb
        ) {
            self.page_mut(list).cells[dst] = cell;
            self.forward(list);
            return Ok(WriteResult::Success);
        }
        match cell.wide() {
            Wide::Wide if cols == 1 => {
                let c = &mut self.page_mut(list).cells[dst];
                c.set_codepoint(0);
                c.set_wide(Wide::Narrow);
                self.forward(list);
                return Ok(WriteResult::SkipNext);
            }
            Wide::Wide if self.x == cols - 1 => {
                let mut spacer = Cell::default();
                spacer.set_wide(Wide::SpacerHead);
                self.page_mut(list).cells[dst] = spacer;
                self.forward(list);
                return Ok(WriteResult::Repeat);
            }
            Wide::SpacerHead => return Ok(WriteResult::Success),
            Wide::SpacerTail if cols == 1 => return Ok(WriteResult::Success),
            _ => {}
        }
        // Only a retry can overwrite owned data. Ordinary reflow writes into
        // fresh cells, avoiding both cleanup and whole-row metadata scans.
        let previous = self.page(list).cells[dst];
        if previous.has_grapheme() || previous.hyperlink() || previous.has_styling() {
            let dirty = self.page(list).rows[y].dirty();
            self.page_mut(list).clear_cells(y, x, x + 1, false)?;
            self.page_mut(list).rows[y].set_dirty(dirty);
        }
        let target = &mut self.page_mut(list).cells[dst];
        *target = cell;
        target.set_content_tag(ContentTag::Codepoint);
        target.set_hyperlink(false);
        target.set_style_id(0);
        if cell.codepoint() == 0x10eeee {
            self.page_mut(list).rows[y].set_kitty_virtual_placeholder(true);
        }
        if cell.has_grapheme() {
            let cps = src.lookup_grapheme(index).ok_or(Error::InvalidId)?;
            if self.page(list).grapheme_count() >= self.page(list).grapheme_capacity() {
                self.increase_capacity(list, Some(IncreaseCapacity::GraphemeBytes))?;
            }
            while !self.page_mut(list).can_allocate_grapheme(cps.len() * 4) {
                self.increase_capacity(list, Some(IncreaseCapacity::GraphemeBytes))?;
            }
            loop {
                let dirty = self.page(list).rows[y].dirty();
                let result = self.page_mut(list).set_graphemes(x, y, cps);
                self.page_mut(list).rows[y].set_dirty(dirty);
                match result {
                    Ok(()) => break,
                    Err(Error::GraphemeMapOutOfMemory | Error::GraphemeAllocOutOfMemory) => {
                        self.increase_capacity(list, Some(IncreaseCapacity::GraphemeBytes))?;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        if cell.hyperlink() {
            loop {
                match self.page_mut(list).clone_hyperlink_from(src, index, x, y) {
                    Ok(()) => break,
                    Err(Error::HyperlinkMapOutOfMemory | Error::SetOutOfMemory) => {
                        self.increase_capacity(list, Some(IncreaseCapacity::HyperlinkBytes))?;
                    }
                    Err(Error::StringsOutOfMemory) => {
                        self.increase_capacity(list, Some(IncreaseCapacity::StringBytes))?;
                    }
                    Err(Error::SetNeedsRehash) => self.increase_capacity(list, None)?,
                    Err(error) => return Err(error),
                }
            }
        }
        if cell.has_styling() {
            let style = *src.styles.get(cell.style_id()).ok_or(Error::InvalidId)?;
            let id = loop {
                match self
                    .page_mut(list)
                    .styles
                    .add_with_id(style, cell.style_id())
                {
                    Ok(id) => break id.unwrap_or(cell.style_id()),
                    Err(crate::ref_counted_set::AddError::OutOfMemory) => {
                        self.increase_capacity(list, Some(IncreaseCapacity::Styles))?;
                    }
                    Err(crate::ref_counted_set::AddError::NeedsRehash) => {
                        self.increase_capacity(list, None)?;
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            let page = self.page_mut(list);
            let dst = page.cell_index(x, y)?;
            page.cells[dst].set_style_id(id);
            page.rows[y].set_styled(true);
        }
        self.forward(list);
        Ok(WriteResult::Success)
    }
    /// Split off the current final row and resume at its previous column.
    fn move_last_row(&mut self, list: &mut PageList, cap: Capacity) -> Result<(), Error> {
        let old_node = self.node;
        let old_y = self.y;
        let node = list.create_node(cap)?;
        list.node_mut(node).ok_or(Error::InvalidId)?.data.size.rows = 1;
        list.insert_after(Some(old_node), node);
        list.copy_row(old_node, old_y, node, 0)?;
        list.remap_rows(old_node, old_y, old_y + 1, node, 0);
        let old = &mut list.node_mut(old_node).ok_or(Error::InvalidId)?.data;
        old.clear_cells(usize::from(old_y), 0, usize::from(old.size.cols), false)?;
        old.size.rows -= 1;
        if old.size.rows == 0 {
            let old = list.detach(old_node).ok_or(Error::InvalidId)?;
            list.recycle(old);
        }
        self.node = node;
        self.y = 0;
        self.pending_wrap = false;
        Ok(())
    }
}

impl PageList {
    /// Trim trailing rows without text, stopping at any tracked location.
    fn trim_blank_rows(&mut self, limit: usize) -> Result<(), Error> {
        for _ in 0..limit {
            let Some(node) = self.last else {
                break;
            };
            let page = &self.node(node).ok_or(Error::InvalidId)?.data;
            let y = page.size.rows - 1;
            if Cell::has_text_any(
                page.get_cells(usize::from(y))
                    .ok_or(Error::InvalidCoordinate)?,
            ) || self.tracked_pins().any(|(_, p)| p.node == node && p.y == y)
            {
                break;
            }
            let page = &mut self.node_mut(node).ok_or(Error::InvalidId)?.data;
            page.size.rows -= 1;
            self.total_rows -= 1;
            if y == 0 {
                let old = self.detach(node).ok_or(Error::InvalidId)?;
                self.recycle(old);
            }
        }
        Ok(())
    }
    /// Resize the active area and optionally reflow wrapped logical lines.
    /// Column growth unwraps before resizing rows; column shrink resizes rows
    /// first so wrapping has the correct active height available.
    pub fn resize(&mut self, opts: Resize) -> Result<(), Error> {
        let cols = opts.cols.unwrap_or(self.cols);
        let rows = opts.rows.unwrap_or(self.rows);
        if cols == 0 || rows == 0 {
            return Err(Error::InvalidDimensions);
        }
        self.invalidate_viewport_offset();
        self.min_max_size = min_max_size(cols, rows);
        if !opts.reflow || cols == self.cols {
            self.resize_without_reflow(opts)?;
        } else if cols > self.cols {
            self.resize_cols(cols, opts.cursor)?;
            self.resize_without_reflow(opts)?;
        } else {
            self.resize_without_reflow(Resize {
                cols: Some(self.cols),
                ..opts
            })?;
            self.resize_cols(cols, opts.cursor)?;
        }
        if self.viewport == Viewport::Pin {
            self.fixup_viewport();
        }
        Ok(())
    }
    fn resize_without_reflow(&mut self, opts: Resize) -> Result<(), Error> {
        if let Some(cols) = opts.cols {
            if cols < self.cols {
                let mut node = self.first;
                while let Some(id) = node {
                    let n = self.node_mut(id).ok_or(Error::InvalidId)?;
                    node = n.next;
                    for y in 0..usize::from(n.data.size.rows) {
                        n.data.clear_cells(
                            y,
                            usize::from(cols),
                            usize::from(n.data.size.cols),
                            false,
                        )?;
                    }
                    n.data.size.cols = cols;
                }
                for slot in &mut self.pins {
                    if let Some(p) = &mut slot.pin {
                        p.x = p.x.min(cols - 1);
                    }
                }
                self.cols = cols;
            } else if cols > self.cols {
                let mut node = self.first;
                self.cols = cols;
                while let Some(id) = node {
                    node = self.node(id).ok_or(Error::InvalidId)?.next;
                    self.resize_without_reflow_grow_cols(id, cols)?;
                }
            }
        }
        if let Some(rows) = opts.rows {
            if rows < self.rows {
                self.trim_blank_rows(usize::from(self.rows - rows))?;
                self.rows = rows;
            } else if rows > self.rows {
                let old_rows = self.rows;
                self.rows = rows;
                if opts.cursor.is_some_and(|c| c.y < old_rows - 1) {
                    for _ in old_rows..rows {
                        self.grow()?;
                    }
                } else {
                    while self.total_rows < usize::from(rows) {
                        self.grow()?;
                    }
                    if self.viewport == Viewport::Pin {
                        self.fixup_viewport();
                    }
                }
            }
        }
        Ok(())
    }
    /// Clone a complete row between two page locations, ignoring columns.
    pub fn clone_row_between(&mut self, src: Pin, dst: Pin) -> Result<(), Error> {
        if src.node == dst.node {
            let cols = usize::from(self.node(src.node).ok_or(Error::InvalidId)?.data.size.cols);
            return self.clone_partial_row_between(src, dst, 0, cols);
        }
        self.copy_row(src.node, src.y, dst.node, dst.y)
    }

    /// Clone columns `[left, right)` between rows without a temporary row copy.
    pub fn clone_partial_row_between(
        &mut self,
        src: Pin,
        dst: Pin,
        left: usize,
        right: usize,
    ) -> Result<(), Error> {
        let len = right.checked_sub(left).ok_or(Error::InvalidCoordinate)?;
        if src.node == dst.node {
            let page = &mut self.node_mut(src.node).ok_or(Error::InvalidId)?.data;
            return page.clone_partial_row_within(
                usize::from(src.y),
                usize::from(dst.y),
                left,
                len,
            );
        }
        let (source, dest) = self.pages_for_copy(src.node, dst.node)?;
        dest.clone_partial_row_from(
            source,
            usize::from(src.y),
            usize::from(dst.y),
            left,
            left,
            len,
        )
    }

    /// Borrow distinct source and destination pages without snapshots or unsafe
    /// pointers. Their slab slots remain stable for the duration of the copy.
    fn copy_row(&mut self, src: NodeId, src_y: u16, dst: NodeId, dst_y: u16) -> Result<(), Error> {
        let (source, dest) = self.pages_for_copy(src, dst)?;
        dest.clone_row_from(source, usize::from(src_y), usize::from(dst_y))
    }
    fn pages_for_copy(&mut self, src: NodeId, dst: NodeId) -> Result<(&Page, &mut Page), Error> {
        if self.node(src).is_none() || self.node(dst).is_none() || src.index == dst.index {
            return Err(Error::InvalidId);
        }
        let (source, dest) = if src.index < dst.index {
            let (left, right) = self.nodes.split_at_mut(dst.index);
            (left[src.index].as_ref(), right[0].as_mut())
        } else {
            let (left, right) = self.nodes.split_at_mut(src.index);
            (right[0].as_ref(), left[dst.index].as_mut())
        };
        Ok((
            &source.ok_or(Error::InvalidId)?.data,
            &mut dest.ok_or(Error::InvalidId)?.data,
        ))
    }
    /// Widen one page, first backfilling the previously rewritten page to avoid
    /// permanently stranding row capacity between page boundaries.
    fn resize_without_reflow_grow_cols(&mut self, source: NodeId, cols: u16) -> Result<(), Error> {
        let src = &self.node(source).ok_or(Error::InvalidId)?.data;
        if src.capacity.cols >= cols
            && (0..usize::from(src.size.rows)).all(|y| {
                src.get_cells(y).expect("source row")[usize::from(src.size.cols - 1)].wide()
                    != Wide::SpacerHead
            })
        {
            self.node_mut(source)
                .ok_or(Error::InvalidId)?
                .data
                .size
                .cols = cols;
            return Ok(());
        }
        let cap = resized_capacity(src, cols, false);
        let rows = src.size.rows;
        let previous = self.node(source).ok_or(Error::InvalidId)?.prev;
        let mut copied = 0;
        if let Some(prev) = previous {
            let page = &self.node(prev).ok_or(Error::InvalidId)?.data;
            let start = page.size.rows;
            let len = (page.capacity.rows - start).min(rows);
            for y in 0..len {
                self.node_mut(prev).ok_or(Error::InvalidId)?.data.size.rows += 1;
                if self.copy_row(source, y, prev, start + y).is_err() {
                    let page = &mut self.node_mut(prev).ok_or(Error::InvalidId)?.data;
                    page.clear_cells(usize::from(start + y), 0, usize::from(cols), false)?;
                    page.size.rows -= 1;
                    break;
                }
                copied += 1;
            }
            self.remap_rows(source, 0, copied, prev, start);
        }
        while copied < rows {
            let id = self.create_node(cap)?;
            let start = copied;
            let len = cap.rows.min(rows - copied);
            self.insert_after(self.node(source).ok_or(Error::InvalidId)?.prev, id);
            for y in 0..len {
                self.node_mut(id).ok_or(Error::InvalidId)?.data.size.rows += 1;
                self.copy_row(source, copied, id, y)?;
                copied += 1;
            }
            self.remap_rows(source, start, copied, id, 0);
        }
        let old = self.detach(source).ok_or(Error::InvalidId)?;
        self.recycle(old);
        Ok(())
    }
    fn remap_rows(&mut self, source: NodeId, start: u16, end: u16, dest: NodeId, dest_y: u16) {
        for slot in &mut self.pins {
            if let Some(p) = &mut slot.pin
                && p.node == source
                && p.y >= start
                && p.y < end
            {
                p.node = dest;
                p.y = p.y - start + dest_y;
            }
        }
    }
    fn wrapped_rows_above(&self, pin: Pin) -> usize {
        let Some(top) = self.get_top_left(Tag::Active) else {
            return 0;
        };
        let mut current = Some(pin);
        let mut count = 0;
        while let Some(p) = current {
            if p.row_and_cell(self)
                .is_some_and(|(r, _)| r.wrap_continuation())
            {
                count += 1;
            }
            if p.node == top.node && p.y == top.y {
                break;
            }
            current = p.up(self, 1);
        }
        count
    }
    /// Rewrite incrementally, releasing each source page as soon as it has been
    /// consumed. Pending blank rows are only materialized before later content.
    fn resize_cols(&mut self, cols: u16, cursor: Option<Cursor>) -> Result<(), Error> {
        self.cols = cols;
        let preserved = if let Some(c) = cursor {
            if let Some(p) = self.pin(Point::Active(Coordinate {
                x: c.x,
                y: u32::from(c.y),
            })) {
                let wrapped = self.wrapped_rows_above(p);
                Some((
                    self.track_pin(p)?,
                    usize::from(self.rows - c.y - 1),
                    wrapped,
                ))
            } else {
                None
            }
        } else {
            None
        };
        let result = self.reflow_pages(cols);
        if let Err(error) = result {
            if let Some((handle, _, _)) = preserved {
                self.untrack_pin(handle);
            }
            return Err(error);
        }
        while self.total_rows < usize::from(self.rows) {
            self.grow()?;
        }
        if let Some((handle, remaining, old_wrapped)) = preserved {
            let p = self.tracked_pin(handle).ok_or(Error::InvalidId)?;
            if let Some(point) = self.point_from_pin(Tag::Active, p) {
                let wrapped = self.wrapped_rows_above(p);
                let current = usize::from(self.rows) - point.coord().y as usize - 1;
                let required = remaining
                    .saturating_sub(wrapped.saturating_sub(old_wrapped))
                    .saturating_sub(current);
                for _ in 0..required {
                    self.grow()?;
                }
            }
            self.untrack_pin(handle);
        }
        Ok(())
    }
    fn reflow_pages(&mut self, cols: u16) -> Result<(), Error> {
        let mut old = self.first;
        let first = old.ok_or(Error::InvalidId)?;
        let cap = resized_capacity(&self.node(first).ok_or(Error::InvalidId)?.data, cols, false);
        let rewritten = self.create_node(cap)?;
        self.node_mut(rewritten)
            .ok_or(Error::InvalidId)?
            .data
            .size
            .rows = 1;
        self.first = Some(rewritten);
        self.last = Some(rewritten);
        let mut cursor = ReflowCursor::new(rewritten);
        while let Some(id) = old {
            // Taking a source node out of the slab permits borrowing its page
            // while allocating or replacing destination pages in the list.
            let source = self.nodes[id.index].take().ok_or(Error::InvalidId)?;
            old = source.next;
            let result = (0..source.data.size.rows)
                .try_for_each(|y| cursor.reflow_row(self, id, &source.data, y));
            self.page_size -= allocated_size(source.data.capacity);
            self.free_nodes.push(id.index);
            self.recycle(source);
            if let Err(error) = result {
                while let Some(id) = old {
                    let source = self.nodes[id.index].take().ok_or(Error::InvalidId)?;
                    old = source.next;
                    self.page_size -= allocated_size(source.data.capacity);
                    self.free_nodes.push(id.index);
                    self.recycle(source);
                }
                return Err(error);
            }
        }
        self.total_rows = cursor.total_rows;
        Ok(())
    }
    /// Clone inclusive whole rows, preserving source page capacities and
    /// boundaries. Pins in copied rows receive independent tracked handles.
    pub fn clone_range(
        &self,
        start: Point,
        end: Option<Point>,
    ) -> Result<(Self, HashMap<TrackedPin, TrackedPin>), Error> {
        let mut result = Self::init(self.cols, self.rows, Some(self.explicit_max_size))?;
        while let Some(id) = result.first {
            let node = result.detach(id).ok_or(Error::InvalidId)?;
            // Freshly initialised and never written: reuse without re-zeroing.
            result.clean_pool.push(node.data);
        }
        result.page_serial = 0;
        result.total_rows = 0;
        result.min_max_size = self.min_max_size;
        let mut remap = HashMap::new();
        for chunk in self.page_iterator(Direction::RightDown, start, end) {
            let source = &self.node(chunk.node).ok_or(Error::InvalidId)?.data;
            let node = result.create_node(source.capacity)?;
            let dest = &mut result.node_mut(node).ok_or(Error::InvalidId)?.data;
            dest.size.cols = source.size.cols;
            dest.size.rows = chunk.end - chunk.start;
            dest.clone_from(source, usize::from(chunk.start), usize::from(chunk.end))?;
            dest.dirty = source.dirty;
            result.insert_after(result.last, node);
            result.total_rows += usize::from(chunk.end - chunk.start);
            for (handle, p) in self.tracked_pins() {
                if Some(handle) == self.viewport_pin
                    || p.node != chunk.node
                    || p.y < chunk.start
                    || p.y >= chunk.end
                {
                    continue;
                }
                remap.insert(
                    handle,
                    result.track_pin(Pin {
                        node,
                        y: p.y - chunk.start,
                        ..p
                    })?,
                );
            }
        }
        let first = result.first.ok_or(Error::InvalidCoordinate)?;
        let viewport = result.viewport_pin.ok_or(Error::InvalidId)?;
        result.pins[viewport.index].pin = Some(Pin {
            node: first,
            x: 0,
            y: 0,
            garbage: false,
        });
        while result.total_rows < usize::from(result.rows) {
            result.grow()?;
        }
        Ok((result, remap))
    }
    /// Fallibly clone the whole screen with independently owned tracked handles.
    pub fn try_clone(&self) -> Result<Self, Error> {
        let (mut result, remap) = self.clone_range(Point::Screen(Coordinate::default()), None)?;
        for handle in remap.into_values() {
            result.untrack_pin(handle);
        }
        Ok(result)
    }
    /// Debug-friendly UTF-8 rows, omitting wide spacers and rendering blanks as spaces.
    pub fn diagram(&self) -> String {
        let mut s = String::new();
        for p in self.row_iterator(
            Direction::RightDown,
            Point::Screen(Coordinate::default()),
            None,
        ) {
            if let Some(n) = self.node(p.node)
                && let Some(cells) = n.data.get_cells(usize::from(p.y))
            {
                for c in cells {
                    if matches!(c.wide(), Wide::SpacerHead | Wide::SpacerTail) {
                        continue;
                    }
                    s.push(
                        char::from_u32(c.codepoint())
                            .filter(|c| *c != '\0')
                            .unwrap_or(' '),
                    );
                }
                s.push('\n');
            }
        }
        s
    }
}
#[cfg(any(debug_assertions, test))]
impl PageList {
    /// Verify list links, byte/row accounting, every page, and all tracked pins.
    pub fn verify_integrity(&self) -> Result<(), String> {
        if self.pause_checks > 0 {
            return Ok(());
        }
        let mut prev = None;
        let mut id = self.first;
        let mut rows = 0;
        let mut bytes = 0;
        let mut count = 0;
        while let Some(i) = id {
            let n = self.node(i).ok_or("stale node")?;
            if n.prev != prev {
                return Err("broken previous link".into());
            }
            n.data
                .verify_integrity()
                .map_err(|e| format!("page: {e:?}"))?;
            rows += usize::from(n.data.size.rows);
            bytes += allocated_size(n.data.capacity);
            count += 1;
            if count > self.nodes.len() {
                return Err("cycle".into());
            }
            prev = id;
            id = n.next;
        }
        if prev != self.last
            || rows != self.total_rows
            || bytes != self.page_size
            || count != self.total_pages()
        {
            return Err("list accounting mismatch".into());
        }
        if rows < usize::from(self.rows) {
            return Err("active rows missing".into());
        }
        for (_, p) in self.tracked_pins() {
            if !self.pin_is_valid(p) {
                return Err("invalid tracked pin".into());
            }
        }
        let cached = self.viewport_pin_row_offset.load(Ordering::Relaxed);
        if self.viewport == Viewport::Pin && cached != usize::MAX {
            let pin = self
                .viewport_pin
                .and_then(|h| self.tracked_pin(h))
                .ok_or("missing viewport pin")?;
            if self.screen_row(pin) != Some(cached) {
                return Err("stale viewport row offset".into());
            }
        }
        Ok(())
    }
    /// Assert all list invariants in debug/test builds.
    pub fn assert_integrity(&self) {
        assert_eq!(self.verify_integrity(), Ok(()));
    }
}

fn double16(v: u16) -> Result<u16, Error> {
    if v == u16::MAX {
        return Err(Error::OutOfSpace);
    }
    Ok(v.saturating_mul(2))
}

fn double32(v: u32) -> Result<u32, Error> {
    if v == u32::MAX {
        return Err(Error::OutOfSpace);
    }
    Ok(v.saturating_mul(2))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn screen(x: u16, y: u32) -> Point {
        Point::Screen(Coordinate { x, y })
    }
    fn active(x: u16, y: u32) -> Point {
        Point::Active(Coordinate { x, y })
    }
    fn grow(s: &mut PageList, n: usize) {
        for _ in 0..n {
            s.grow().unwrap();
        }
    }
    fn write_rows(s: &mut PageList) {
        for y in 0..s.total_rows {
            *s.get_cell_mut(screen(0, y as u32)).unwrap() = Cell::init(y as u32);
        }
    }
    fn finish(s: &PageList) {
        s.assert_integrity();
    }
    fn first_cap(s: &PageList) -> usize {
        usize::from(s.node(s.first.unwrap()).unwrap().data.capacity.rows)
    }
    fn fill_pages(s: &mut PageList, n: usize) {
        let target = first_cap(s) * n;
        grow(s, target - s.total_rows);
    }
    #[test]
    fn page_list() {
        let mut s = PageList::init(80, 24, None).unwrap();
        assert_eq!(s.viewport, Viewport::Active);
        assert!(s.first.is_some());
        assert_eq!(s.total_rows, usize::from(s.rows));
        assert_eq!(
            s.tracked_pin(s.viewport_pin.unwrap()).unwrap().node,
            s.first.unwrap()
        );
        assert_eq!(
            s.get_top_left(Tag::Active),
            Some(Pin {
                node: s.first.unwrap(),
                x: 0,
                y: 0,
                garbage: false
            })
        );
        assert_scrollbar(&s, 0);
        // Keep a mutable binding to mirror construction/destruction ownership.
        s.clear_dirty();
    }
    #[test]
    fn page_list_init_error() {
        use crate::tripwire::ResetMode;
        // Test every failure point in `init` and ensure we exit early
        // without panicking or leaving the tripwire armed.
        for tag in [
            InitPoint::InitPages,
            InitPoint::ViewportPin,
            InitPoint::ViewportPinTrack,
        ] {
            init_tw::error_always(tag, Error::OutOfMemory);
            assert_eq!(
                Err(Error::OutOfMemory),
                PageList::init(80, 24, None).map(drop)
            );
            init_tw::end(ResetMode::Reset).unwrap();
        }

        // init creates pages transitively, so check each failure there too.
        let big = page::STD_CAPACITY.max_cols().unwrap() + 1;
        for tag in [
            InitPagesPoint::Node,
            InitPagesPoint::BufStd,
            InitPagesPoint::BufNonStd,
        ] {
            init_pages_tw::error_always(tag, Error::OutOfMemory);
            let cols = if tag == InitPagesPoint::BufStd {
                80
            } else {
                big
            };
            assert_eq!(
                Err(Error::OutOfMemory),
                PageList::init(cols, 24, None).map(drop)
            );
            init_pages_tw::end(ResetMode::Reset).unwrap();
        }

        // Non-standard pages: fail on the second one.
        init_pages_tw::error_after(InitPagesPoint::BufNonStd, Error::OutOfMemory, 1);
        assert_eq!(
            Err(Error::OutOfMemory),
            PageList::init(big, page::STD_CAPACITY.rows + 1, None).map(drop)
        );
        init_pages_tw::end(ResetMode::Reset).unwrap();

        // And with nothing armed init still works.
        PageList::init(80, 24, None).unwrap();
    }
    #[test]
    fn page_list_init_rows_across_two_pages() {
        let mut cols = 50;
        while initial_capacity(cols).rows >= 100 {
            cols += 50;
        }
        let s = PageList::init(cols, 100, None).unwrap();
        assert_eq!(s.viewport, Viewport::Active);
        assert!(s.first.is_some());
        assert_eq!(s.total_rows, usize::from(s.rows));
        assert_scrollbar(&s, 0);
    }
    #[test]
    fn page_list_init_more_than_max_cols() {
        let s = PageList::init(page::STD_CAPACITY.max_cols().unwrap() + 1, 80, None).unwrap();
        assert_eq!(s.viewport, Viewport::Active);
        assert_eq!(s.total_rows, usize::from(s.rows));
        assert!(s.first.is_some());
        assert!(
            Page::layout(s.node(s.first.unwrap()).unwrap().data.capacity).total_size > std_size()
        );
        assert_scrollbar(&s, 0);
    }
    #[test]
    fn page_list_point_from_pin_active_no_history() {
        let s = PageList::init(80, 24, None).unwrap();
        for (x, y) in [(0, 0), (4, 2)] {
            let p = s.pin(screen(x, y)).unwrap();
            assert_eq!(s.point_from_pin(Tag::Active, p), Some(active(x, y)));
        }
    }
    #[test]
    fn page_list_point_from_pin_active_with_history() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 30);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.pin(screen(2, 30)).unwrap()),
            Some(active(2, 0))
        );
        assert_eq!(
            s.point_from_pin(Tag::Active, s.pin(screen(2, 21)).unwrap()),
            None
        );
    }
    #[test]
    fn page_list_point_from_pin_active_from_prior_page() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let cap = first_cap(&s);
        grow(&mut s, cap * 5);
        let p = Pin {
            node: s.last.unwrap(),
            x: 2,
            y: 0,
            garbage: false,
        };
        assert_eq!(s.point_from_pin(Tag::Active, p), Some(active(2, 0)));
        assert_eq!(
            s.point_from_pin(Tag::Active, s.get_top_left(Tag::Screen).unwrap()),
            None
        );
    }
    #[test]
    fn page_list_point_from_pin_traverse_pages() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let cap = first_cap(&s);
        grow(&mut s, cap * 2);
        let p = Pin {
            node: s.node(s.last.unwrap()).unwrap().prev.unwrap(),
            y: 5,
            x: 2,
            garbage: false,
        };
        assert_eq!(
            s.point_from_pin(Tag::Screen, p),
            Some(screen(2, (cap * (s.total_pages() - 2) + 5) as u32))
        );
        assert!(
            s.point_from_pin(Tag::Active, s.get_top_left(Tag::Screen).unwrap())
                .is_none()
        );
    }
    #[test]
    fn page_list_active_after_grow() {
        let mut s = PageList::init(80, 24, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        grow_rows(&mut s, 10);
        assert_eq!(s.total_rows, usize::from(s.rows) + 10);
        assert_viewport_point(&s, 10);
        for (tag, y) in [(Tag::Screen, 0), (Tag::Active, 10)] {
            assert_eq!(
                s.point_from_pin(Tag::Screen, s.get_top_left(tag).unwrap()),
                Some(screen_point(0, y))
            );
        }
        assert_scrollbar(&s, 10);
    }
    #[test]
    fn page_list_scrollbar_with_max_size_0_after_grow() {
        let mut s = PageList::init(80, 24, Some(0)).unwrap();
        grow(&mut s, 10);
        assert_eq!(
            s.scrollbar(),
            Scrollbar {
                total: 24,
                offset: 0,
                len: 24
            }
        );
    }
    #[test]
    fn page_list_scroll_with_max_size_0_no_history() {
        let mut s = PageList::init(80, 24, Some(0)).unwrap();
        grow(&mut s, 10);
        let before = s.get_top_left(Tag::Viewport);
        s.scroll(Scroll::DeltaRow(-5));
        assert_eq!(s.viewport, Viewport::Active);
        s.scroll(Scroll::Top);
        assert_eq!(s.get_top_left(Tag::Viewport), before);
    }
    #[test]
    fn page_list_scroll_top() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 10);
        s.scroll(Scroll::Top);
        assert_viewport_point(&s, 0);
        assert_scrollbar(&s, 0);
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 0);
        assert_scrollbar(&s, 0);
        s.scroll(Scroll::Active);
        assert_viewport_point(&s, 20);
        assert_scrollbar(&s, 20);
    }
    #[test]
    fn page_list_scroll_delta_row_back() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 10);
        s.scroll(Scroll::DeltaRow(-1));
        assert_viewport_point(&s, 9);
        assert_scrollbar(&s, 9);
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 9);
        assert_scrollbar(&s, 9);
        s.scroll(Scroll::DeltaRow(-1));
        assert_scrollbar(&s, 8);
    }
    #[test]
    fn page_list_scroll_delta_row_back_overflow() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 10);
        s.scroll(Scroll::DeltaRow(-100));
        assert_viewport_point(&s, 0);
        assert_scrollbar(&s, 0);
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 0);
        assert_scrollbar(&s, 0);
    }
    #[test]
    fn page_list_scroll_delta_row_forward() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 10);
        s.scroll(Scroll::Top);
        s.scroll(Scroll::DeltaRow(2));
        assert_viewport_point(&s, 2);
        assert_scrollbar(&s, 2);
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 2);
        assert_scrollbar(&s, 2);
    }
    #[test]
    fn page_list_scroll_delta_row_forward_into_active() {
        let mut s = PageList::init(80, 24, None).unwrap();
        s.scroll(Scroll::DeltaRow(2));
        assert_viewport_point(&s, 0);
        assert_scrollbar(&s, 0);
    }
    #[test]
    fn page_list_scroll_to_pin() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 10);
        for y in [4, 5] {
            s.scroll(Scroll::Pin(s.pin(screen_point(2, y)).unwrap()));
            assert_scrollbar(&s, y);
            assert_viewport_point(&s, y);
        }
    }
    #[test]
    fn page_list_scroll_to_pin_in_active() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 10);
        s.scroll(Scroll::Pin(s.pin(screen_point(2, 30)).unwrap()));
        assert_viewport_point(&s, 10);
        assert_scrollbar(&s, 10);
    }
    #[test]
    fn page_list_scroll_to_pin_at_top() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 10);
        s.scroll(Scroll::Pin(s.pin(screen_point(2, 0)).unwrap()));
        assert_eq!(s.viewport, Viewport::Top);
        assert_viewport_point(&s, 0);
        assert_scrollbar(&s, 0);
    }
    #[test]
    fn page_list_scroll_to_row_0() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 10);
        let target = 0;
        s.scroll(Scroll::Row(target));
        assert_eq!(s.viewport, Viewport::Top);
        assert_viewport_point(&s, target);
        assert_scrollbar(&s, target);
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, target);
        assert_scrollbar(&s, target);
    }
    #[test]
    fn page_list_scroll_to_row_in_scrollback() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 20);
        assert_viewport_point(&s, 20);
        let target = 5;
        s.scroll(Scroll::Row(target));
        assert_eq!(s.viewport, Viewport::Pin);
        assert_viewport_point(&s, target);
        assert_scrollbar(&s, target);
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, target);
        assert_scrollbar(&s, target);
    }
    #[test]
    fn page_list_scroll_to_row_in_middle() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 50);
        let target = s.total_rows / 2;
        s.scroll(Scroll::Row(target));
        assert_eq!(s.viewport, Viewport::Pin);
        assert_viewport_point(&s, target);
        assert_scrollbar(&s, target);
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, target);
        assert_scrollbar(&s, target);
    }
    #[test]
    fn page_list_scroll_to_row_at_active_boundary() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 20);
        s.scroll(Scroll::Row(s.total_rows - usize::from(s.rows)));
        assert_eq!(s.viewport, Viewport::Active);
        assert_viewport_point(&s, 20);
        assert_scrollbar(&s, 20);
        grow_rows(&mut s, 10);
        assert_eq!(s.viewport, Viewport::Active);
        assert_scrollbar(&s, s.total_rows - usize::from(s.rows));
    }
    #[test]
    fn page_list_scroll_to_row_beyond_active() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 10);
        s.scroll(Scroll::Row(1000));
        assert_eq!(s.viewport, Viewport::Active);
        assert_viewport_point(&s, 10);
        assert_scrollbar(&s, 10);
    }
    #[test]
    fn page_list_scroll_to_row_without_scrollback() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 0);
        s.scroll(Scroll::Row(5));
        assert_eq!(s.viewport, Viewport::Active);
        assert_viewport_point(&s, 0);
        assert_scrollbar(&s, 0);
    }
    #[test]
    fn page_list_scroll_to_row_then_delta() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 30);
        for (scroll, offset) in [
            (Scroll::Row(10), 10),
            (Scroll::DeltaRow(5), 15),
            (Scroll::DeltaRow(-3), 12),
        ] {
            s.scroll(scroll);
            assert_eq!(s.viewport, Viewport::Pin);
            assert_viewport_point(&s, offset);
            assert_scrollbar(&s, offset);
        }
    }
    #[test]
    fn page_list_scroll_to_row_with_cache_fast_path_down() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 50);
        s.scroll(Scroll::Row(10));
        assert_eq!(s.viewport, Viewport::Pin);
        assert_viewport_point(&s, 10);
        assert_scrollbar(&s, 10);
        assert_ne!(
            s.viewport_pin_row_offset.load(Ordering::Relaxed),
            usize::MAX
        );
        assert_eq!(s.viewport_pin_row_offset.load(Ordering::Relaxed), 10);
        s.scroll(Scroll::Row(20));
        assert_eq!(s.viewport, Viewport::Pin);
        assert_viewport_point(&s, 20);
        assert_scrollbar(&s, 20);
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 20);
        assert_scrollbar(&s, 20);
    }
    #[test]
    fn page_list_scroll_to_row_with_cache_fast_path_up() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow_rows(&mut s, 50);
        s.scroll(Scroll::Row(30));
        assert_eq!(s.viewport, Viewport::Pin);
        assert_viewport_point(&s, 30);
        assert_scrollbar(&s, 30);
        assert_ne!(
            s.viewport_pin_row_offset.load(Ordering::Relaxed),
            usize::MAX
        );
        assert_eq!(s.viewport_pin_row_offset.load(Ordering::Relaxed), 30);
        s.scroll(Scroll::Row(15));
        assert_eq!(s.viewport, Viewport::Pin);
        assert_viewport_point(&s, 15);
        assert_scrollbar(&s, 15);
        grow_rows(&mut s, 10);
        assert_viewport_point(&s, 15);
        assert_scrollbar(&s, 15);
    }
    #[test]
    fn page_list_grow_fit_in_capacity() {
        let mut s = PageList::init(80, 24, None).unwrap();
        assert!(first_cap(&s) > 24);
        assert_eq!(s.grow().unwrap(), None);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen(0, 1))
        );
    }
    #[test]
    fn page_list_grow_allocate() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let old = s.last.unwrap();
        let capacity = s.node(old).unwrap().data.capacity.rows;
        for _ in s.rows..capacity {
            assert!(s.grow().unwrap().is_none());
        }
        let new = s.grow().unwrap().unwrap();
        assert_eq!(s.last, Some(new));
        assert_eq!(s.node(old).unwrap().next, Some(new));
        let p = s.pin(active_point(0, usize::from(s.rows - 1))).unwrap();
        assert_eq!(p.node, new);
        assert_eq!(
            s.point_from_pin(Tag::Screen, p),
            Some(screen_point(0, usize::from(capacity)))
        );
    }
    #[test]
    fn page_list_grow_prune_scrollback() {
        let mut s = PageList::init(80, 24, Some(std_size())).unwrap();
        let cap = first_cap(&s);
        fill_pages(&mut s, 2);
        let old = s.first.unwrap();
        let second = s.node(old).unwrap().next.unwrap();
        let bytes = s.page_size;
        let h = s.track_pin(s.pin(screen(0, 0)).unwrap()).unwrap();
        let y = cap / 2;
        s.scroll(Scroll::Row(y));
        assert_eq!(s.viewport, Viewport::Pin);
        assert_eq!(s.scrollbar().offset, y);
        let viewport_node = s.tracked_pin(s.viewport_pin.unwrap()).unwrap().node;
        assert_eq!(viewport_node, old);
        let new = s.grow().unwrap().unwrap();
        assert_eq!(s.last, Some(new));
        assert_eq!(new.index(), old.index());
        assert_ne!(new.generation(), old.generation());
        assert_eq!(s.page_size, bytes);
        assert_eq!(s.first, Some(second));
        let p = s.tracked_pin(h).unwrap();
        assert_eq!(p.node, second);
        assert!(p.garbage);
        assert_eq!(p.x, 0);
        assert_eq!(p.y, 0);
        assert_eq!(s.scrollbar().offset, y.saturating_sub(cap));
        assert!(s.node(old).is_none());
        finish(&s);
    }
    #[test]
    fn page_list_grow_prune_scrollback_with_viewport_pin_not_in_pruned_page() {
        let mut s = PageList::init(80, 24, Some(std_size())).unwrap();
        let cap = first_cap(&s);
        fill_pages(&mut s, 2);
        let old = s.first.unwrap();
        let second = s.node(old).unwrap().next.unwrap();
        let bytes = s.page_size;
        let h = s.track_pin(s.pin(screen(0, 0)).unwrap()).unwrap();
        let y = cap + 5;
        s.scroll(Scroll::Row(y));
        assert_eq!(s.viewport, Viewport::Pin);
        assert_eq!(s.scrollbar().offset, y);
        let viewport_node = s.tracked_pin(s.viewport_pin.unwrap()).unwrap().node;
        assert_eq!(viewport_node, second);
        let new = s.grow().unwrap().unwrap();
        assert_eq!(s.last, Some(new));
        assert_eq!(new.index(), old.index());
        assert_ne!(new.generation(), old.generation());
        assert_eq!(s.page_size, bytes);
        assert_eq!(s.first, Some(second));
        let p = s.tracked_pin(h).unwrap();
        assert_eq!(p.node, second);
        assert!(p.garbage);
        assert_eq!(p.x, 0);
        assert_eq!(p.y, 0);
        assert_eq!(s.scrollbar().offset, y.saturating_sub(cap));
        assert_eq!(s.tracked_pin(s.viewport_pin.unwrap()).unwrap().node, second);
        assert!(s.node(old).is_none());
        finish(&s);
    }
    #[test]
    fn page_list_increase_capacity_to_increase_styles() {
        let mut s = PageList::init(2, 2, Some(0)).unwrap();
        let original = s.node(s.first.unwrap()).unwrap().data.capacity.styles;
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        s.increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::Styles))
            .unwrap();
        assert_eq!(s.first, s.last);
        assert_eq!(
            s.node(s.first.unwrap()).unwrap().data.capacity.styles,
            original * 2
        );
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(
                    s.get_cell(active_point(x, y)).unwrap().codepoint(),
                    u32::from(x)
                );
            }
        }
    }
    #[test]
    fn page_list_increase_capacity_to_increase_graphemes() {
        let mut s = PageList::init(2, 2, Some(0)).unwrap();
        let original = s
            .node(s.first.unwrap())
            .unwrap()
            .data
            .capacity
            .grapheme_bytes;
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        s.increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::GraphemeBytes))
            .unwrap();
        assert_eq!(s.first, s.last);
        assert_eq!(
            s.node(s.first.unwrap())
                .unwrap()
                .data
                .capacity
                .grapheme_bytes,
            original * 2
        );
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(
                    s.get_cell(active_point(x, y)).unwrap().codepoint(),
                    u32::from(x)
                );
            }
        }
    }
    #[test]
    fn page_list_increase_capacity_to_increase_hyperlinks() {
        let mut s = PageList::init(2, 2, Some(0)).unwrap();
        let original = s
            .node(s.first.unwrap())
            .unwrap()
            .data
            .capacity
            .hyperlink_bytes;
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        s.increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::HyperlinkBytes))
            .unwrap();
        assert_eq!(s.first, s.last);
        assert_eq!(
            s.node(s.first.unwrap())
                .unwrap()
                .data
                .capacity
                .hyperlink_bytes,
            original * 2
        );
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(
                    s.get_cell(active_point(x, y)).unwrap().codepoint(),
                    u32::from(x)
                );
            }
        }
    }
    #[test]
    fn page_list_increase_capacity_to_increase_string_bytes() {
        let mut s = PageList::init(2, 2, Some(0)).unwrap();
        let original = s.node(s.first.unwrap()).unwrap().data.capacity.string_bytes;
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        s.increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::StringBytes))
            .unwrap();
        assert_eq!(s.first, s.last);
        assert_eq!(
            s.node(s.first.unwrap()).unwrap().data.capacity.string_bytes,
            original * 2
        );
        for y in 0..2 {
            for x in 0..2 {
                assert_eq!(
                    s.get_cell(active_point(x, y)).unwrap().codepoint(),
                    u32::from(x)
                );
            }
        }
    }
    #[test]
    fn page_list_increase_capacity_tracked_pins() {
        let mut s = PageList::init(2, 2, Some(0)).unwrap();
        let tracked = s.track_pin(s.pin(active_point(1, 1)).unwrap()).unwrap();
        assert_eq!(s.tracked_pin(tracked).unwrap().node, s.first.unwrap());
        let new = s
            .increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::Styles))
            .unwrap();
        let p = s.tracked_pin(tracked).unwrap();
        assert_eq!(p.node, new);
        assert_eq!(p.x, 1);
        assert_eq!(p.y, 1);
    }
    #[test]
    fn page_list_increase_capacity_returns_out_of_space_at_max_capacity() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let id = s.first.unwrap();
        s.node_mut(id).unwrap().data.capacity.styles = u16::MAX;
        assert_eq!(
            s.increase_capacity(id, Some(IncreaseCapacity::Styles)),
            Err(Error::OutOfSpace)
        );
    }
    #[test]
    fn page_list_increase_capacity_preserves_dirty_flag() {
        let mut s = PageList::init(2, 4, Some(0)).unwrap();
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        page.dirty = true;
        for y in 0..4 {
            page.rows[y].set_dirty(y % 2 == 0);
        }
        let new = s
            .increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::Styles))
            .unwrap();
        let page = &s.node(new).unwrap().data;
        assert!(page.dirty);
        for y in 0..4 {
            assert_eq!(page.rows[y].dirty(), y % 2 == 0);
        }
    }
    #[test]
    fn page_list_page_iterator_single_page() {
        let s = PageList::init(80, 24, None).unwrap();
        assert!(s.node(s.first.unwrap()).unwrap().next.is_none());
        let mut it = s.page_iterator(Direction::RightDown, active_point(0, 0), None);
        let chunk = it.next().unwrap();
        assert_eq!(chunk.node, s.first.unwrap());
        assert_eq!(chunk.start, 0);
        assert_eq!(chunk.end, s.rows);
        assert!(it.next().is_none());
    }
    #[test]
    fn page_list_page_iterator_two_pages() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let capacity = s.node(s.first.unwrap()).unwrap().data.capacity.rows;
        for _ in s.rows..capacity {
            assert!(s.grow().unwrap().is_none());
        }
        assert!(s.grow().unwrap().is_some());
        let expected = [
            Chunk {
                node: s.first.unwrap(),
                start: capacity - s.rows + 1,
                end: capacity,
            },
            Chunk {
                node: s.last.unwrap(),
                start: 0,
                end: 1,
            },
        ];
        let mut it = s.page_iterator(Direction::RightDown, active_point(0, 0), None);
        let mut count = 0;
        for index in [0, 1] {
            let chunk = it.next().unwrap();
            assert_eq!(chunk.node, expected[index].node);
            assert_eq!(chunk.start, expected[index].start);
            assert_eq!(chunk.end, expected[index].end);
            count += chunk.end - chunk.start;
        }
        assert!(it.next().is_none());
        assert_eq!(count, s.rows);
    }
    #[test]
    fn page_list_cell_iterator() {
        let mut s = PageList::init(2, 2, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        let mut it = s.cell_iterator(Direction::RightDown, screen_point(0, 0), None);
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            assert_eq!(
                s.point_from_pin(Tag::Screen, it.next().unwrap()),
                Some(screen_point(x, y))
            );
        }
        assert!(it.next().is_none());
    }
    #[test]
    fn page_list_page_iterator_reverse_single_page() {
        let s = PageList::init(80, 24, None).unwrap();
        assert!(s.node(s.first.unwrap()).unwrap().next.is_none());
        let mut it = s.page_iterator(Direction::LeftUp, active_point(0, 0), None);
        let chunk = it.next().unwrap();
        assert_eq!(chunk.node, s.first.unwrap());
        assert_eq!(chunk.start, 0);
        assert_eq!(chunk.end, s.rows);
        assert!(it.next().is_none());
    }
    #[test]
    fn page_list_page_iterator_reverse_two_pages() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let capacity = s.node(s.first.unwrap()).unwrap().data.capacity.rows;
        for _ in s.rows..capacity {
            assert!(s.grow().unwrap().is_none());
        }
        assert!(s.grow().unwrap().is_some());
        let expected = [
            Chunk {
                node: s.first.unwrap(),
                start: capacity - s.rows + 1,
                end: capacity,
            },
            Chunk {
                node: s.last.unwrap(),
                start: 0,
                end: 1,
            },
        ];
        let mut it = s.page_iterator(Direction::LeftUp, active_point(0, 0), None);
        let mut count = 0;
        for index in [1, 0] {
            let chunk = it.next().unwrap();
            assert_eq!(chunk.node, expected[index].node);
            assert_eq!(chunk.start, expected[index].start);
            assert_eq!(chunk.end, expected[index].end);
            count += chunk.end - chunk.start;
        }
        assert!(it.next().is_none());
        assert_eq!(count, s.rows);
    }
    #[test]
    fn page_list_cell_iterator_reverse() {
        let mut s = PageList::init(2, 2, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        let mut it = s.cell_iterator(Direction::LeftUp, screen_point(0, 0), None);
        for (x, y) in [(1, 1), (0, 1), (1, 0), (0, 0)] {
            assert_eq!(
                s.point_from_pin(Tag::Screen, it.next().unwrap()),
                Some(screen_point(x, y))
            );
        }
        assert!(it.next().is_none());
    }
    #[test]
    fn page_list_resize_no_reflow_more_rows() {
        let mut s = PageList::init(10, 3, Some(0)).unwrap();
        assert_eq!(s.total_rows, 3);
        grow_rows(&mut s, 0);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 0))
        );
        let p = s.track_pin(s.pin(active_point(0, 2)).unwrap()).unwrap();
        s.resize(Resize {
            rows: Some(10),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 10);
        assert_eq!(s.total_rows, 10);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(0, 2))
        );
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 0))
        );
    }
    #[test]
    fn page_list_resize_no_reflow_more_rows_with_history() {
        let mut s = PageList::init(10, 3, None).unwrap();
        assert_eq!(s.total_rows, 3);
        grow_rows(&mut s, 50);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 50))
        );
        let p = s.track_pin(s.pin(active_point(0, 2)).unwrap()).unwrap();
        s.resize(Resize {
            rows: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 5);
        assert_eq!(s.total_rows, 53);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(0, 4))
        );
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 48))
        );
    }
    #[test]
    fn page_list_resize_no_reflow_less_rows() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        assert_eq!(s.total_rows, 10);
        assert_eq!(s.first, s.last);
        for y in 0..10 {
            set_raw(&mut s, 0, y, 'A' as u32, Wide::Narrow);
        }
        s.resize(Resize {
            rows: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 5);
        assert_eq!(s.total_rows, 10);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 5))
        );
    }
    #[test]
    fn page_list_resize_no_reflow_one_rows() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        assert_eq!(s.total_rows, 10);
        assert_eq!(s.first, s.last);
        for y in 0..10 {
            set_raw(&mut s, 0, y, 'A' as u32, Wide::Narrow);
        }
        s.resize(Resize {
            rows: Some(1),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 1);
        assert_eq!(s.total_rows, 10);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 9))
        );
    }
    #[test]
    fn page_list_resize_no_reflow_less_cols() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        s.resize(Resize {
            cols: Some(5),
            rows: Some(10),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        assert_eq!(s.rows, 10);
        assert_eq!(s.total_rows, 10);
        for p in s.row_iterator(Direction::RightDown, screen_point(0, 0), None) {
            assert_eq!(p.cells(&s, CellSubset::All).unwrap().len(), 5);
        }
    }
    #[test]
    fn page_list_resize_no_reflow_less_cols_pin_in_trimmed_cols() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let p = s.track_pin(s.pin(active_point(8, 2)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        assert_eq!(s.total_rows, 10);
        for p in s.row_iterator(Direction::RightDown, screen_point(0, 0), None) {
            assert_eq!(p.cells(&s, CellSubset::All).unwrap().len(), 5);
        }
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(4, 2))
        );
    }
    #[test]
    fn page_list_resize_no_reflow_more_cols() {
        let mut s = PageList::init(5, 3, Some(0)).unwrap();
        s.resize(Resize {
            cols: Some(10),
            rows: Some(3),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 10);
        assert_eq!(s.rows, 3);
        assert_eq!(s.total_rows, 3);
        for p in s.row_iterator(Direction::RightDown, screen_point(0, 0), None) {
            assert_eq!(p.cells(&s, CellSubset::All).unwrap().len(), 10);
        }
    }
    #[test]
    fn page_list_resize_no_reflow_less_cols_then_more_cols() {
        let mut s = PageList::init(5, 3, Some(0)).unwrap();
        s.resize(Resize {
            cols: Some(2),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        s.resize(Resize {
            cols: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        assert_eq!(s.total_rows, 3);
        for p in s.row_iterator(Direction::RightDown, screen_point(0, 0), None) {
            assert_eq!(p.cells(&s, CellSubset::All).unwrap().len(), 5);
        }
    }
    #[test]
    fn page_list_resize_reflow_more_cols_no_wrapped_rows() {
        let mut s = PageList::init(5, 3, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..3 {
            for x in 0..5 {
                set_raw(&mut s, x, y, 'A' as u32, Wide::Narrow);
            }
        }
        s.resize(Resize {
            cols: Some(10),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 10);
        assert_eq!(s.total_rows, 3);
        for p in s.row_iterator(Direction::RightDown, screen_point(0, 0), None) {
            let cells = p.cells(&s, CellSubset::All).unwrap();
            assert_eq!(cells.len(), 10);
            assert_eq!(cells[0].codepoint(), 'A' as u32);
        }
    }
    #[test]
    fn page_list_resize_reflow_more_cols_wrapped_rows() {
        let mut s = PageList::init(2, 4, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..4 {
            set_wrap(&mut s, y, y % 2 == 0, y % 2 != 0);
            for x in 0..2 {
                set_raw(&mut s, x, y, 'A' as u32, Wide::Narrow);
            }
        }
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, 4);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 0))
        );
        let p = s
            .row_iterator(Direction::RightDown, screen_point(0, 0), None)
            .next()
            .unwrap();
        assert!(!p.row_and_cell(&s).unwrap().0.wrap());
        let cells = p.cells(&s, CellSubset::All).unwrap();
        assert_eq!(cells.len(), 4);
        assert_eq!(cells[0].codepoint(), 'A' as u32);
        assert_eq!(cells[2].codepoint(), 'A' as u32);
    }
    #[test]
    fn page_list_resize_reflow_less_cols_no_wrapped_rows() {
        let mut s = PageList::init(10, 3, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..3 {
            for x in 0..4 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        s.resize(Resize {
            cols: Some(5),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        assert_eq!(s.total_rows, 3);
        for p in s.row_iterator(Direction::RightDown, screen_point(0, 0), None) {
            let cells = p.cells(&s, CellSubset::All).unwrap();
            for x in 0..4 {
                assert_eq!(cells.len(), 5);
                assert_eq!(cells[x].codepoint(), x as u32);
            }
        }
    }
    #[test]
    fn page_list_resize_reflow_less_cols_wrapped_rows() {
        let mut s = PageList::init(4, 2, None).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..4 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 4);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 2))
        );
        assert_eq!(s.first, s.last);
        let mut it = s.row_iterator(Direction::RightDown, screen_point(0, 0), None);
        for y in 0..4 {
            let p = it.next().unwrap();
            assert_eq!(p.row_and_cell(&s).unwrap().0.wrap(), y % 2 == 0);
            let cells = p.cells(&s, CellSubset::All).unwrap();
            assert_eq!(cells.len(), 2);
            assert_eq!(cells[0].codepoint(), if y % 2 == 0 { 0 } else { 2 });
        }
    }
    #[test]
    fn page_list_reset() {
        let mut s = PageList::init(80, 24, None).unwrap();
        s.reset().unwrap();
        assert_eq!(s.viewport, Viewport::Active);
        assert_eq!(s.total_rows, 24);
        assert_eq!(
            s.get_top_left(Tag::Active),
            Some(Pin {
                node: s.first.unwrap(),
                y: 0,
                x: 0,
                garbage: false
            })
        );
        finish(&s);
    }
    #[test]
    fn page_list_reset_across_two_pages() {
        let rows = 100;
        let mut cols = 50;
        while initial_capacity(cols).rows >= rows {
            cols += 50;
        }
        let mut s = PageList::init(cols, rows, None).unwrap();
        s.reset().unwrap();
        assert_eq!(s.viewport, Viewport::Active);
        assert!(s.first.is_some());
        assert_eq!(s.total_rows, usize::from(s.rows));
        finish(&s);
    }

    #[test]
    fn page_list_clears_history() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 30);
        s.reset().unwrap();
        assert_eq!(s.viewport, Viewport::Active);
        assert_eq!(s.total_rows, 24);
        assert_eq!(
            s.get_top_left(Tag::Active),
            Some(Pin {
                node: s.first.unwrap(),
                y: 0,
                x: 0,
                garbage: false
            })
        );
        finish(&s);
    }
    #[test]
    fn page_list_reset_moves_tracked_pins_and_marks_them_as_garbage() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let h = s.track_pin(s.pin(active(42, 12)).unwrap()).unwrap();
        s.reset().unwrap();
        assert!(s.tracked_pin(h).unwrap().garbage);
        assert!(!s.tracked_pin(s.viewport_pin.unwrap()).unwrap().garbage);
        finish(&s);
    }
    #[test]
    fn page_list_split_at_middle_row() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        write_rows(&mut s);
        let p = s.pin(screen(0, 5)).unwrap();
        s.split(p).unwrap();
        assert_eq!(s.total_pages(), 2);
        assert_eq!(s.node(s.first.unwrap()).unwrap().data.size.rows, 5);
        for y in 0..10 {
            assert_eq!(s.get_cell(screen(0, y)).unwrap().codepoint(), y);
        }
        assert_eq!(s.node(s.last.unwrap()).unwrap().data.size.rows, 5);
        finish(&s);
    }
    #[test]
    fn page_list_split_at_row_0_is_no_op() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        write_rows(&mut s);
        let p = s.pin(screen(0, 0)).unwrap();
        s.split(p).unwrap();
        assert_eq!(s.total_pages(), 1);
        assert_eq!(s.node(s.first.unwrap()).unwrap().data.size.rows, 10);
        for y in 0..10 {
            assert_eq!(s.get_cell(screen(0, y)).unwrap().codepoint(), y);
        }
        finish(&s);
    }
    #[test]
    fn page_list_split_at_last_row() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        write_rows(&mut s);
        let p = s.pin(screen(0, 9)).unwrap();
        s.split(p).unwrap();
        assert_eq!(s.total_pages(), 2);
        assert_eq!(s.node(s.first.unwrap()).unwrap().data.size.rows, 9);
        for y in 0..10 {
            assert_eq!(s.get_cell(screen(0, y)).unwrap().codepoint(), y);
        }
        assert_eq!(s.node(s.last.unwrap()).unwrap().data.size.rows, 1);
        finish(&s);
    }
    #[test]
    fn page_list_split_single_row_page_returns_out_of_space() {
        let mut s = PageList::init(10, 1, Some(0)).unwrap();
        assert_eq!(
            s.split(s.get_top_left(Tag::Active).unwrap()),
            Err(Error::OutOfSpace)
        );
    }
    #[test]
    fn page_list_split_moves_tracked_pins() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let h = s.track_pin(s.pin(screen(3, 7)).unwrap()).unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        let p = s.tracked_pin(h).unwrap();
        assert_eq!(p.node, s.last.unwrap());
        assert_eq!(p.y, 2);
        assert_eq!(p.x, 3);
        finish(&s);
    }
    #[test]
    fn page_list_split_tracked_pin_before_split_point_unchanged() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let h = s.track_pin(s.pin(screen(5, 2)).unwrap()).unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        let p = s.tracked_pin(h).unwrap();
        assert_eq!(p.node, s.first.unwrap());
        assert_eq!(p.y, 2);
        assert_eq!(p.x, 5);
        finish(&s);
    }
    #[test]
    fn page_list_split_tracked_pin_at_split_point_moves_to_new_page() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let h = s.track_pin(s.pin(screen(4, 5)).unwrap()).unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        let p = s.tracked_pin(h).unwrap();
        assert_eq!(p.node, s.last.unwrap());
        assert_eq!(p.y, 0);
        assert_eq!(p.x, 4);
        finish(&s);
    }
    #[test]
    fn page_list_split_multiple_tracked_pins_across_regions() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let coords = [(0, 1), (2, 5), (3, 7), (8, 9)];
        let pins = coords.map(|(x, y)| s.track_pin(s.pin(screen(x, y)).unwrap()).unwrap());
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        for ((x, y), handle) in coords.into_iter().zip(pins) {
            let p = s.tracked_pin(handle).unwrap();
            assert_eq!(
                p.node,
                if y < 5 {
                    s.first.unwrap()
                } else {
                    s.last.unwrap()
                }
            );
            assert_eq!(p.y, if y < 5 { y as u16 } else { y as u16 - 5 });
            assert_eq!(p.x, x);
        }
        finish(&s);
    }

    #[test]
    fn page_list_split_tracked_viewport_pin_in_split_region_moves_correctly() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        s.set_tracked_pin(s.viewport_pin.unwrap(), s.pin(screen(6, 7)).unwrap())
            .unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        let p = s.tracked_pin(s.viewport_pin.unwrap()).unwrap();
        assert_eq!(p.node, s.last.unwrap());
        assert_eq!(p.y, 2);
        assert_eq!(p.x, 6);
        finish(&s);
    }

    #[test]
    fn page_list_split_preserves_styled_cells() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        let style = crate::style::Style {
            flags: crate::style::Flags {
                bold: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let id = page.styles.add(style).unwrap();
        for y in 5..8 {
            let i = page.cell_index(0, y).unwrap();
            page.cells[i] = Cell::init('S' as u32);
            page.cells[i].set_style_id(id);
            page.rows[y].set_styled(true);
            page.styles.r#use(id).unwrap();
        }
        page.styles.release(id).unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        assert_eq!(s.node(s.first.unwrap()).unwrap().data.styles.count(), 0);
        let page = &s.node(s.last.unwrap()).unwrap().data;
        assert_eq!(page.styles.count(), 1);
        for y in 0..3 {
            let (row, cell) = page.get_row_and_cell(0, y).unwrap();
            assert_eq!(cell.codepoint(), 'S' as u32);
            assert_ne!(cell.style_id(), 0);
            assert!(page.styles.get(cell.style_id()).unwrap().flags.bold);
            assert!(row.styled());
        }
        finish(&s);
    }

    #[test]
    fn page_list_split_preserves_grapheme_clusters() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        set_raw(&mut s, 0, 6, 0x1f468, Wide::Narrow);
        s.node_mut(s.first.unwrap())
            .unwrap()
            .data
            .set_graphemes(0, 6, &[0x200d, 0x1f469])
            .unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        assert_eq!(s.node(s.first.unwrap()).unwrap().data.grapheme_count(), 0);
        let page = &s.node(s.last.unwrap()).unwrap().data;
        assert_eq!(page.grapheme_count(), 1);
        let (row, cell) = page.get_row_and_cell(0, 1).unwrap();
        assert_eq!(cell.codepoint(), 0x1f468);
        assert!(row.grapheme());
        assert_eq!(
            page.lookup_grapheme(page.cell_index(0, 1).unwrap())
                .unwrap(),
            [0x200d, 0x1f469]
        );
        finish(&s);
    }

    #[test]
    fn page_list_split_preserves_hyperlinks() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        let id = page
            .insert_hyperlink(crate::hyperlink::Hyperlink {
                id: crate::hyperlink::HyperlinkId::Implicit(0),
                uri: b"https://example.com".to_vec(),
            })
            .unwrap();
        let i = page.cell_index(0, 7).unwrap();
        page.cells[i] = Cell::init('L' as u32);
        page.set_hyperlink(0, 7, id).unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        assert_eq!(
            s.node(s.first.unwrap()).unwrap().data.hyperlink_set.count(),
            0
        );
        let page = &s.node(s.last.unwrap()).unwrap().data;
        assert_eq!(page.hyperlink_set.count(), 1);
        let (_, cell) = page.get_row_and_cell(0, 2).unwrap();
        assert_eq!(cell.codepoint(), 'L' as u32);
        assert!(cell.hyperlink());
        let id = page
            .lookup_hyperlink(page.cell_index(0, 2).unwrap())
            .unwrap();
        assert_eq!(
            page.hyperlink_set.get(id).unwrap().uri,
            b"https://example.com"
        );
        finish(&s);
    }

    #[test]
    fn page_list_split_preserves_wrap_flags() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        page.rows[5].set_wrap(true);
        page.rows[6].set_wrap_continuation(true);
        page.rows[7].set_wrap(true);
        page.rows[7].set_wrap_continuation(true);
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        let page = &s.node(s.last.unwrap()).unwrap().data;
        for (y, (wrap, continuation)) in [(true, false), (false, true), (true, true)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(page.rows[y].wrap(), wrap);
            assert_eq!(page.rows[y].wrap_continuation(), continuation);
        }
        finish(&s);
    }

    #[test]
    fn page_list_split_middle_page_preserves_linked_list_order() {
        let mut s = PageList::init(10, 12, Some(0)).unwrap();
        s.split(s.pin(screen(0, 4)).unwrap()).unwrap();
        let first = s.first.unwrap();
        let middle = s.last.unwrap();
        assert_eq!(s.node(first).unwrap().data.size.rows, 4);
        assert_eq!(s.node(middle).unwrap().data.size.rows, 8);
        s.split(Pin {
            node: middle,
            x: 0,
            y: 4,
            garbage: false,
        })
        .unwrap();
        let last = s.node(middle).unwrap().next.unwrap();
        assert_eq!(s.first, Some(first));
        assert_eq!(s.node(first).unwrap().next, Some(middle));
        assert_eq!(s.last, Some(last));
        assert_eq!(s.node(first).unwrap().prev, None);
        assert_eq!(s.node(middle).unwrap().prev, Some(first));
        assert_eq!(s.node(last).unwrap().prev, Some(middle));
        assert_eq!(s.node(middle).unwrap().next, Some(last));
        assert_eq!(s.node(last).unwrap().next, None);
        for id in [first, middle, last] {
            assert_eq!(s.node(id).unwrap().data.size.rows, 4);
        }
        finish(&s);
    }

    #[test]
    fn page_list_split_last_page_makes_new_page_the_last() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        let old = s.last.unwrap();
        assert_eq!(s.node(old).unwrap().data.size.rows, 5);
        s.split(Pin {
            node: old,
            x: 0,
            y: 2,
            garbage: false,
        })
        .unwrap();
        let new = s.last.unwrap();
        assert_ne!(new, old);
        assert_eq!(s.node(new).unwrap().prev, Some(old));
        assert_eq!(s.node(new).unwrap().next, None);
        assert_eq!(s.node(old).unwrap().data.size.rows, 2);
        assert_eq!(s.node(new).unwrap().data.size.rows, 3);
        finish(&s);
    }

    #[test]
    fn page_list_split_first_page_keeps_original_as_first() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let first = s.first.unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        let second = s.last.unwrap();
        s.split(s.pin(screen(0, 2)).unwrap()).unwrap();
        assert_eq!(s.first, Some(first));
        assert_eq!(s.node(first).unwrap().prev, None);
        let inserted = s.node(first).unwrap().next.unwrap();
        assert_ne!(inserted, second);
        assert_eq!(s.node(inserted).unwrap().next, Some(second));
        assert_eq!(s.node(first).unwrap().data.size.rows, 2);
        assert_eq!(s.node(inserted).unwrap().data.size.rows, 3);
        assert_eq!(s.node(second).unwrap().data.size.rows, 5);
        finish(&s);
    }

    #[test]
    fn page_list_compact_std_size_page_returns_null() {
        let mut s = PageList::init(80, 24, Some(0)).unwrap();
        let node = s.first.unwrap();
        assert!(Page::layout(s.node(node).unwrap().data.capacity).total_size <= std_size());
        assert_eq!(s.compact(node).unwrap(), None);
        assert_eq!(s.first, Some(node));
        finish(&s);
    }

    #[test]
    fn page_list_compact_oversized_page() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let additional = first_cap(&s) - s.total_rows;
        grow(&mut s, additional + 1);
        assert_ne!(s.first, s.last);
        let mut node = s.first.unwrap();
        let page = &mut s.node_mut(node).unwrap().data;
        for y in 0..usize::from(page.size.rows) {
            for x in 0..80 {
                let i = page.cell_index(x, y).unwrap();
                page.cells[i] = Cell::init((x + y * 80) as u32);
            }
        }
        let tracked = s
            .track_pin(Pin {
                node,
                x: 5,
                y: 10,
                garbage: false,
            })
            .unwrap();
        while Page::layout(s.node(node).unwrap().data.capacity).total_size <= std_size() {
            node = s
                .increase_capacity(node, Some(IncreaseCapacity::GraphemeBytes))
                .unwrap();
        }
        let oversized_len = Page::layout(s.node(node).unwrap().data.capacity).total_size;
        assert!(oversized_len > std_size());
        let original_size = s.node(node).unwrap().data.size;
        let second = s.node(node).unwrap().next.unwrap();
        s.node_mut(node).unwrap().data.dirty = true;
        let new = s.compact(node).unwrap().unwrap();
        let page = &s.node(new).unwrap().data;
        assert!(Page::layout(page.capacity).total_size < oversized_len);
        assert_eq!(page.size, original_size);
        assert!(page.dirty);
        assert_eq!(s.first, Some(new));
        assert_eq!(s.node(new).unwrap().prev, None);
        assert_eq!(s.node(new).unwrap().next, Some(second));
        assert_eq!(s.node(second).unwrap().prev, Some(new));
        let pin = s.tracked_pin(tracked).unwrap();
        assert_eq!((pin.node, pin.x, pin.y), (new, 5, 10));
        for y in 0..usize::from(page.size.rows) {
            for x in 0..80 {
                assert_eq!(
                    page.get_row_and_cell(x, y).unwrap().1.codepoint(),
                    (x + y * 80) as u32
                );
            }
        }
        finish(&s);
    }

    #[test]
    fn page_list_clone_partial_trimmed_right() {
        let mut s = PageList::init(80, 20, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        grow_rows(&mut s, 30);
        let (s2, _) = s
            .clone_range(screen_point(0, 0), Some(screen_point(0, 39)))
            .unwrap();
        assert_eq!(s2.total_rows, 40);
    }

    #[test]
    fn page_list_clone_partial_trimmed_left() {
        let mut s = PageList::init(80, 20, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        grow_rows(&mut s, 30);
        let (s2, _) = s.clone_range(screen_point(0, 10), None).unwrap();
        assert_eq!(s2.total_rows, 40);
    }

    #[test]
    fn page_list_clone_partial_trimmed_both() {
        let mut s = PageList::init(80, 20, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        grow_rows(&mut s, 30);
        let (s2, _) = s
            .clone_range(screen_point(0, 10), Some(screen_point(0, 35)))
            .unwrap();
        assert_eq!(s2.total_rows, 26);
    }

    #[test]
    fn page_list_clone() {
        let mut s = PageList::init(80, 24, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        grow_rows(&mut s, 0);
        let (s2, _) = s.clone_range(screen_point(0, 0), None).unwrap();
        assert_eq!(s2.total_rows, 24);
    }

    #[test]
    fn page_list_clone_less_than_active() {
        let mut s = PageList::init(80, 24, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        grow_rows(&mut s, 0);
        let (s2, _) = s.clone_range(active_point(0, 5), None).unwrap();
        assert_eq!(s2.total_rows, 24);
    }

    #[test]
    fn page_list_clone_remap_tracked_pin() {
        let mut s = PageList::init(80, 24, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        let h = s.track_pin(s.pin(active(0, 6)).unwrap()).unwrap();
        let (q, map) = s.clone_range(active(0, 5), None).unwrap();
        assert_eq!(
            q.point_from_pin(Tag::Active, q.tracked_pin(map[&h]).unwrap()),
            Some(active(0, 1))
        );
        finish(&q);
    }

    #[test]
    fn page_list_clone_remap_tracked_pin_not_in_cloned_area() {
        let mut s = PageList::init(80, 24, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        let h = s.track_pin(s.pin(active(0, 3)).unwrap()).unwrap();
        let (q, map) = s.clone_range(active(0, 5), None).unwrap();
        assert!(!map.contains_key(&h));
        finish(&q);
    }

    #[test]
    fn page_list_clone_full_dirty() {
        let mut s = PageList::init(80, 24, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        for y in [0, 12, 23] {
            s.pin(active_point(0, y)).unwrap().mark_dirty(&mut s);
        }
        let (s2, _) = s.clone_range(screen_point(0, 0), None).unwrap();
        assert_eq!(s2.total_rows, usize::from(s.rows));
        for (y, dirty) in [(0, true), (1, false), (12, true), (14, false), (23, true)] {
            assert_eq!(s2.is_dirty(active_point(0, y)), dirty);
        }
    }

    #[test]
    fn page_list_clone_partial_trimmed_left_reclaims_styles() {
        use crate::style::{Flags, Style};
        let mut s = PageList::init(80, 20, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        grow_rows(&mut s, 30);
        assert_eq!(s.first, s.last);
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        let style = Style {
            flags: Flags {
                bold: true,
                ..Flags::default()
            },
            ..Style::default()
        };
        let id = page.styles.add(style).unwrap();
        for y in (0..10).rev() {
            let i = page.cell_index(0, y).unwrap();
            page.cells[i] = Cell::init('A' as u32);
            page.cells[i].set_style_id(id);
            page.rows[y].set_styled(true);
            page.styles.r#use(id).unwrap();
        }
        page.styles.release(id).unwrap();
        assert_eq!(page.styles.count(), 1);
        let (s2, _) = s.clone_range(screen_point(0, 10), None).unwrap();
        assert_eq!(s2.total_rows, 40);
        assert_eq!(s2.first, s2.last);
        assert_eq!(s2.node(s2.first.unwrap()).unwrap().data.styles.count(), 0);
    }

    #[test]
    fn page_list_erase() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let bytes = s.page_size;
        let cap = first_cap(&s);
        grow(&mut s, cap * 5);
        assert_eq!(s.total_pages(), 6);
        let h = s.track_pin(s.pin(screen(0, 0)).unwrap()).unwrap();
        s.erase_rows(Point::History(Coordinate::default()), None)
            .unwrap();
        assert_eq!(s.total_rows, 24);
        assert_eq!(s.total_pages(), 1);
        assert_eq!(s.page_size, bytes);
        let pin = s.tracked_pin(h).unwrap();
        assert_eq!(pin.node, s.first.unwrap());
        assert_eq!((pin.x, pin.y), (0, 0));
        finish(&s);
    }

    #[test]
    fn page_list_erase_reaccounts_page_size() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let bytes = s.page_size;
        let cap = first_cap(&s);
        grow(&mut s, cap * 5);
        assert_eq!(s.total_pages(), 6);
        let h = s.track_pin(s.pin(screen(0, 0)).unwrap()).unwrap();
        s.erase_rows(Point::History(Coordinate::default()), None)
            .unwrap();
        assert_eq!(s.total_rows, 24);
        assert_eq!(s.total_pages(), 1);
        assert_eq!(s.page_size, bytes);
        let pin = s.tracked_pin(h).unwrap();
        assert_eq!(pin.node, s.first.unwrap());
        assert_eq!((pin.x, pin.y), (0, 0));
        finish(&s);
    }

    #[test]
    fn page_list_erase_row_with_tracked_pin_resets_to_top_left() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let bytes = s.page_size;
        let cap = first_cap(&s);
        grow(&mut s, cap * 5);
        assert_eq!(s.total_pages(), 6);
        let h = s.track_pin(s.pin(screen(0, 0)).unwrap()).unwrap();
        s.erase_rows(Point::History(Coordinate::default()), None)
            .unwrap();
        assert_eq!(s.total_rows, 24);
        assert_eq!(s.total_pages(), 1);
        assert_eq!(s.page_size, bytes);
        let pin = s.tracked_pin(h).unwrap();
        assert_eq!(pin.node, s.first.unwrap());
        assert_eq!((pin.x, pin.y), (0, 0));
        finish(&s);
    }

    #[test]
    fn page_list_erase_row_with_tracked_pin_shifts() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let handle = s.track_pin(s.pin(active_point(2, 4)).unwrap()).unwrap();
        s.erase_rows(active_point(0, 0), Some(active_point(0, 3)))
            .unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        let p = s.tracked_pin(handle).unwrap();
        assert_eq!(p.node, s.first.unwrap());
        assert_eq!(p.y, 0);
        assert_eq!(p.x, 2);
    }

    #[test]
    fn page_list_erase_row_with_tracked_pin_is_erased() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let handle = s.track_pin(s.pin(active_point(2, 2)).unwrap()).unwrap();
        s.erase_rows(active_point(0, 0), Some(active_point(0, 3)))
            .unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        let p = s.tracked_pin(handle).unwrap();
        assert_eq!(p.node, s.first.unwrap());
        assert_eq!(p.y, 0);
        assert_eq!(p.x, 0);
    }

    #[test]
    fn page_list_erase_active_regrows_automatically() {
        let mut s = PageList::init(80, 24, None).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        s.erase_rows(active(0, 0), Some(active(0, 10))).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        finish(&s);
    }

    #[test]
    fn page_list_erase_a_one_row_active() {
        let mut s = PageList::init(10, 1, None).unwrap();
        assert_eq!(s.total_pages(), 1);
        set_raw(&mut s, 0, 0, 'A' as u32, Wide::Narrow);
        s.erase_rows(active_point(0, 0), Some(active_point(0, 0)))
            .unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        assert_eq!(s.get_cell(active_point(0, 0)).unwrap().codepoint(), 0);
    }

    #[test]
    fn page_list_erase_row_bounded_less_than_full_row() {
        let mut s = PageList::init(80, 10, None).unwrap();
        let handles: Vec<_> = [5, 8, 9]
            .into_iter()
            .map(|y| s.track_pin(s.pin(active_point(0, y)).unwrap()).unwrap())
            .collect();
        s.erase_row_bounded(active_point(0, 5), 3).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        for y in 5..8 {
            assert!(s.is_dirty(active_point(0, y)));
        }
        for (h, y) in handles.into_iter().zip([4, 7, 9]) {
            let p = s.tracked_pin(h).unwrap();
            assert_eq!(p.node, s.first.unwrap());
            assert_eq!(p.y, y);
            assert_eq!(p.x, 0);
        }
    }

    #[test]
    fn page_list_erase_row_bounded_with_pin_at_top() {
        let mut s = PageList::init(80, 10, None).unwrap();
        let h = s.track_pin(s.pin(active_point(5, 0)).unwrap()).unwrap();
        s.erase_row_bounded(active_point(0, 0), 3).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        for y in 0..3 {
            assert!(s.is_dirty(active_point(0, y)));
        }
        let p = s.tracked_pin(h).unwrap();
        assert_eq!(p.node, s.first.unwrap());
        assert_eq!(p.y, 0);
        assert_eq!(p.x, 0);
    }

    #[test]
    fn page_list_erase_row_bounded_full_rows_single_page() {
        let mut s = PageList::init(80, 10, None).unwrap();
        let handles: Vec<_> = [7, 9]
            .into_iter()
            .map(|y| s.track_pin(s.pin(active_point(0, y)).unwrap()).unwrap())
            .collect();
        s.erase_row_bounded(active_point(0, 5), 10).unwrap();
        assert_eq!(s.total_rows, usize::from(s.rows));
        for y in 5..10 {
            assert!(s.is_dirty(active_point(0, y)));
        }
        for (h, y) in handles.into_iter().zip([6, 8]) {
            let p = s.tracked_pin(h).unwrap();
            assert_eq!(p.node, s.first.unwrap());
            assert_eq!(p.y, y);
            assert_eq!(p.x, 0);
        }
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_in_unchanged_row() {
        let mut s = PageList::init(4, 2, None).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        let p = s.track_pin(s.pin(active_point(1, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(1, 0))
        );
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_in_blank_cell() {
        let mut s = PageList::init(6, 2, None).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        let p = s.track_pin(s.pin(active_point(2, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(2, 0))
        );
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_in_final_blank_cell() {
        let mut s = PageList::init(6, 2, None).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        let p = s.track_pin(s.pin(active_point(3, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(3, 0))
        );
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_in_wrapped_blank_cell() {
        let mut s = PageList::init(6, 2, None).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        let p = s.track_pin(s.pin(active_point(5, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(3, 0))
        );
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_in_wrapped_row() {
        let mut s = PageList::init(4, 2, None).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..4 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        let p = s.track_pin(s.pin(active_point(2, 1)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 4);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(0, 1))
        );
    }

    #[test]
    fn page_list_resize_reflow_less_cols_blank_lines() {
        let mut s = PageList::init(4, 3, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for y in [0] {
            for x in 0..4 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 3);
        let mut it = s.row_iterator(Direction::RightDown, active_point(0, 0), None);
        for (wrap, cp) in [(true, 0), (false, 2)] {
            let p = it.next().unwrap();
            assert_eq!(p.row_and_cell(&s).unwrap().0.wrap(), wrap);
            let cells = p.cells(&s, CellSubset::All).unwrap();
            assert_eq!(cells.len(), 2);
            assert_eq!(cells[0].codepoint(), cp);
        }
    }

    #[test]
    fn page_list_resize_reflow_less_cols_blank_lines_between() {
        let mut s = PageList::init(4, 3, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for y in [0, 2] {
            for x in 0..4 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 5);
        let mut it = s.row_iterator(Direction::RightDown, active_point(0, 0), None);
        assert!(!it.next().unwrap().row_and_cell(&s).unwrap().0.wrap());
        for (wrap, cp) in [(true, 0), (false, 2)] {
            let p = it.next().unwrap();
            assert_eq!(p.row_and_cell(&s).unwrap().0.wrap(), wrap);
            let cells = p.cells(&s, CellSubset::All).unwrap();
            assert_eq!(cells.len(), 2);
            assert_eq!(cells[0].codepoint(), cp);
        }
    }

    #[test]
    fn page_list_resize_reflow_less_cols_blank_lines_between_no_scrollback() {
        let mut s = PageList::init(5, 3, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        set_raw(&mut s, 0, 0, 'A' as u32, Wide::Narrow);
        set_raw(&mut s, 0, 2, 'C' as u32, Wide::Narrow);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 3);
        let mut it = s.row_iterator(Direction::RightDown, active_point(0, 0), None);
        for cp in ['A' as u32, 0, 'C' as u32] {
            let p = it.next().unwrap();
            let cells = p.cells(&s, CellSubset::All).unwrap();
            assert_eq!(cells[0].codepoint(), cp);
            if cp != 0 {
                assert!(!p.row_and_cell(&s).unwrap().0.wrap());
                assert_eq!(cells.len(), 2);
            }
        }
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_not_on_last_line_preserves_location() {
        let mut s = PageList::init(5, 5, Some(1)).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..5 {
            for x in 0..2 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        grow_rows(&mut s, 5);
        assert_eq!(s.total_rows, 10);
        let p = s.track_pin(s.pin(active_point(0, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(4),
            cursor: Some(Cursor { x: 1, y: 1 }),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, 10);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(0, 0))
        );
    }

    #[test]
    fn page_list_resize_reflow_less_cols_no_reflow_preserves_semantic_prompt_on_first_line() {
        let mut s = PageList::init(4, 4, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        mark_prompt(&mut s, 0, SemanticPrompt::Prompt);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 4);
        assert_eq!(s.first, s.last);
        assert_eq!(
            s.pin(active_point(0, 0))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .semantic_prompt(),
            SemanticPrompt::Prompt
        );
    }

    #[test]
    fn page_list_resize_reflow_less_cols_wrap_preserves_semantic_prompt() {
        let mut s = PageList::init(4, 4, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        mark_prompt(&mut s, 0, SemanticPrompt::Prompt);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 4);
        assert_eq!(s.first, s.last);
        assert_eq!(
            s.pin(active_point(0, 0))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .semantic_prompt(),
            SemanticPrompt::Prompt
        );
    }

    #[test]
    fn page_list_resize_reflow_less_cols_no_reflow_preserves_semantic_prompt() {
        let mut s = PageList::init(4, 4, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        mark_prompt(&mut s, 1, SemanticPrompt::Prompt);
        for x in 0..4 {
            set_raw(&mut s, x, 1, u32::from(x), Wide::Narrow);
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 4);
        assert_eq!(s.first, s.last);
        assert_eq!(
            s.pin(active_point(0, 1))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .semantic_prompt(),
            SemanticPrompt::Prompt
        );
        assert!(
            s.pin(active_point(0, 1))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .wrap()
        );
        assert_eq!(
            s.pin(active_point(0, 2))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .semantic_prompt(),
            SemanticPrompt::Prompt
        );
    }

    #[test]
    fn page_list_resize_reflow_less_cols_to_wrap_a_wide_char() {
        let mut s = PageList::init(3, 1, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        set_raw(&mut s, 0, 0, 'x' as u32, Wide::Narrow);
        set_raw(&mut s, 1, 0, '😀' as u32, Wide::Wide);
        set_raw(&mut s, 2, 0, 0, Wide::SpacerTail);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 2);
        assert_eq!(s.first, s.last);
        assert_cell(&s, 0, 0, 'x' as u32, Wide::Narrow);
        assert!(
            s.pin(screen_point(0, 0))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .wrap()
        );
        assert_cell(&s, 1, 0, 0, Wide::SpacerHead);
        assert_cell(&s, 0, 1, '😀' as u32, Wide::Wide);
        assert_cell(&s, 1, 1, 0, Wide::SpacerTail);
    }

    #[test]
    fn page_list_resize_reflow_more_cols_unwrap_still_requires_wide_spacer_head() {
        let mut s = PageList::init(2, 2, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for x in 0..2 {
            set_raw(&mut s, x, 0, 'x' as u32, Wide::Narrow);
        }
        set_wrap(&mut s, 0, true, false);
        set_raw(&mut s, 0, 1, '😀' as u32, Wide::Wide);
        set_raw(&mut s, 1, 1, 0, Wide::SpacerTail);
        set_wrap(&mut s, 1, false, true);
        s.resize(Resize {
            cols: Some(3),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 3);
        assert_eq!(s.total_rows, 2);
        assert_eq!(s.first, s.last);
        assert!(
            s.pin(screen_point(0, 0))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .wrap()
        );
        for x in 0..2 {
            assert_cell(&s, x, 0, 'x' as u32, Wide::Narrow);
        }
        assert_cell(&s, 2, 0, 0, Wide::SpacerHead);
        assert_cell(&s, 0, 1, '😀' as u32, Wide::Wide);
        assert_cell(&s, 1, 1, 0, Wide::SpacerTail);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_to_wrap_a_multi_codepoint_grapheme_with_a_spacer_head() {
        let mut s = PageList::init(4, 2, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        let cps = [0x200D, 0x1F468, 0x200D, 0x1F466, 0x200D, 0x1F466];
        for x in [0, 2] {
            set_raw(&mut s, x, 0, 0x1F468, Wide::Wide);
            s.node_mut(s.first.unwrap())
                .unwrap()
                .data
                .set_graphemes(usize::from(x), 0, &cps)
                .unwrap();
            set_raw(&mut s, x + 1, 0, 0, Wide::SpacerTail);
        }
        s.resize(Resize {
            cols: Some(3),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 3);
        assert_eq!(s.total_rows, 2);
        assert_eq!(s.first, s.last);
        // Zig checks the first cluster twice; also check the moved second one.
        for y in [0, 0, 1] {
            assert_cell(&s, 0, y, 0x1F468, Wide::Wide);
            let p = s.pin(screen_point(0, y)).unwrap();
            assert_eq!(p.grapheme(&s).unwrap().len(), 6);
            assert_eq!(p.grapheme(&s).unwrap(), cps);
        }
        assert!(
            s.pin(screen_point(0, 0))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .wrap()
        );
        assert_cell(&s, 1, 0, 0, Wide::SpacerTail);
        assert_cell(&s, 2, 0, 0, Wide::SpacerHead);
        assert_cell(&s, 1, 1, 0, Wide::SpacerTail);
    }

    #[test]
    fn page_list_prompt_iterator_left_up() {
        let mut s = PageList::init(2, 20, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for y in [3, 6] {
            mark_prompt(&mut s, y, SemanticPrompt::Prompt);
        }
        for y in [7, 8, 12] {
            mark_prompt(&mut s, y, SemanticPrompt::PromptContinuation);
        }
        let mut it = s.prompt_iterator(Direction::LeftUp, screen_point(0, 0), None);
        for y in [12, 6, 3] {
            assert_eq!(
                s.point_from_pin(Tag::Screen, it.next().unwrap()),
                Some(screen_point(0, y))
            );
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn page_list_prompt_iterator_right_down() {
        let mut s = PageList::init(2, 20, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for y in [3, 6] {
            mark_prompt(&mut s, y, SemanticPrompt::Prompt);
        }
        for y in [7, 8, 12] {
            mark_prompt(&mut s, y, SemanticPrompt::PromptContinuation);
        }
        let mut it = s.prompt_iterator(Direction::RightDown, screen_point(0, 0), None);
        for y in [3, 6, 12] {
            assert_eq!(
                s.point_from_pin(Tag::Screen, it.next().unwrap()),
                Some(screen_point(0, y))
            );
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn page_list_highlight_semantic_content_prompt() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..5 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('A' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 5..8 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('B' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(2, 5)).unwrap(), SemanticContent::Prompt);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(0, 5)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(7, 5)));
    }

    #[test]
    fn page_list_highlight_semantic_content_prompt_with_output() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..3 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 3..7 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('l' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }

            for x in 7..10 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Prompt);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(0, 5)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(6, 5)));
    }

    #[test]
    fn page_list_highlight_semantic_content_prompt_multiline() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..10 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }
        }

        {
            for x in 0..5 {
                let cell = &mut page.get_cells_mut(6).unwrap()[x];
                *cell = Cell::init('c' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(2, 5)).unwrap(), SemanticContent::Prompt);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(0, 5)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(4, 6)));
    }

    #[test]
    fn page_list_highlight_semantic_content_prompt_only() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..5 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Prompt);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(0, 5)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(4, 5)));
    }

    #[test]
    fn page_list_highlight_semantic_content_prompt_to_end_of_screen() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 15;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..3 {
                let cell = &mut page.get_cells_mut(15).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 3..8 {
                let cell = &mut page.get_cells_mut(15).unwrap()[x];
                *cell = Cell::init('c' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(0, 15)).unwrap(), SemanticContent::Prompt);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(0, 15)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(7, 15)));
    }

    #[test]
    fn page_list_highlight_semantic_content_input_basic() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..3 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 3..8 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('l' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl = s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Input);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(3, 5)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(7, 5)));
    }

    #[test]
    fn page_list_highlight_semantic_content_input_with_output() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..2 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 2..5 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('c' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }

            for x in 5..10 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl = s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Input);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(2, 5)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(4, 5)));
    }

    #[test]
    fn page_list_highlight_semantic_content_input_multiline_with_continuation() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..2 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 2..10 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('c' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }
        }

        {
            for x in 0..2 {
                let cell = &mut page.get_cells_mut(6).unwrap()[x];
                *cell = Cell::init('>' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 2..6 {
                let cell = &mut page.get_cells_mut(6).unwrap()[x];
                *cell = Cell::init('d' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl = s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Input);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(2, 5)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(5, 6)));
    }

    #[test]
    fn page_list_highlight_semantic_content_input_no_input_returns_null() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..3 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 3..10 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl = s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Input);
        assert!(hl.is_none());
    }

    #[test]
    fn page_list_highlight_semantic_content_input_to_end_of_screen() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 15;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..2 {
                let cell = &mut page.get_cells_mut(15).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 2..7 {
                let cell = &mut page.get_cells_mut(15).unwrap()[x];
                *cell = Cell::init('c' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(0, 15)).unwrap(), SemanticContent::Input);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(2, 15)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(6, 15)));
    }

    #[test]
    fn page_list_highlight_semantic_content_input_prompt_only_returns_null() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..10 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }
        }

        {
            for y in 6..10 {
                for x in 0..10 {
                    let cell = &mut page.get_cells_mut(y).unwrap()[x];
                    cell.set_semantic_content(SemanticContent::Prompt);
                }
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl = s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Input);
        assert!(hl.is_none());
    }

    #[test]
    fn page_list_highlight_semantic_content_output_basic() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..2 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 2..5 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('l' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }

            for x in 5..8 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }

            for x in 8..10 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                cell.set_semantic_content(SemanticContent::Prompt);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Output);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(5, 5)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(7, 5)));
    }

    #[test]
    fn page_list_highlight_semantic_content_output_multiline() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..2 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 2..4 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('l' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }

            for x in 4..10 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }
        }

        {
            for x in 0..10 {
                let cell = &mut page.get_cells_mut(6).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }
        }

        {
            for x in 0..5 {
                let cell = &mut page.get_cells_mut(7).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }
            for x in 5..10 {
                let cell = &mut page.get_cells_mut(7).unwrap()[x];
                cell.set_semantic_content(SemanticContent::Input);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Output);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(4, 5)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(4, 7)));
    }

    #[test]
    fn page_list_highlight_semantic_content_output_stops_at_next_prompt() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..2 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 2..4 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('l' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }

            for x in 4..10 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }
        }

        {
            for x in 0..3 {
                let cell = &mut page.get_cells_mut(6).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }

            for x in 3..6 {
                let cell = &mut page.get_cells_mut(6).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Output);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(4, 5)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(2, 6)));
    }

    #[test]
    fn page_list_highlight_semantic_content_output_to_end_of_screen() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 15;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..2 {
                let cell = &mut page.get_cells_mut(15).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 2..4 {
                let cell = &mut page.get_cells_mut(15).unwrap()[x];
                *cell = Cell::init('c' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }

            for x in 4..10 {
                let cell = &mut page.get_cells_mut(15).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }
        }

        {
            for x in 0..8 {
                let cell = &mut page.get_cells_mut(16).unwrap()[x];
                *cell = Cell::init('o' as u32);
                cell.set_semantic_content(SemanticContent::Output);
            }
            for x in 8..10 {
                let cell = &mut page.get_cells_mut(16).unwrap()[x];
                cell.set_semantic_content(SemanticContent::Prompt);
            }
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(0, 15)).unwrap(), SemanticContent::Output);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(4, 15)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(7, 16)));
    }

    #[test]
    fn page_list_highlight_semantic_content_output_no_output_returns_null() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..3 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }

            for x in 3..10 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('c' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }
        }

        {
            for y in 6..10 {
                for x in 0..10 {
                    let cell = &mut page.get_cells_mut(y).unwrap()[x];
                    cell.set_semantic_content(SemanticContent::Input);
                }
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Output);
        assert!(hl.is_none());
    }

    #[test]
    fn page_list_highlight_semantic_content_output_skips_empty_cells() {
        let mut s = PageList::init(10, 20, Some(0)).unwrap();

        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;

        {
            let rac_y = 5;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);

            for x in 0..3 {
                let cell = &mut page.get_cells_mut(5).unwrap()[x];
                *cell = Cell::init('$' as u32);
                cell.set_semantic_content(SemanticContent::Prompt);
            }
        }

        {
            for x in 0..4 {
                let cell = &mut page.get_cells_mut(6).unwrap()[x];
                *cell = Cell::init('l' as u32);
                cell.set_semantic_content(SemanticContent::Input);
            }
        }

        {
            for y in 7..9 {
                for x in 0..5 {
                    let cell = &mut page.get_cells_mut(y).unwrap()[x];
                    *cell = Cell::init('o' as u32);
                    cell.set_semantic_content(SemanticContent::Output);
                }
            }
        }

        {
            let rac_y = 10;
            page.rows[rac_y].set_semantic_prompt(SemanticPrompt::Prompt);
        }

        let hl =
            s.highlight_semantic_content(s.pin(screen(0, 5)).unwrap(), SemanticContent::Output);
        let hl = hl.unwrap();
        assert_eq!(s.point_from_pin(Tag::Screen, hl.0), Some(screen(0, 7)));
        assert_eq!(s.point_from_pin(Tag::Screen, hl.1), Some(screen(4, 8)));
    }
    #[test]
    fn page_list_resize_no_reflow_less_rows_trims_blank_lines() {
        let mut s = PageList::init(10, 5, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        set_raw(&mut s, 0, 0, 'A' as u32, Wide::Narrow);
        for y in 1..usize::from(s.rows) {
            let c = s.get_cell_mut(active_point(0, y)).unwrap();
            c.set_content_tag(ContentTag::BgColorRgb);
            c.set_color_rgb(ghostty_vt::color::Rgb { r: 255, g: 0, b: 0 });
        }
        let p = s.track_pin(s.pin(active_point(0, 0)).unwrap()).unwrap();
        let pt = s
            .point_from_pin(Tag::Active, s.tracked_pin(p).unwrap())
            .unwrap();
        assert_eq!(s.get_cell(pt).unwrap().codepoint(), 'A' as u32);
        s.resize(Resize {
            rows: Some(2),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 2);
        assert_eq!(s.total_rows, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(0, 0))
        );
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 0))
        );
    }

    #[test]
    fn page_list_resize_no_reflow_less_rows_trims_blank_lines_cursor_in_blank_line() {
        let mut s = PageList::init(10, 5, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        set_raw(&mut s, 0, 0, 'A' as u32, Wide::Narrow);
        for y in 1..usize::from(s.rows) {
            let c = s.get_cell_mut(active_point(0, y)).unwrap();
            c.set_content_tag(ContentTag::BgColorRgb);
            c.set_color_rgb(ghostty_vt::color::Rgb { r: 255, g: 0, b: 0 });
        }
        let p = s.track_pin(s.pin(active_point(0, 3)).unwrap()).unwrap();
        s.resize(Resize {
            rows: Some(2),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 2);
        assert_eq!(s.total_rows, 4);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(0, 1))
        );
    }

    #[test]
    fn page_list_resize_no_reflow_less_rows_trims_blank_lines_erases_pages() {
        let mut s = PageList::init(100, 5, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        let rows = s.node(s.first.unwrap()).unwrap().data.capacity.rows + 10;
        s.resize(Resize {
            rows: Some(rows),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_pages(), 2);
        set_raw(&mut s, 0, 0, 'A' as u32, Wide::Narrow);
        s.resize(Resize {
            rows: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 5);
        assert_eq!(s.total_rows, 5);
        assert_eq!(s.total_pages(), 1);
    }

    #[test]
    fn page_list_resize_no_reflow_more_rows_extends_blank_lines() {
        let mut s = PageList::init(10, 3, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        set_raw(&mut s, 0, 0, 'A' as u32, Wide::Narrow);
        for y in 1..usize::from(s.rows) {
            let c = s.get_cell_mut(active_point(0, y)).unwrap();
            c.set_content_tag(ContentTag::BgColorRgb);
            c.set_color_rgb(ghostty_vt::color::Rgb { r: 255, g: 0, b: 0 });
        }
        s.resize(Resize {
            rows: Some(7),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 7);
        assert_eq!(s.total_rows, 7);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 0))
        );
    }

    #[test]
    fn page_list_resize_no_reflow_less_cols_clears_graphemes() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let p = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        p.cells[9] = Cell::init('A' as u32);
        p.append_grapheme(9, 0, 'A' as u32).unwrap();
        assert_eq!(p.grapheme_count(), 1);
        s.resize(Resize {
            cols: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        assert_eq!(s.total_rows, 10);
        for n in s.nodes.iter().flatten() {
            assert_eq!(n.data.grapheme_count(), 0);
        }
        finish(&s);
    }

    #[test]
    fn page_list_resize_no_reflow_less_rows_and_cols() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        s.resize(Resize {
            cols: Some(5),
            rows: Some(7),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        assert_eq!(s.rows, 7);
        assert_eq!(s.total_rows, 7);
        for p in s.row_iterator(Direction::RightDown, screen_point(0, 0), None) {
            assert_eq!(p.cells(&s, CellSubset::All).unwrap().len(), 5);
        }
    }

    #[test]
    fn page_list_resize_no_reflow_more_rows_and_less_cols() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        s.resize(Resize {
            cols: Some(5),
            rows: Some(20),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        assert_eq!(s.rows, 20);
        assert_eq!(s.total_rows, 20);
        for p in s.row_iterator(Direction::RightDown, screen_point(0, 0), None) {
            assert_eq!(p.cells(&s, CellSubset::All).unwrap().len(), 5);
        }
    }

    #[test]
    fn page_list_resize_no_reflow_empty_screen() {
        let mut s = PageList::init(5, 5, Some(0)).unwrap();
        s.resize(Resize {
            cols: Some(10),
            rows: Some(10),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 10);
        assert_eq!(s.rows, 10);
        assert_eq!(s.total_rows, 10);
        for p in s.row_iterator(Direction::RightDown, screen_point(0, 0), None) {
            assert_eq!(p.cells(&s, CellSubset::All).unwrap().len(), 10);
        }
    }

    #[test]
    fn page_list_increase_capacity_after_col_shrink() {
        let mut s = PageList::init(10, 2, Some(0)).unwrap();
        s.resize(Resize {
            cols: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        assert_eq!(s.node(s.first.unwrap()).unwrap().data.size.cols, 5);
        assert!(s.node(s.first.unwrap()).unwrap().data.capacity.cols >= 10);
        s.increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::Styles))
            .unwrap();
        assert_eq!(s.node(s.first.unwrap()).unwrap().data.size.cols, 5);
        assert_eq!(s.cols, 5);
    }

    #[test]
    fn page_list_increase_capacity_multi_page() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let capacity = s.node(s.first.unwrap()).unwrap().data.capacity.rows;
        for _ in s.rows..capacity {
            assert!(s.grow().unwrap().is_none());
        }
        assert!(s.grow().unwrap().is_some());
        assert_ne!(s.first, s.last);
        let before_first = s.node(s.first.unwrap()).unwrap().data.capacity.styles;
        let before_last = s.node(s.last.unwrap()).unwrap().data.capacity.styles;
        s.increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::Styles))
            .unwrap();
        assert_eq!(
            s.node(s.first.unwrap()).unwrap().data.capacity.styles,
            before_first * 2
        );
        assert_eq!(
            s.node(s.last.unwrap()).unwrap().data.capacity.styles,
            before_last
        );
    }

    #[test]
    fn page_list_compact_insufficient_savings_returns_null() {
        let mut s = PageList::init(80, 24, Some(0)).unwrap();
        let id = s
            .increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::GraphemeBytes))
            .unwrap();
        let bytes = s.page_size;
        let result = s.compact(id).unwrap();
        if bytes <= std_size() {
            assert_eq!(result, None);
        } else if result.is_some() {
            assert!(s.page_size < bytes);
        }
        finish(&s);
    }

    #[test]
    fn page_list_page_iterator_history_two_pages() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let capacity = s.node(s.first.unwrap()).unwrap().data.capacity.rows;
        for _ in s.rows..capacity {
            assert!(s.grow().unwrap().is_none());
        }
        assert!(s.grow().unwrap().is_some());
        let mut it = s.page_iterator(
            Direction::RightDown,
            Point::History(Coordinate::default()),
            None,
        );
        let chunk = it.next().unwrap();
        assert_eq!(chunk.node, s.first.unwrap());
        assert_eq!(chunk.start, 0);
        assert_eq!(chunk.end, s.get_top_left(Tag::Active).unwrap().y);
        assert!(it.next().is_none());
    }

    #[test]
    fn page_list_page_iterator_reverse_history_two_pages() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let capacity = s.node(s.first.unwrap()).unwrap().data.capacity.rows;
        for _ in s.rows..capacity {
            assert!(s.grow().unwrap().is_none());
        }
        assert!(s.grow().unwrap().is_some());
        let mut it = s.page_iterator(
            Direction::LeftUp,
            Point::History(Coordinate::default()),
            None,
        );
        let chunk = it.next().unwrap();
        assert_eq!(chunk.node, s.first.unwrap());
        assert_eq!(chunk.start, 0);
        assert_eq!(chunk.end, s.get_top_left(Tag::Active).unwrap().y);
        assert!(it.next().is_none());
    }

    #[test]
    fn page_list_erase_row_bounded_full_rows_two_pages() {
        let mut s = PageList::init(80, 10, None).unwrap();
        let capacity = s.node(s.first.unwrap()).unwrap().data.capacity.rows;
        grow_rows(&mut s, usize::from(capacity - 10) + 5);
        assert_eq!(s.total_pages(), 2);
        assert_eq!(s.node(s.last.unwrap()).unwrap().data.size.rows, 5);
        let handles: Vec<_> = [4, 3, 8, 9]
            .into_iter()
            .map(|y| s.track_pin(s.pin(active_point(0, y)).unwrap()).unwrap())
            .collect();
        for (h, (node, y)) in handles.iter().zip([
            (s.first.unwrap(), capacity - 1),
            (s.first.unwrap(), capacity - 2),
            (s.last.unwrap(), 3),
            (s.last.unwrap(), 4),
        ]) {
            let p = s.tracked_pin(*h).unwrap();
            assert_eq!(p.node, node);
            assert_eq!(p.y, y);
            assert_eq!(p.x, 0);
        }
        s.erase_row_bounded(active_point(0, 4), 4).unwrap();
        for y in 4..8 {
            assert!(s.is_dirty(active_point(0, y)));
        }
        for (h, (node, y)) in handles.iter().zip([
            (s.first.unwrap(), capacity - 2),
            (s.first.unwrap(), capacity - 2),
            (s.last.unwrap(), 2),
            (s.last.unwrap(), 4),
        ]) {
            let p = s.tracked_pin(*h).unwrap();
            assert_eq!(p.node, node);
            assert_eq!(p.y, y);
            assert_eq!(p.x, 0);
        }
    }

    #[test]
    fn page_list_resize_reflow_less_cols_wrapped_rows_with_graphemes() {
        let mut s = PageList::init(4, 2, None).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..4 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
            s.node_mut(s.first.unwrap())
                .unwrap()
                .data
                .append_grapheme(2, y, 'A' as u32)
                .unwrap();
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 4);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 2))
        );
        assert_eq!(s.first, s.last);
        let mut it = s.row_iterator(Direction::RightDown, screen_point(0, 0), None);
        for y in 0..4 {
            let p = it.next().unwrap();
            assert_eq!(p.row_and_cell(&s).unwrap().0.wrap(), y % 2 == 0);
            let cells = p.cells(&s, CellSubset::All).unwrap();
            assert_eq!(cells.len(), 2);
            assert_eq!(cells[0].codepoint(), if y % 2 == 0 { 0 } else { 2 });
            if y % 2 != 0 {
                assert!(p.row_and_cell(&s).unwrap().0.grapheme());
                let cps = p.grapheme(&s).unwrap();
                assert_eq!(cps.len(), 1);
                assert_eq!(cps[0], 'A' as u32);
            }
        }
    }

    #[test]
    fn page_list_resize_reflow_less_cols_copy_style() {
        use crate::style::{Flags, Style};
        let mut s = PageList::init(4, 2, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        let id = page
            .styles
            .add(Style {
                flags: Flags {
                    bold: true,
                    ..Flags::default()
                },
                ..Style::default()
            })
            .unwrap();
        for x in 0..3 {
            page.get_cells_mut(0).unwrap()[x] = Cell::init(x as u32);
            page.get_cells_mut(0).unwrap()[x].set_style_id(id);
            page.styles.r#use(id).unwrap();
        }
        page.styles.release(id).unwrap();
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 2);
        for p in s.row_iterator(Direction::RightDown, active_point(0, 0), None) {
            for x in 0..s.cols - 1 {
                let p = Pin { x, ..p };
                let (row, cell) = p.row_and_cell(&s).unwrap();
                assert_ne!(cell.style_id(), 0);
                assert!(p.style(&s).unwrap().flags.bold);
                assert!(row.styled());
            }
        }
    }

    #[test]
    fn page_list_resize_reflow_less_cols_wraps_spacer_head() {
        let mut s = PageList::init(4, 3, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for x in 0..3 {
            set_raw(&mut s, x, 0, 'x' as u32, Wide::Narrow);
        }
        set_raw(&mut s, 3, 0, 0, Wide::SpacerHead);
        set_wrap(&mut s, 0, true, false);
        set_raw(&mut s, 0, 1, '😀' as u32, Wide::Wide);
        set_raw(&mut s, 1, 1, 0, Wide::SpacerTail);
        set_wrap(&mut s, 1, false, true);
        s.resize(Resize {
            cols: Some(3),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 3);
        assert_eq!(s.total_rows, 3);
        assert_eq!(s.first, s.last);
        assert!(
            s.pin(screen_point(0, 0))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .wrap()
        );
        for x in 0..3 {
            assert_cell(&s, x, 0, 'x' as u32, Wide::Narrow);
        }
        assert_cell(&s, 0, 1, '😀' as u32, Wide::Wide);
        assert_cell(&s, 1, 1, 0, Wide::SpacerTail);
    }

    #[test]
    fn page_list_resize_reflow_more_cols_no_reflow_preserves_semantic_prompt() {
        let mut s = PageList::init(2, 4, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        mark_prompt(&mut s, 1, SemanticPrompt::Prompt);
        s.resize(Resize {
            cols: Some(4),
            reflow: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, 4);
        assert_eq!(s.first, s.last);
        assert_eq!(
            s.node(s.first.unwrap()).unwrap().data.rows[1].semantic_prompt(),
            SemanticPrompt::Prompt
        );
    }

    #[test]
    fn page_list_scroll_clear() {
        let mut s = PageList::init(80, 24, None).unwrap();
        for y in [0, 1] {
            *s.get_cell_mut(active(0, y)).unwrap() = Cell::init('A' as u32);
        }
        s.scroll_clear().unwrap();
        assert_viewport_point(&s, 2);
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_to_eliminate_a_wide_char() {
        let mut s = PageList::init(2, 1, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        set_raw(&mut s, 0, 0, '😀' as u32, Wide::Wide);
        set_raw(&mut s, 1, 0, 0, Wide::SpacerTail);
        s.resize(Resize {
            cols: Some(1),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 1);
        assert_eq!(s.total_rows, 1);
        assert_eq!(s.first, s.last);
        assert_cell(&s, 0, 0, 0, Wide::Narrow);
    }
    #[test]
    fn stale_tracked_handle_cannot_release_reused_slot() {
        let mut s = PageList::init(4, 2, None).unwrap();
        let p = s.get_top_left(Tag::Active).unwrap();
        let old = s.track_pin(p).unwrap();
        s.untrack_pin(old);
        let new = s.track_pin(p).unwrap();
        s.untrack_pin(old);
        assert!(s.tracked_pin(old).is_none());
        assert_eq!(s.tracked_pin(new), Some(p));
        finish(&s);
    }
    #[test]
    fn row_erasure_retains_page_dimensions_and_cursor_row() {
        let mut s = PageList::init(80, 24, None).unwrap();
        fill_pages(&mut s, 2);
        write_rows(&mut s);
        let p = s.pin(screen(3, 5)).unwrap();
        let h = s.track_pin(p).unwrap();
        let total = s.total_rows;
        let cap = first_cap(&s);
        s.erase_row(screen(0, 5)).unwrap();
        assert_eq!(s.total_rows, total);
        assert_eq!(first_cap(&s), cap);
        assert_eq!(s.tracked_pin(h), Some(p));
        assert_eq!(s.get_cell(screen(0, 5)).unwrap().codepoint(), 6);
        finish(&s);
    }
    fn grow_rows(s: &mut PageList, count: usize) {
        for _ in 0..count {
            s.grow().unwrap();
        }
    }
    fn screen_point(x: u16, y: usize) -> Point {
        Point::Screen(Coordinate { x, y: y as u32 })
    }
    fn active_point(x: u16, y: usize) -> Point {
        Point::Active(Coordinate { x, y: y as u32 })
    }
    fn mark_prompt(s: &mut PageList, y: usize, prompt: SemanticPrompt) {
        let p = s.pin(screen_point(0, y)).unwrap();
        s.node_mut(p.node).unwrap().data.rows[usize::from(p.y)].set_semantic_prompt(prompt);
    }
    fn assert_scrollbar(s: &PageList, offset: usize) {
        assert_eq!(
            s.scrollbar(),
            Scrollbar {
                total: s.total_rows,
                offset,
                len: usize::from(s.rows)
            }
        );
    }
    fn assert_viewport_point(s: &PageList, y: usize) {
        assert_eq!(
            s.point_from_pin(
                Tag::Screen,
                s.pin(Point::Viewport(Coordinate::default())).unwrap()
            ),
            Some(screen_point(0, y))
        );
    }
    #[test]
    fn page_list_grow_allows_exceeding_max_size_for_active_area() {
        let cap = initial_capacity(5);
        let mut s = PageList::init(5, cap.rows, Some(0)).unwrap();
        assert_eq!(usize::from(s.rows), s.total_rows);
        s.grow().unwrap();
        let start_pages = s.total_pages();
        assert!(start_pages >= 2);
        for node in s.nodes.iter_mut().flatten() {
            node.data.size.rows = 1;
            node.data.capacity.rows = 1;
        }
        s.total_rows = start_pages;
        s.grow().unwrap();
        assert_eq!(start_pages + 1, s.total_pages());
    }
    #[test]
    fn page_list_grow_prune_required_with_a_single_page() {
        let mut s = PageList::init(80, 24, None).unwrap();
        while Page::layout(s.node(s.first.unwrap()).unwrap().data.capacity).total_size <= std_size()
        {
            s.increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::GraphemeBytes))
                .unwrap();
        }
        assert!(s.first.is_some());
        assert_eq!(s.first, s.last);
        let page = &s.node(s.first.unwrap()).unwrap().data;
        let rem = page.capacity.rows - page.size.rows;
        for _ in 0..rem {
            assert!(s.grow().unwrap().is_none());
        }
        let new = s.grow().unwrap();
        assert!(new.is_some());
        assert_ne!(new, s.first);
        assert_scrollbar(&s, s.total_rows - usize::from(s.rows));
    }
    #[test]
    fn page_list_scroll_delta_row_back_without_space_preserves_active() {
        let mut s = PageList::init(80, 24, None).unwrap();
        s.scroll(Scroll::DeltaRow(-1));
        assert_viewport_point(&s, 0);
        assert_eq!(s.viewport, Viewport::Active);
        assert_scrollbar(&s, s.total_rows - usize::from(s.rows));
    }
    #[test]
    fn page_list_jump_zero_prompts() {
        let mut s = PageList::init(5, 3, None).unwrap();
        grow_rows(&mut s, 3);
        assert_eq!(s.first, s.last);
        mark_prompt(&mut s, 1, SemanticPrompt::Prompt);
        mark_prompt(&mut s, 5, SemanticPrompt::Prompt);
        s.scroll(Scroll::DeltaPrompt(0));
        assert_eq!(s.viewport, Viewport::Active);
        assert_scrollbar(&s, s.total_rows - usize::from(s.rows));
    }
    #[test]
    fn screen_jump_back_one_prompt() {
        let mut s = PageList::init(5, 3, None).unwrap();
        grow_rows(&mut s, 3);
        assert_eq!(s.first, s.last);
        mark_prompt(&mut s, 1, SemanticPrompt::Prompt);
        mark_prompt(&mut s, 5, SemanticPrompt::Prompt);
        for _ in 0..2 {
            s.scroll(Scroll::DeltaPrompt(-1));
            assert_eq!(s.viewport, Viewport::Pin);
            assert_viewport_point(&s, 1);
            assert_scrollbar(&s, 1);
        }
        for _ in 0..2 {
            s.scroll(Scroll::DeltaPrompt(1));
            assert_eq!(s.viewport, Viewport::Active);
            assert_scrollbar(&s, s.total_rows - usize::from(s.rows));
        }
    }
    #[test]
    fn screen_jump_forward_prompt_skips_multiline_continuation() {
        let mut s = PageList::init(5, 3, None).unwrap();
        grow_rows(&mut s, 7);
        mark_prompt(&mut s, 1, SemanticPrompt::Prompt);
        mark_prompt(&mut s, 2, SemanticPrompt::PromptContinuation);
        mark_prompt(&mut s, 3, SemanticPrompt::PromptContinuation);
        mark_prompt(&mut s, 6, SemanticPrompt::Prompt);
        for start in [1, 2] {
            s.scroll(Scroll::Row(start));
            s.scroll(Scroll::DeltaPrompt(1));
            assert_eq!(s.viewport, Viewport::Pin);
            assert_viewport_point(&s, 6);
        }
    }
    fn cache_erase_fixture(pages: usize) -> (PageList, usize) {
        let mut s = PageList::init(80, 24, None).unwrap();
        let capacity = usize::from(s.node(s.last.unwrap()).unwrap().data.capacity.rows);
        grow_rows(&mut s, capacity * pages);
        (s, capacity)
    }
    fn cache_pin(s: &mut PageList, y: usize) {
        s.scroll(Scroll::Pin(s.pin(screen_point(0, y)).unwrap()));
        assert_eq!(s.viewport, Viewport::Pin);
        assert_scrollbar(s, y);
    }
    #[test]
    fn page_list_erase_rows_invalidates_viewport_offset_cache() {
        let (mut s, capacity) = cache_erase_fixture(3);
        cache_pin(&mut s, capacity);
        let erased = capacity / 2;
        s.erase_rows(
            Point::History(Coordinate::default()),
            Some(Point::History(Coordinate {
                x: 0,
                y: (erased - 1) as u32,
            })),
        )
        .unwrap();
        assert_scrollbar(&s, capacity - erased);
    }
    #[test]
    fn page_list_erase_row_invalidates_viewport_offset_cache() {
        let (mut s, capacity) = cache_erase_fixture(3);
        cache_pin(&mut s, capacity);
        s.erase_row(Point::History(Coordinate::default())).unwrap();
        assert_scrollbar(&s, capacity - 1);
    }
    #[test]
    fn page_list_erase_row_bounded_invalidates_viewport_offset_cache() {
        let (mut s, _) = cache_erase_fixture(3);
        cache_pin(&mut s, 4);
        s.erase_row_bounded(Point::History(Coordinate::default()), 10)
            .unwrap();
        assert_scrollbar(&s, 3);
    }
    #[test]
    fn page_list_erase_row_bounded_multi_page_invalidates_viewport_offset_cache() {
        let (mut s, capacity) = cache_erase_fixture(3);
        cache_pin(&mut s, capacity + 1);
        s.erase_row_bounded(Point::History(Coordinate::default()), capacity + 10)
            .unwrap();
        assert_scrollbar(&s, capacity);
    }
    #[test]
    fn page_list_erase_row_bounded_full_page_shift_invalidates_viewport_offset_cache() {
        let (mut s, capacity) = cache_erase_fixture(4);
        cache_pin(&mut s, 5);
        s.erase_row_bounded(Point::History(Coordinate::default()), capacity * 2 + 10)
            .unwrap();
        assert_scrollbar(&s, 4);
    }
    #[test]
    fn page_list_erase_row_bounded_exhausts_pages_invalidates_viewport_offset_cache() {
        let (mut s, capacity) = cache_erase_fixture(3);
        let total_rows_before = s.total_rows;
        assert!(total_rows_before > usize::from(s.rows));
        cache_pin(&mut s, capacity * 2 + 10);
        s.erase_row_bounded(Point::History(Coordinate::default()), total_rows_before * 2)
            .unwrap();
        assert_scrollbar(&s, capacity * 2 + 9);
    }
    #[test]
    fn page_list_prompt_iterator_right_down_continuation_at_start() {
        let mut s = PageList::init(2, 20, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        mark_prompt(&mut s, 0, SemanticPrompt::PromptContinuation);
        mark_prompt(&mut s, 1, SemanticPrompt::PromptContinuation);
        mark_prompt(&mut s, 5, SemanticPrompt::Prompt);
        let mut it = s.prompt_iterator(Direction::RightDown, screen_point(0, 0), None);
        for y in [0, 5] {
            assert_eq!(
                s.point_from_pin(Tag::Screen, it.next().unwrap()),
                Some(screen_point(0, y))
            );
        }
        assert!(it.next().is_none());
    }
    #[test]
    fn page_list_prompt_iterator_right_down_with_prompt_before_continuation() {
        let mut s = PageList::init(2, 20, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        mark_prompt(&mut s, 2, SemanticPrompt::Prompt);
        mark_prompt(&mut s, 3, SemanticPrompt::PromptContinuation);
        mark_prompt(&mut s, 4, SemanticPrompt::PromptContinuation);
        let mut it = s.prompt_iterator(Direction::RightDown, screen_point(0, 3), None);
        assert_eq!(
            s.point_from_pin(Tag::Screen, it.next().unwrap()),
            Some(screen_point(0, 3))
        );
        assert!(it.next().is_none());
    }
    #[test]
    fn page_list_prompt_iterator_right_down_limit_inclusive() {
        let mut s = PageList::init(2, 20, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        mark_prompt(&mut s, 5, SemanticPrompt::Prompt);
        mark_prompt(&mut s, 10, SemanticPrompt::Prompt);
        let mut it = s.prompt_iterator(
            Direction::RightDown,
            screen_point(0, 0),
            Some(screen_point(0, 5)),
        );
        assert_eq!(
            s.point_from_pin(Tag::Screen, it.next().unwrap()),
            Some(screen_point(0, 5))
        );
        assert!(it.next().is_none());
    }
    #[test]
    fn page_list_prompt_iterator_left_up_limit_inclusive() {
        let mut s = PageList::init(2, 20, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        mark_prompt(&mut s, 5, SemanticPrompt::Prompt);
        mark_prompt(&mut s, 10, SemanticPrompt::Prompt);
        let mut it = s.prompt_iterator(
            Direction::LeftUp,
            screen_point(0, 10),
            Some(screen_point(0, 15)),
        );
        assert_eq!(
            s.point_from_pin(Tag::Screen, it.next().unwrap()),
            Some(screen_point(0, 10))
        );
        assert!(it.next().is_none());
    }
    #[test]
    fn page_list_erase_resets_viewport_to_active_if_moves_within_active() {
        let (mut s, _) = cache_erase_fixture(5);
        s.scroll(Scroll::DeltaRow(-(s.total_rows as isize)));
        assert_eq!(s.viewport, Viewport::Top);
        s.erase_rows(Point::History(Coordinate::default()), None)
            .unwrap();
        assert_eq!(s.viewport, Viewport::Active);
    }
    #[test]
    fn page_list_erase_resets_viewport_if_inside_erased_page_but_not_active() {
        let (mut s, _) = cache_erase_fixture(5);
        s.scroll(Scroll::DeltaRow(-(s.total_rows as isize)));
        assert_eq!(s.viewport, Viewport::Top);
        s.erase_rows(
            Point::History(Coordinate::default()),
            Some(Point::History(Coordinate { x: 0, y: 2 })),
        )
        .unwrap();
        assert_eq!(s.viewport, Viewport::Top);
    }
    #[test]
    fn page_list_erase_resets_viewport_to_active_if_top_is_inside_active() {
        let (mut s, _) = cache_erase_fixture(5);
        s.scroll(Scroll::Top);
        s.erase_rows(Point::History(Coordinate::default()), None)
            .unwrap();
        assert_eq!(s.viewport, Viewport::Active);
    }
    fn shrink_rows_cursor_fixture(y: usize) -> (PageList, TrackedPin) {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        assert_eq!(s.total_rows, 10);
        assert_eq!(s.first, s.last);
        for row in 0..10 {
            *s.get_cell_mut(active_point(0, row)).unwrap() = Cell::init(row as u32);
        }
        let p = s.track_pin(s.pin(active_point(0, y)).unwrap()).unwrap();
        let cursor = s
            .point_from_pin(Tag::Active, s.tracked_pin(p).unwrap())
            .unwrap();
        assert_eq!(s.get_cell(cursor).unwrap().codepoint(), y as u32);
        s.resize(Resize {
            rows: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 5);
        assert_eq!(s.total_rows, 10);
        (s, p)
    }
    #[test]
    fn page_list_resize_no_reflow_less_rows_cursor_on_bottom() {
        let (s, p) = shrink_rows_cursor_fixture(9);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(0, 4))
        );
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 5))
        );
    }
    #[test]
    fn page_list_resize_no_reflow_less_rows_cursor_in_scrollback() {
        let (s, p) = shrink_rows_cursor_fixture(2);
        assert!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap())
                .is_none()
        );
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.tracked_pin(p).unwrap()),
            Some(screen_point(0, 2))
        );
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 5))
        );
    }
    #[test]
    fn page_list_resize_no_reflow_more_rows_contains_viewport() {
        let mut s = PageList::init(5, 5, Some(1)).unwrap();
        assert_eq!(s.first, s.last);
        s.grow().unwrap();
        assert_eq!(s.rows, 5);
        assert_eq!(s.total_rows, 6);
        s.scroll(Scroll::DeltaRow(-1));
        assert_eq!(s.viewport, Viewport::Top);
        s.resize(Resize {
            rows: Some(7),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 7);
        assert_eq!(s.total_rows, 7);
        assert_eq!(s.viewport, Viewport::Top);
    }

    fn set_raw(s: &mut PageList, x: u16, y: usize, cp: u32, wide: Wide) {
        let cell = s.get_cell_mut(screen_point(x, y)).unwrap();
        *cell = Cell::init(cp);
        cell.set_wide(wide);
    }
    fn set_wrap(s: &mut PageList, y: usize, wrap: bool, continuation: bool) {
        let p = s.pin(screen_point(0, y)).unwrap();
        let row = &mut s.node_mut(p.node).unwrap().data.rows[usize::from(p.y)];
        row.set_wrap(wrap);
        row.set_wrap_continuation(continuation);
    }
    fn assert_cell(s: &PageList, x: u16, y: usize, cp: u32, wide: Wide) {
        let c = s.get_cell(screen_point(x, y)).unwrap();
        assert_eq!(c.codepoint(), cp);
        assert_eq!(c.wide(), wide);
    }
    fn spacer_fixture(rows: u16) -> PageList {
        let mut s = PageList::init(2, rows, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        set_raw(&mut s, 0, 0, 'x' as u32, Wide::Narrow);
        set_raw(&mut s, 1, 0, 0, Wide::SpacerHead);
        set_wrap(&mut s, 0, true, false);
        set_raw(&mut s, 0, 1, '😀' as u32, Wide::Wide);
        set_raw(&mut s, 1, 1, 0, Wide::SpacerTail);
        s
    }
    #[test]
    fn page_list_resize_no_reflow_more_cols_with_spacer_head() {
        let mut s = spacer_fixture(3);
        s.resize(Resize {
            cols: Some(3),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 3);
        assert_eq!(s.total_rows, 3);
        assert_eq!(s.first, s.last);
        assert_cell(&s, 0, 0, 'x' as u32, Wide::Narrow);
        assert_cell(&s, 1, 0, 0, Wide::Narrow);
        assert_cell(&s, 2, 0, 0, Wide::Narrow);
    }
    #[test]
    fn page_list_resize_no_reflow_grow_cols_fast_path_with_spacer_head() {
        let mut s = PageList::init(10, 3, Some(0)).unwrap();
        s.resize(Resize {
            cols: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        set_raw(&mut s, 0, 0, 'x' as u32, Wide::Narrow);
        for y in 0..2 {
            set_raw(&mut s, 4, y, 0, Wide::SpacerHead);
            set_wrap(&mut s, y, true, false);
        }
        s.resize(Resize {
            cols: Some(10),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 10);
        for y in 0..2 {
            let p = s.pin(screen_point(4, y)).unwrap();
            let (row, cell) = p.row_and_cell(&s).unwrap();
            assert_eq!(cell.wide(), Wide::Narrow);
            assert!(!row.wrap());
        }
    }
    fn assert_pages_full_except_last(s: &PageList) {
        let mut node = s.first;
        while let Some(id) = node {
            if Some(id) == s.last {
                break;
            }
            let n = s.node(id).unwrap();
            assert_eq!(n.data.size.rows, n.data.capacity.rows);
            node = n.next;
        }
    }
    #[test]
    fn page_list_resize_no_reflow_more_cols_forces_less_rows_per_page() {
        assert!(initial_capacity(5).rows >= 150);
        let mut s = PageList::init(5, 150, Some(0)).unwrap();
        let mut cols = 50;
        while initial_capacity(cols).rows >= 150 {
            cols += 50;
        }
        s.resize(Resize {
            cols: Some(cols),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, cols);
        assert_eq!(s.total_rows, 150);
        assert_pages_full_except_last(&s);
        let page = &s.node(s.first.unwrap()).unwrap().data;
        assert_eq!(page.size.rows, page.capacity.rows);
        let previous_rows = page.size.rows;
        cols = page.size.cols + 50;
        while initial_capacity(cols).rows >= previous_rows {
            cols += 50;
        }
        s.resize(Resize {
            cols: Some(cols),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, cols);
        assert_eq!(s.total_rows, 150);
        assert_pages_full_except_last(&s);
    }
    #[test]
    fn page_list_resize_more_rows_and_cols_doesnt_fit_in_single_std_page() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        assert!(initial_capacity(600).rows < 600);
        s.resize(Resize {
            cols: Some(600),
            rows: Some(600),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 600);
        assert_eq!(s.rows, 600);
        assert_eq!(s.total_rows, 600);
    }
    #[test]
    fn page_list_resize_no_reflow_more_cols_forces_smaller_cap() {
        let cap = initial_capacity(100);
        let cap2 = initial_capacity(500);
        assert_eq!(cap2.cols, 500);
        assert!(cap2.rows < cap.rows);
        let mut s = PageList::init(cap.cols, cap.rows, None).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..usize::from(s.rows) {
            for x in 0..s.cols {
                set_raw(&mut s, x, y, 'A' as u32, Wide::Narrow);
            }
        }
        let rows = s.total_rows;
        s.resize(Resize {
            cols: Some(cap2.cols),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, rows);
        for p in s.row_iterator(Direction::RightDown, screen_point(0, 0), None) {
            let cells = p.cells(&s, CellSubset::All).unwrap();
            assert_eq!(cells.len(), usize::from(cap2.cols));
            assert_eq!(cells[0].codepoint(), 'A' as u32);
        }
    }
    #[test]
    fn page_list_resize_no_reflow_more_rows_adds_blank_rows_if_cursor_at_bottom() {
        let mut s = PageList::init(5, 3, None).unwrap();
        grow_rows(&mut s, 2);
        assert_eq!(s.first, s.last);
        for y in 0..s.total_rows {
            set_raw(&mut s, 0, y, y as u32, Wide::Narrow);
        }
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 2))
        );
        let p = s
            .track_pin(s.pin(active_point(0, usize::from(s.rows - 2))).unwrap())
            .unwrap();
        let original_cursor = s
            .point_from_pin(Tag::Active, s.tracked_pin(p).unwrap())
            .unwrap();
        assert_eq!(s.get_cell(original_cursor).unwrap().codepoint(), 3);
        s.resize(Resize {
            rows: Some(10),
            reflow: false,
            cursor: Some(Cursor {
                x: 0,
                y: s.rows - 2,
            }),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        assert_eq!(s.rows, 10);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(original_cursor)
        );
        assert_eq!(s.total_rows, 12);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen_point(0, 2))
        );
        for y in 0..3 {
            assert_eq!(
                s.get_cell(active_point(0, y)).unwrap().codepoint(),
                (y + 2) as u32
            );
        }
    }
    #[test]
    fn page_list_resize_reflow_invalidates_viewport_offset_cache() {
        let mut s = PageList::init(2, 4, None).unwrap();
        grow_rows(&mut s, 20);
        for y in 0..usize::from(s.rows) {
            set_wrap(&mut s, y, y % 2 == 0, y % 2 != 0);
            for x in 0..s.cols {
                set_raw(&mut s, x, y, 'A' as u32, Wide::Narrow);
            }
        }
        cache_pin(&mut s, 10);
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_scrollbar(&s, 5);
    }
    #[test]
    fn page_list_resize_reflow_more_cols_creates_multiple_pages() {
        let mut cols = 100;
        while initial_capacity(cols).rows >= 100 {
            cols += 100;
        }
        let cap = initial_capacity(cols);
        let mut s = PageList::init(cap.cols, cap.rows, None).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..usize::from(s.rows) {
            set_wrap(&mut s, y, y % 2 == 0, y % 2 != 0);
            set_raw(&mut s, 0, y, 'A' as u32, Wide::Narrow);
        }
        let newcap = cap
            .adjust(Adjustment {
                cols: Some(cap.cols + 100),
            })
            .unwrap();
        assert!(newcap.rows < cap.rows);
        s.resize(Resize {
            cols: Some(newcap.cols),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, newcap.cols);
        assert_eq!(s.total_rows, usize::from(cap.rows));
        for n in s.nodes.iter().flatten() {
            assert_eq!(n.data.capacity.cols, newcap.cols);
            assert_eq!(n.data.capacity.rows, newcap.rows);
        }
        assert!(s.total_pages() > 1);
    }
    fn two_page_fixture(cols: u16, extra: usize, max: Option<usize>) -> PageList {
        let mut s = PageList::init(cols, 10, max).unwrap();
        assert_eq!(s.total_pages(), 1);
        let page = &s.node(s.first.unwrap()).unwrap().data;
        let rem = usize::from(page.capacity.rows - page.size.rows);
        grow_rows(&mut s, rem);
        assert_eq!(s.total_pages(), 1);
        grow_rows(&mut s, extra);
        assert_eq!(s.total_pages(), 2);
        s
    }
    fn wrap_page_boundary(s: &mut PageList) {
        let y = usize::from(s.node(s.first.unwrap()).unwrap().data.size.rows - 1);
        set_wrap(s, y, true, false);
        set_wrap(s, y + 1, false, true);
        for row in [y, y + 1] {
            for x in 0..s.cols {
                set_raw(s, x, row, u32::from(x), Wide::Narrow);
            }
        }
    }
    fn assert_active_row(s: &PageList, y: usize, wrap: bool, continuation: bool, cps: &[u32]) {
        let p = s.pin(active_point(0, y)).unwrap();
        let (row, _) = p.row_and_cell(s).unwrap();
        assert_eq!(row.wrap(), wrap);
        assert_eq!(row.wrap_continuation(), continuation);
        let cells = p.cells(s, CellSubset::All).unwrap();
        for (cell, cp) in cells.iter().zip(cps) {
            assert_eq!(cell.codepoint(), *cp);
        }
    }
    #[test]
    fn page_list_resize_reflow_more_cols_wrap_across_page_boundary() {
        let mut s = two_page_fixture(2, 1, Some(0));
        wrap_page_boundary(&mut s);
        let end_rows = s.total_rows - 1;
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, end_rows);
        assert_active_row(&s, 8, false, false, &[0; 4]);
        for c in s
            .pin(active_point(0, 8))
            .unwrap()
            .cells(&s, CellSubset::All)
            .unwrap()
        {
            assert!(!c.has_text());
        }
        assert_active_row(&s, 9, false, false, &[0, 1, 0, 1]);
    }
    #[test]
    fn page_list_resize_reflow_more_cols_wrap_across_page_boundary_cursor_in_second_page() {
        let mut s = two_page_fixture(2, 1, Some(0));
        wrap_page_boundary(&mut s);
        let p = s.track_pin(s.pin(active_point(1, 9)).unwrap()).unwrap();
        assert_eq!(Some(s.tracked_pin(p).unwrap().node), s.last);
        let end_rows = s.total_rows - 1;
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, end_rows);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(3, 9))
        );
        assert_active_row(&s, 9, false, false, &[0, 1, 0, 1]);
    }
    #[test]
    fn page_list_resize_reflow_less_cols_wrap_across_page_boundary_cursor_in_second_page() {
        let mut s = two_page_fixture(5, 5, None);
        wrap_page_boundary(&mut s);
        let p = s.track_pin(s.pin(active_point(2, 5)).unwrap()).unwrap();
        assert_eq!(Some(s.tracked_pin(p).unwrap().node), s.last);
        assert_eq!(s.tracked_pin(p).unwrap().y, 0);
        s.resize(Resize {
            cols: Some(4),
            cursor: Some(Cursor { x: 2, y: 5 }),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(3, 5))
        );
        for y in [3, 7] {
            assert_active_row(&s, y, false, false, &[0; 4]);
            for c in s
                .pin(active_point(0, y))
                .unwrap()
                .cells(&s, CellSubset::All)
                .unwrap()
            {
                assert!(!c.has_text());
            }
        }
        assert_active_row(&s, 4, true, false, &[0, 1, 2, 3]);
        assert_active_row(&s, 5, true, true, &[4, 0, 1, 2]);
        assert_active_row(&s, 6, false, true, &[3, 4]);
    }
    fn grow_cursor_fixture(wrapped_rows: usize, cursor_y: usize) -> (PageList, TrackedPin) {
        let mut s = PageList::init(2, 4, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..wrapped_rows {
            set_wrap(&mut s, y, y + 1 < wrapped_rows, y != 0);
            for x in 0..s.cols {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        let p = s
            .track_pin(s.pin(active_point(1, cursor_y)).unwrap())
            .unwrap();
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, 4);
        (s, p)
    }
    #[test]
    fn page_list_resize_reflow_more_cols_cursor_in_wrapped_row() {
        let (s, p) = grow_cursor_fixture(2, 1);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(3, 0))
        );
    }
    #[test]
    fn page_list_resize_reflow_more_cols_cursor_in_not_wrapped_row() {
        let (s, p) = grow_cursor_fixture(2, 0);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(1, 0))
        );
    }
    #[test]
    fn page_list_resize_reflow_more_cols_cursor_in_wrapped_row_that_isnt_unwrapped() {
        let (s, p) = grow_cursor_fixture(3, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap()),
            Some(active_point(1, 1))
        );
    }
    #[test]
    fn page_list_resize_reflow_more_cols_unwrap_wide_spacer_head() {
        let mut s = spacer_fixture(2);
        set_wrap(&mut s, 1, false, true);
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, 2);
        assert_eq!(s.first, s.last);
        assert_cell(&s, 0, 0, 'x' as u32, Wide::Narrow);
        assert!(
            !s.pin(screen_point(0, 0))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .wrap()
        );
        assert_cell(&s, 1, 0, '😀' as u32, Wide::Wide);
        assert_cell(&s, 2, 0, 0, Wide::SpacerTail);
    }
    #[test]
    fn page_list_resize_reflow_more_cols_unwrap_wide_spacer_head_across_two_rows() {
        let mut s = PageList::init(2, 3, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for (x, y) in [(0, 0), (1, 0), (0, 1)] {
            set_raw(&mut s, x, y, 'x' as u32, Wide::Narrow);
        }
        set_raw(&mut s, 1, 1, 0, Wide::SpacerHead);
        set_raw(&mut s, 0, 2, '😀' as u32, Wide::Wide);
        set_raw(&mut s, 1, 2, 0, Wide::SpacerTail);
        for y in 0..3 {
            set_wrap(&mut s, y, y < 2, y > 0);
        }
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, 3);
        assert_eq!(s.first, s.last);
        for x in 0..3 {
            assert_cell(&s, x, 0, 'x' as u32, Wide::Narrow);
        }
        assert!(
            s.pin(screen_point(0, 0))
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .wrap()
        );
        assert_cell(&s, 3, 0, 0, Wide::SpacerHead);
        assert_cell(&s, 0, 1, '😀' as u32, Wide::Wide);
        assert_cell(&s, 1, 1, 0, Wide::SpacerTail);
    }
    #[test]
    fn page_list_resize_reflow_less_cols_cursor_goes_to_scrollback() {
        let mut s = PageList::init(4, 2, None).unwrap();
        assert_eq!(s.first, s.last);
        for y in 0..2 {
            for x in 0..4 {
                set_raw(&mut s, x, y, u32::from(x), Wide::Narrow);
            }
        }
        let p = s.track_pin(s.pin(active_point(2, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 4);
        assert!(
            s.point_from_pin(Tag::Active, s.tracked_pin(p).unwrap())
                .is_none()
        );
    }

    #[test]
    fn page_list_resize_reflow_exceeds_hyperlink_memory_forcing_capacity_increase() {
        use crate::hyperlink::{Hyperlink, HyperlinkId};
        let mut s = two_page_fixture(2, 1, Some(0));
        assert_ne!(s.first, s.last);
        assert_eq!(s.node(s.first.unwrap()).unwrap().next, s.last);
        for (index, node) in [s.first.unwrap(), s.last.unwrap()].into_iter().enumerate() {
            let page = &mut s.node_mut(node).unwrap().data;
            let (x, y) = if index == 0 {
                (page.size.cols - 1, page.size.rows - 1)
            } else {
                (0, 0)
            };
            let id = page
                .insert_hyperlink(Hyperlink {
                    id: HyperlinkId::Implicit(index as u32),
                    uri: vec![b'a'; page::STRING_BYTES_DEFAULT as usize - 1],
                })
                .unwrap();
            let i = page.cell_index(usize::from(x), usize::from(y)).unwrap();
            page.cells[i] = Cell::init('X' as u32);
            page.rows[usize::from(y)].set_wrap(index == 0);
            page.rows[usize::from(y)].set_wrap_continuation(index == 1);
            page.set_hyperlink(usize::from(x), usize::from(y), id)
                .unwrap();
            assert_eq!(
                page.insert_hyperlink(Hyperlink {
                    id: HyperlinkId::Implicit(index as u32 + 1),
                    uri: b"AAAAAAAAAAAAAAAAAAAAAAAAAA".to_vec()
                }),
                Err(Error::StringsOutOfMemory)
            );
        }
        s.resize(Resize {
            cols: Some(s.cols + 1),
            ..Resize::default()
        })
        .unwrap();
        s.assert_integrity();
    }
    #[test]
    fn page_list_resize_reflow_exceeds_grapheme_memory_forcing_capacity_increase() {
        let mut s = two_page_fixture(2, 1, Some(0));
        assert_ne!(s.first, s.last);
        assert_eq!(s.node(s.first.unwrap()).unwrap().next, s.last);
        for (index, node) in [s.first.unwrap(), s.last.unwrap()].into_iter().enumerate() {
            let page = &mut s.node_mut(node).unwrap().data;
            let (x, y) = if index == 0 {
                (page.size.cols - 1, page.size.rows - 1)
            } else {
                (0, 0)
            };
            let i = page.cell_index(usize::from(x), usize::from(y)).unwrap();
            page.cells[i] = Cell::init('X' as u32);
            page.rows[usize::from(y)].set_wrap(true);
            page.set_graphemes(
                usize::from(x),
                usize::from(y),
                &vec!['a' as u32; (page::GRAPHEME_BYTES_DEFAULT as usize - 1) / 4],
            )
            .unwrap();
            assert!(!page.can_allocate_grapheme(16 * 4));
        }
        s.resize(Resize {
            cols: Some(s.cols + 1),
            ..Resize::default()
        })
        .unwrap();
        s.assert_integrity();
    }
    #[test]
    fn page_list_resize_reflow_exceeds_style_memory_forcing_capacity_increase() {
        use ghostty_vt::color::Rgb;

        use crate::style::{Color, Style};
        let mut s = two_page_fixture(page::STD_CAPACITY.styles - 1, 1, Some(0));
        assert_ne!(s.first, s.last);
        assert_eq!(s.node(s.first.unwrap()).unwrap().next, s.last);
        for (index, node) in [s.first.unwrap(), s.last.unwrap()].into_iter().enumerate() {
            let page = &mut s.node_mut(node).unwrap().data;
            let y = if index == 0 { page.size.rows - 1 } else { 0 };
            for x in 0..usize::from(page.size.cols) {
                let color = Color::Rgb(Rgb {
                    r: x as u8,
                    g: (x >> 8) as u8,
                    b: (x >> 16) as u8,
                });
                let style = if index == 0 {
                    Style {
                        bg_color: color,
                        ..Style::default()
                    }
                } else {
                    Style {
                        fg_color: color,
                        ..Style::default()
                    }
                };
                let Ok(id) = page.styles.add(style) else {
                    break;
                };
                let i = page.cell_index(x, usize::from(y)).unwrap();
                page.cells[i] = Cell::init('X' as u32);
                page.cells[i].set_style_id(id);
                page.rows[usize::from(y)].set_styled(true);
                page.rows[usize::from(y)].set_wrap(index == 0);
                page.rows[usize::from(y)].set_wrap_continuation(index == 1);
            }
        }
        s.resize(Resize {
            cols: Some(s.cols * 2),
            ..Resize::default()
        })
        .unwrap();
        s.assert_integrity();
    }
    fn kitty_fixture(start: u16) -> PageList {
        let mut s = PageList::init(4, 2, Some(0)).unwrap();
        assert_eq!(s.first, s.last);
        for x in start..s.cols - 1 {
            set_raw(&mut s, x, 0, 0x10eeee, Wide::Narrow);
        }
        s.node_mut(s.first.unwrap()).unwrap().data.rows[0].set_kitty_virtual_placeholder(true);
        s
    }
    #[test]
    fn page_list_resize_reflow_less_cols_copy_kitty_placeholder() {
        let mut s = kitty_fixture(0);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 2);
        for p in s.row_iterator(Direction::RightDown, active_point(0, 0), None) {
            for x in 0..s.cols - 1 {
                assert!(
                    Pin { x, ..p }
                        .row_and_cell(&s)
                        .unwrap()
                        .0
                        .kitty_virtual_placeholder()
                );
            }
        }
    }
    #[test]
    fn page_list_resize_reflow_more_cols_clears_kitty_placeholder() {
        let mut s = kitty_fixture(0);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert_eq!(s.total_rows, 2);
        let mut it = s.row_iterator(Direction::RightDown, active_point(0, 0), None);
        assert!(
            it.next()
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .kitty_virtual_placeholder()
        );
        assert!(
            !it.next()
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .kitty_virtual_placeholder()
        );
        assert!(it.next().is_none());
    }
    #[test]
    fn page_list_resize_reflow_wrap_moves_kitty_placeholder() {
        let mut s = kitty_fixture(2);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        assert_eq!(s.total_rows, 2);
        let mut it = s.row_iterator(Direction::RightDown, active_point(0, 0), None);
        assert!(
            !it.next()
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .kitty_virtual_placeholder()
        );
        assert!(
            it.next()
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .kitty_virtual_placeholder()
        );
        assert!(it.next().is_none());
    }
    #[test]
    fn page_list_resize_reflow_grapheme_map_capacity_exceeded() {
        let mut s = PageList::init(4, 10, Some(0)).unwrap();
        assert_eq!(s.total_pages(), 1);
        let page = &s.node(s.first.unwrap()).unwrap().data;
        let capacity = page.grapheme_capacity();
        let per_page = capacity / 2 + capacity / 4;
        let rem = usize::from(page.capacity.rows - page.size.rows);
        grow_rows(&mut s, rem);
        assert_eq!(s.total_pages(), 1);
        grow_rows(&mut s, per_page);
        assert_eq!(s.total_pages(), 2);
        assert_ne!(s.first, s.last);
        assert_eq!(s.node(s.first.unwrap()).unwrap().next, s.last);
        for (index, node) in [s.first.unwrap(), s.last.unwrap()].into_iter().enumerate() {
            let page = &mut s.node_mut(node).unwrap().data;
            let start = if index == 0 {
                usize::from(page.size.rows) - per_page
            } else {
                0
            };
            let count = per_page.min(usize::from(page.size.rows));
            for y in start..start + count {
                let i = page.cell_index(0, y).unwrap();
                page.cells[i] = Cell::init(if index == 0 { 'A' } else { 'B' } as u32);
                page.append_grapheme(0, y, 0x0301 + index as u32).unwrap();
            }
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 2);
        s.assert_integrity();
    }
    #[test]
    fn page_list_resize_grow_cols_with_unwrap_fixes_viewport_pin() {
        let mut s = PageList::init(2, 10, None).unwrap();
        grow_rows(&mut s, 30);
        assert_eq!(s.total_rows, 40);
        for y in 0..40 {
            set_wrap(&mut s, y, y % 2 == 0, y % 2 != 0);
            for x in 0..2 {
                set_raw(&mut s, x, y, 'A' as u32, Wide::Narrow);
            }
        }
        s.scroll(Scroll::Pin(s.pin(screen_point(0, 28)).unwrap()));
        assert_eq!(s.viewport, Viewport::Pin);
        assert!(s.get_bottom_right(Tag::Viewport).is_some());
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 4);
        assert!(s.total_rows < 40);
        assert!(s.get_bottom_right(Tag::Viewport).is_some());
    }
    fn oversize_first(s: &mut PageList) {
        while Page::layout(s.node(s.first.unwrap()).unwrap().data.capacity).total_size <= std_size()
        {
            s.increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::GraphemeBytes))
                .unwrap();
        }
        assert!(
            Page::layout(s.node(s.first.unwrap()).unwrap().data.capacity).total_size > std_size()
        );
    }
    #[test]
    fn page_list_grow_reuses_non_standard_page_without_leak() {
        let mut s = PageList::init(80, 24, Some(3 * std_size())).unwrap();
        oversize_first(&mut s);
        let first = s.first.unwrap();
        let page = &s.node(first).unwrap().data;
        let rem = usize::from(page.capacity.rows - page.size.rows);
        grow_rows(&mut s, rem);
        s.grow().unwrap();
        assert_ne!(s.first, s.last);
        while s.page_size + std_size() <= s.max_size()
            || s.node(s.last.unwrap()).unwrap().data.size.rows
                < s.node(s.last.unwrap()).unwrap().data.capacity.rows
        {
            s.grow().unwrap();
        }
        assert!(
            Page::layout(s.node(s.first.unwrap()).unwrap().data.capacity).total_size > std_size()
        );
        assert!(s.total_rows >= usize::from(s.rows));
        assert_eq!(
            s.node(s.last.unwrap()).unwrap().data.size.rows,
            s.node(s.last.unwrap()).unwrap().data.capacity.rows
        );
        let first_page = s.first.unwrap();
        let first_allocation = first_page;
        let tracked = s
            .track_pin(Pin {
                node: first_page,
                x: 0,
                y: 0,
                garbage: false,
            })
            .unwrap();
        s.grow().unwrap();
        assert_ne!(s.first, Some(first_page));
        // Rust may reuse the freed Vec address. Node generations identify the
        // allocation, and oversized pages must never enter the standard pool.
        assert_ne!(s.last, Some(first_allocation));
        assert!(
            s.page_pool
                .iter()
                .all(|page| Page::layout(page.capacity).total_size <= std_size())
        );
        let p = s.tracked_pin(tracked).unwrap();
        assert_eq!(Some(p.node), s.first);
        assert_eq!(p.x, 0);
        assert_eq!(p.y, 0);
        assert!(p.garbage);
        s.assert_integrity();
    }
    #[test]
    fn page_list_grow_non_standard_page_prune_protection() {
        let mut s = PageList::init(80, 600, Some(std_size())).unwrap();
        oversize_first(&mut s);
        let first = s.first.unwrap();
        let first_cap = s.node(first).unwrap().data.capacity.rows;
        while s.node(first).unwrap().data.size.rows < first_cap {
            s.grow().unwrap();
        }
        while s.first == s.last {
            s.grow().unwrap();
        }
        assert_ne!(s.first, s.last);
        let last = s.last.unwrap();
        let second_cap = s.node(last).unwrap().data.capacity.rows;
        while s.node(last).unwrap().data.size.rows < second_cap {
            s.grow().unwrap();
        }
        let would_remain = s.total_rows - usize::from(first_cap) + 1;
        assert!(would_remain < usize::from(s.rows));
        assert_ne!(s.first, s.last);
        assert!(s.page_size + std_size() > s.max_size());
        assert!(s.total_rows >= usize::from(s.rows));
        assert_eq!(s.node(last).unwrap().data.size.rows, second_cap);
        s.grow().unwrap();
        assert!(s.total_rows >= usize::from(s.rows));
    }
    #[test]
    fn page_list_resize_no_reflow_more_cols_remaps_pins_in_backfill_path() {
        let cap = initial_capacity(5);
        let mut s = PageList::init(5, cap.rows, None).unwrap();
        while s.first == s.last {
            s.grow().unwrap();
        }
        let first = s.first.unwrap();
        let second = s.last.unwrap();
        assert_ne!(first, second);
        s.erase_rows(
            Point::History(Coordinate::default()),
            Some(Point::History(Coordinate::default())),
        )
        .unwrap();
        assert!(s.node(first).unwrap().data.size.rows < s.node(first).unwrap().data.capacity.rows);
        let adjusted = s
            .node(second)
            .unwrap()
            .data
            .capacity
            .adjust(Adjustment { cols: Some(6) })
            .unwrap();
        assert!(s.node(second).unwrap().data.capacity.cols < adjusted.cols);
        let tracked = s
            .track_pin(Pin {
                node: second,
                x: 0,
                y: 0,
                garbage: false,
            })
            .unwrap();
        s.node_mut(second).unwrap().data.get_cells_mut(0).unwrap()[0] = Cell::init('X' as u32);
        s.resize(Resize {
            cols: Some(6),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        let p = s.tracked_pin(tracked).unwrap();
        let mut node = s.first;
        let mut found = false;
        while let Some(id) = node {
            if id == p.node {
                found = true;
                break;
            }
            node = s.node(id).unwrap().next;
        }
        assert!(found);
        assert!(p.y < s.node(p.node).unwrap().data.size.rows);
        let cell = p.row_and_cell(&s).unwrap().1;
        assert_eq!(cell.content_tag(), ContentTag::Codepoint);
        assert_eq!(cell.codepoint(), 'X' as u32);
    }
    #[test]
    fn active_erasure_preserves_history_and_regrows_removed_rows() {
        let mut s = PageList::init(8, 4, None).unwrap();
        grow(&mut s, 8);
        for y in 0..s.total_rows {
            set_raw(&mut s, 0, y, 65 + y as u32, Wide::Narrow);
        }
        s.scroll(Scroll::Row(3));
        assert_eq!(s.viewport_row_offset(), 3);
        let total = s.total_rows;
        s.erase_rows(active(0, 0), Some(active(0, 1))).unwrap();
        assert_eq!(s.total_rows, total);
        assert_eq!(s.viewport, Viewport::Pin);
        assert_eq!(s.viewport_row_offset(), 3);
        for y in 0..8 {
            assert_eq!(s.get_cell(screen(0, y)).unwrap().codepoint(), 65 + y);
        }
        assert_eq!(s.get_cell(active(0, 0)).unwrap().codepoint(), 75);
        assert_eq!(s.get_cell(active(0, 1)).unwrap().codepoint(), 76);
        for y in 2..4 {
            assert!(s.get_cell(active(0, y)).unwrap().is_empty());
        }
        finish(&s);
    }

    #[test]
    fn pruning_does_not_populate_invalid_viewport_offset_cache() {
        let mut s = PageList::init(80, 24, Some(1)).unwrap();
        fill_pages(&mut s, 2);
        let target = s.pin(screen(0, 5)).unwrap();
        s.scroll(Scroll::Pin(target));
        assert_eq!(
            s.viewport_pin_row_offset.load(Ordering::Relaxed),
            usize::MAX
        );
        s.grow().unwrap();
        assert_eq!(
            s.viewport_pin_row_offset.load(Ordering::Relaxed),
            usize::MAX
        );
        assert_eq!(s.viewport, Viewport::Pin);
        assert_eq!(s.viewport_row_offset(), 0);
        finish(&s);
    }
    #[test]
    fn pin_iterators_use_inclusive_rows_across_pages() {
        let mut list = PageList::init(4, 4, None).unwrap();
        list.split(list.pin(active_point(0, 2)).unwrap()).unwrap();
        let start = list.pin(active_point(2, 1)).unwrap();
        let limit = list.pin(active_point(1, 2)).unwrap();
        let cells: Vec<_> = start
            .cell_iterator(&list, Direction::RightDown, Some(limit))
            .collect();
        assert_eq!(cells.len(), 6);
        assert_eq!(cells[0], start);
        assert_eq!(cells[5].x, 3);
        let reverse: Vec<_> = limit
            .cell_iterator(&list, Direction::LeftUp, Some(start))
            .collect();
        assert_eq!(reverse.len(), 6);
        assert_eq!(reverse[0], limit);
        assert_eq!(reverse[5].x, 0);
        for (start, limit, direction) in [
            (start, limit, Direction::RightDown),
            (limit, start, Direction::LeftUp),
        ] {
            assert_eq!(start.row_iterator(&list, direction, Some(limit)).count(), 2);
            let pages: Vec<_> = start.page_iterator(&list, direction, Some(limit)).collect();
            assert_eq!(pages.len(), 2);
            assert!(pages.iter().all(|c| c.end - c.start == 1));
        }
    }

    #[test]
    fn row_copy_between_pages_preserves_managed_values_and_metadata() {
        let mut list = PageList::init(4, 4, None).unwrap();
        list.split(list.pin(active_point(0, 2)).unwrap()).unwrap();
        let src = list.pin(active_point(0, 0)).unwrap();
        let dst = list.pin(active_point(0, 2)).unwrap();
        let page = &mut list.node_mut(src.node).unwrap().data;
        page.cells[1] = Cell::init('a' as u32);
        page.append_grapheme(1, 0, 0x301).unwrap();
        let id = page
            .insert_hyperlink(crate::hyperlink::Hyperlink {
                uri: b"https://example.com".to_vec(),
                id: crate::hyperlink::HyperlinkId::Implicit(1),
            })
            .unwrap();
        page.set_hyperlink(1, 0, id).unwrap();
        page.rows[0].set_kitty_virtual_placeholder(true);
        page.rows[0].set_dirty(false);
        let page = &mut list.node_mut(dst.node).unwrap().data;
        page.cells[0] = Cell::init('z' as u32);
        page.rows[0].set_wrap(true);
        page.rows[0].set_dirty(true);
        list.clone_partial_row_between(src, dst, 1, 2).unwrap();
        let page = &list.node(dst.node).unwrap().data;
        assert_eq!(page.get_cells(0).unwrap()[0].codepoint(), 'z' as u32);
        assert!(page.rows[0].wrap());
        assert!(page.rows[0].dirty());
        assert!(page.rows[0].kitty_virtual_placeholder());
        assert_eq!(
            page.lookup_grapheme(page.cell_index(1, 0).unwrap()),
            Some(&[0x301][..])
        );
        assert!(
            page.lookup_hyperlink(page.cell_index(1, 0).unwrap())
                .is_some()
        );
        list.clone_row_between(src, dst).unwrap();
        let page = &list.node(dst.node).unwrap().data;
        assert_eq!(page.get_cells(0).unwrap()[0].codepoint(), 0);
        assert!(!page.rows[0].wrap());
        assert!(!page.rows[0].dirty());
        list.assert_integrity();
    }

    #[test]
    fn erase_empty_history_and_top_viewport() {
        let mut list = PageList::init(4, 4, None).unwrap();
        list.erase_rows(Point::History(Coordinate::default()), None)
            .unwrap();
        assert_eq!(list.total_rows, 4);
        list.grow().unwrap();
        list.scroll(Scroll::Top);
        list.erase_rows(Point::History(Coordinate::default()), None)
            .unwrap();
        assert_eq!(list.total_rows, 4);
        assert_eq!(list.viewport, Viewport::Active);
        list.assert_integrity();
    }
}
