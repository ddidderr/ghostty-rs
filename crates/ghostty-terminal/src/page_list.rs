//! Linked terminal pages, byte-bounded history, and stable tracked coordinates.
use crate::{
    page::{
        self, Adjustment, Capacity, Cell, Error, OwnedCell, Page, Row, SemanticContent,
        SemanticPrompt, Wide,
    },
    point::{Coordinate, Point, Tag},
};
use std::collections::HashMap;
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
    pin_serial: u64,
    pause_checks: usize,
}
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
            pin_serial: 0,
            pause_checks: 0,
        };
        while s.total_rows < usize::from(rows) {
            s.grow()?;
        }
        let pin = s
            .get_top_left(Tag::Screen)
            .ok_or(Error::InvalidCoordinate)?;
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
        self.nodes.try_reserve(1).map_err(|_| Error::OutOfMemory)?;
        let mut data = if let Some(pos) = self.page_pool.iter().position(|p| p.capacity == cap) {
            let mut p = self.page_pool.swap_remove(pos);
            p.reinit()?;
            p
        } else {
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
    /// Current viewport's absolute row offset.
    pub fn viewport_row_offset(&self) -> usize {
        match self.viewport {
            Viewport::Active => self.active_offset(),
            Viewport::Top => 0,
            Viewport::Pin => self
                .viewport_pin
                .and_then(|h| self.tracked_pin(h))
                .and_then(|p| self.screen_row(p))
                .unwrap_or(0),
        }
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
    /// Map a tagged coordinate to an untracked pin.
    pub fn pin(&self, pt: Point) -> Option<Pin> {
        let (start, end) = self.bounds(pt.tag());
        let c = pt.coord();
        let y = start.checked_add(c.y as usize)?;
        if y >= end {
            return None;
        }
        self.pin_at(y, c.x)
    }
    /// Convert a pin to a coordinate in a reference frame, if inside it.
    pub fn point_from_pin(&self, tag: Tag, p: Pin) -> Option<Point> {
        let y = self.screen_row(p)?;
        let (start, end) = self.bounds(tag);
        if y < start || y >= end {
            return None;
        }
        Some(Point::new(
            tag,
            Coordinate {
                x: p.x,
                y: (y - start) as u32,
            },
        ))
    }
    /// Top-left coordinate in a reference frame, if the region exists.
    pub fn get_top_left(&self, tag: Tag) -> Option<Pin> {
        self.pin(Point::new(tag, Coordinate::default()))
    }
    /// Bottom-right coordinate in a reference frame, if the region exists.
    pub fn get_bottom_right(&self, tag: Tag) -> Option<Pin> {
        let (start, end) = self.bounds(tag);
        if start == end {
            return None;
        }
        self.pin_at(end - 1, self.cols - 1)
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
    /// Update a tracked location to another valid cell.
    pub fn set_tracked_pin(&mut self, h: TrackedPin, p: Pin) -> Result<(), Error> {
        if !self.pin_is_valid(p) {
            return Err(Error::InvalidCoordinate);
        }
        let slot = self
            .pins
            .get_mut(h.index)
            .filter(|s| s.generation == h.generation && s.pin.is_some())
            .ok_or(Error::InvalidId)?;
        slot.pin = Some(p);
        Ok(())
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
                let viewport = self.viewport_row_offset();
                let n = self.detach(first).ok_or(Error::InvalidId)?;
                self.total_rows -= removed;
                self.page_serial_min = n.serial.wrapping_add(1);
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
                if self.viewport == Viewport::Pin && viewport < removed {
                    self.viewport = Viewport::Top;
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
        let row = match behavior {
            Scroll::Active => {
                self.viewport = Viewport::Active;
                return;
            }
            Scroll::Top => {
                self.viewport = Viewport::Top;
                return;
            }
            Scroll::Row(y) => y,
            Scroll::DeltaRow(d) => self.viewport_row_offset().saturating_add_signed(d),
            Scroll::Pin(p) => {
                let Some(y) = self.screen_row(p) else {
                    return;
                };
                y
            }
            Scroll::DeltaPrompt(d) => {
                if d == 0 {
                    return;
                }
                let current = self.viewport_row_offset();
                let mut prompts: Vec<_> = self
                    .prompt_iterator(
                        Direction::RightDown,
                        Point::Screen(Coordinate::default()),
                        None,
                    )
                    .filter_map(|p| self.screen_row(p))
                    .filter(|y| if d < 0 { *y < current } else { *y > current })
                    .collect();
                if d < 0 {
                    prompts.reverse();
                }
                let Some(&y) = prompts.get(
                    d.unsigned_abs()
                        .saturating_sub(1)
                        .min(prompts.len().saturating_sub(1)),
                ) else {
                    return;
                };
                y
            }
        };
        if row >= self.active_offset() {
            self.viewport = Viewport::Active;
        } else if row == 0 {
            self.viewport = Viewport::Top;
        } else if let (Some(p), Some(h)) = (self.pin_at(row, 0), self.viewport_pin) {
            self.pins[h.index].pin = Some(p);
            self.viewport = Viewport::Pin;
        }
    }
    fn fixup_viewport(&mut self) {
        if self.viewport == Viewport::Pin && self.viewport_row_offset() >= self.active_offset() {
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
        let mut copy = Page::init(page.capacity)?;
        copy.size = page.size;
        copy.size.rows -= p.y;
        copy.clone_from(page, usize::from(p.y), usize::from(page.size.rows))?;
        copy.dirty = page.dirty;
        let id = self.create_node(copy.capacity)?;
        self.node_mut(id).ok_or(Error::InvalidId)?.data = copy;
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
    fn replace_page(&mut self, old: NodeId, page: Page) -> Result<NodeId, Error> {
        let prev = self.node(old).ok_or(Error::InvalidId)?.prev;
        let id = self.create_node(page.capacity)?;
        self.node_mut(id).ok_or(Error::InvalidId)?.data = page;
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
        let mut page = Page::init(cap)?;
        page.size = old.size;
        page.clone_from(old, 0, usize::from(old.size.rows))?;
        page.dirty = old.dirty;
        self.replace_page(id, page)
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
        let mut page = Page::init(cap)?;
        page.size = old.size;
        if page.clone_from(old, 0, usize::from(old.size.rows)).is_err() {
            return Ok(None);
        }
        page.dirty = old.dirty;
        self.replace_page(id, page).map(Some)
    }
    fn erase_range(&mut self, start: usize, end: usize) -> Result<(), Error> {
        if start > end || end > self.total_rows {
            return Err(Error::InvalidCoordinate);
        }
        if start == end {
            return Ok(());
        }
        let coords: Vec<_> = self
            .tracked_pins()
            .filter_map(|(h, p)| self.screen_row(p).map(|y| (h, p, y)))
            .collect();
        let mut id = self.first;
        let mut offset = 0;
        while let Some(i) = id {
            let n = self.node(i).ok_or(Error::InvalidId)?;
            id = n.next;
            let count = usize::from(n.data.size.rows);
            let a = start.saturating_sub(offset).min(count);
            let b = end.saturating_sub(offset).min(count);
            offset += count;
            if a == b {
                continue;
            }
            if a == 0 && b == count && self.first != self.last {
                let n = self.detach(i).ok_or(Error::InvalidId)?;
                self.recycle(n);
            } else {
                let p = &mut self.node_mut(i).ok_or(Error::InvalidId)?.data;
                for y in a..b {
                    p.clear_cells(y, 0, usize::from(p.size.cols), false)?;
                }
                p.rows[a..count].rotate_left(b - a);
                p.size.rows -= (b - a) as u16;
                for r in &mut p.rows[a..usize::from(p.size.rows)] {
                    r.set_dirty(true);
                }
            }
        }
        self.total_rows -= end - start;
        if self.total_rows == 0 {
            self.grow()?;
        }
        for (h, old, y) in coords {
            let erased = y >= start && y < end;
            let mapped_y = if y < start {
                y
            } else if y >= end {
                y - (end - start)
            } else {
                start.min(self.total_rows - 1)
            };
            let mut p = self
                .pin_at(mapped_y, if erased { 0 } else { old.x })
                .ok_or(Error::InvalidCoordinate)?;
            p.garbage = old.garbage;
            self.pins[h.index].pin = Some(p);
        }
        self.fixup_viewport();
        Ok(())
    }
    /// Remove an inclusive row range, regrowing erased active rows.
    pub fn erase_rows(&mut self, start: Point, end: Option<Point>) -> Result<(), Error> {
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
        self.erase_range(a, b)?;
        while self.total_rows < usize::from(self.rows) {
            self.grow()?;
        }
        Ok(())
    }
    /// Remove one row, shifting following rows without changing page dimensions.
    pub fn erase_row(&mut self, pt: Point) -> Result<(), Error> {
        let p = self.pin(pt).ok_or(Error::InvalidCoordinate)?;
        let y = self.screen_row(p).ok_or(Error::InvalidCoordinate)?;
        let stationary: Vec<_> = self
            .tracked_pins()
            .filter(|(_, q)| q.node == p.node && q.y == p.y)
            .collect();
        self.erase_row_bounded(pt, self.total_rows - y - 1)?;
        for (h, p) in stationary {
            self.pins[h.index].pin = Some(p);
        }
        Ok(())
    }
    /// Remove a row and shift at most `limit` following rows, blanking the last.
    pub fn erase_row_bounded(&mut self, pt: Point, limit: usize) -> Result<(), Error> {
        let first = self.pin(pt).ok_or(Error::InvalidCoordinate)?;
        let start = self.screen_row(first).ok_or(Error::InvalidCoordinate)?;
        let end = start.saturating_add(limit).min(self.total_rows - 1);
        let old_pins: Vec<_> = self
            .tracked_pins()
            .filter_map(|(h, p)| self.screen_row(p).map(|y| (h, p, y)))
            .collect();
        let mut current = first;
        loop {
            let node = self.node(current.node).ok_or(Error::InvalidId)?;
            let offset = self
                .screen_row(Pin { y: 0, ..current })
                .ok_or(Error::InvalidCoordinate)?;
            let stop = (end - offset + 1).min(usize::from(node.data.size.rows));
            let next = node.next;
            let has_next = offset + stop <= end;
            // Rotation moves only compact row metadata and keeps cell indices stable.
            let page = &mut self.node_mut(current.node).ok_or(Error::InvalidId)?.data;
            page.rows[usize::from(current.y)..stop].rotate_left(1);
            page.dirty = true;
            if !has_next {
                page.clear_cells(stop - 1, 0, usize::from(page.size.cols), false)?;
                break;
            }
            let next = next.ok_or(Error::InvalidId)?;
            let source = &self.node(next).ok_or(Error::InvalidId)?.data;
            let row = source.rows[0];
            let values: Vec<_> = (row.cells()..row.cells() + usize::from(source.size.cols))
                .map(|i| source.export_cell(i))
                .collect();
            let dest = &mut self.node_mut(current.node).ok_or(Error::InvalidId)?.data;
            dest.clear_cells(stop - 1, 0, usize::from(dest.size.cols), false)?;
            for (x, value) in values.iter().enumerate() {
                dest.import_cell(x, stop - 1, value)?;
            }
            let cells = dest.rows[stop - 1].cells();
            dest.rows[stop - 1] = row;
            dest.rows[stop - 1].set_cells(cells);
            current = Pin {
                node: next,
                y: 0,
                x: 0,
                garbage: false,
            };
        }
        for (h, old, y) in old_pins {
            if y >= start && y <= end {
                let mut p = self
                    .pin_at(y.saturating_sub(1), if y == 0 { 0 } else { old.x })
                    .ok_or(Error::InvalidCoordinate)?;
                p.garbage = old.garbage;
                self.pins[h.index].pin = Some(p);
            }
        }
        self.fixup_viewport();
        Ok(())
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
            let data: Vec<_> = {
                let page = &self.node(src.node).ok_or(Error::InvalidId)?.data;
                (left..=right)
                    .map(|x| {
                        page.export_cell(page.rows[usize::from(src.y)].cells() + usize::from(x))
                    })
                    .collect()
            };
            for (x, v) in data.iter().enumerate() {
                self.node_mut(dst.node)
                    .ok_or(Error::InvalidId)?
                    .data
                    .import_cell(usize::from(left) + x, usize::from(dst.y), v)?;
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
        let y = list.screen_row(self)?;
        if n <= y {
            Some(Overflow::Offset(self.up(list, n)?))
        } else {
            Some(Overflow::Overflow {
                end: list.pin_at(0, self.x)?,
                remaining: n - y,
            })
        }
    }
    /// Move downward, returning the edge and unconsumed distance on overflow.
    pub fn down_overflow(self, list: &PageList, n: usize) -> Option<Overflow> {
        let y = list.screen_row(self)?;
        let available = list.total_rows - y - 1;
        if n <= available {
            Some(Overflow::Offset(self.down(list, n)?))
        } else {
            Some(Overflow::Overflow {
                end: list.pin_at(list.total_rows - 1, self.x)?,
                remaining: n - available,
            })
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
        match (list.screen_row(self), list.screen_row(other)) {
            (Some(a), Some(b)) => (a, self.x) < (b, other.x),
            _ => false,
        }
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

#[derive(Clone)]
struct SavedRow {
    row: Row,
    cells: Vec<OwnedCell>,
}
impl PageList {
    fn snapshot(&self) -> Vec<SavedRow> {
        self.row_iterator(
            Direction::RightDown,
            Point::Screen(Coordinate::default()),
            None,
        )
        .filter_map(|p| {
            let page = &self.node(p.node)?.data;
            let mut row = *page.get_row(usize::from(p.y))?;
            row.set_dirty(row.dirty() || page.dirty);
            let cells = (row.cells()..row.cells() + usize::from(page.size.cols))
                .map(|i| page.export_cell(i))
                .collect();
            Some(SavedRow { row, cells })
        })
        .collect()
    }
    fn build_pages(cols: u16, rows: &[SavedRow], mark_dirty: bool) -> Result<Vec<Page>, Error> {
        let mut pages = Vec::new();
        let mut offset = 0;
        while offset < rows.len() {
            let mut cap = initial_capacity(cols);
            let count = usize::from(cap.rows).min(rows.len() - offset);
            // Count required values before allocating, so cloning is monotonic.
            let mut styles = std::collections::HashSet::new();
            let mut links = std::collections::HashSet::new();
            let mut gb = 0usize;
            let mut sb = 0usize;
            let mut hc = 0usize;
            for r in &rows[offset..offset + count] {
                for c in &r.cells {
                    if let Some(s) = c.style {
                        styles.insert(s);
                    }
                    if let Some(g) = &c.grapheme {
                        gb += (g.len() * 4).next_multiple_of(16);
                    }
                    if let Some(h) = &c.hyperlink {
                        hc += 1;
                        if links.insert(h.clone()) {
                            sb += h.string_bytes();
                        }
                    }
                }
            }
            let sc = crate::style::Set::capacity_for_count(styles.len());
            let lc = crate::hyperlink::Set::capacity_for_count(links.len()).max(hc.div_ceil(16));
            cap.styles = cap
                .styles
                .max(u16::try_from(sc).map_err(|_| Error::OutOfSpace)?);
            cap.hyperlink_bytes = cap
                .hyperlink_bytes
                .max(u16::try_from(lc * 48).map_err(|_| Error::OutOfSpace)?);
            cap.grapheme_bytes = cap
                .grapheme_bytes
                .max(u32::try_from(gb).map_err(|_| Error::OutOfSpace)?);
            cap.string_bytes = cap
                .string_bytes
                .max(u32::try_from(sb).map_err(|_| Error::OutOfSpace)?);
            let mut page = Page::init(cap)?;
            page.size.rows = count as u16;
            for (y, row) in rows[offset..offset + count].iter().enumerate() {
                for (x, c) in row.cells.iter().take(usize::from(cols)).enumerate() {
                    page.import_cell(x, y, c)?;
                }
                let start = page.rows[y].cells();
                page.rows[y] = row.row;
                page.rows[y].set_cells(start);
                page.rows[y].set_dirty(mark_dirty || row.row.dirty());
                page.update_row_grapheme_flag(y);
                page.update_row_styled_flag(y);
                page.update_row_hyperlink_flag(y);
                page.rows[y].set_kitty_virtual_placeholder(
                    row.cells.iter().any(|c| c.cell.codepoint() == 0x10eeee),
                );
            }
            pages.push(page);
            offset += count;
        }
        Ok(pages)
    }
    fn install_pages(
        &mut self,
        pages: Vec<Page>,
        cols: u16,
        coords: &[(TrackedPin, usize, u16, bool)],
    ) -> Result<(), Error> {
        let old: Vec<_> = self
            .page_iterator(
                Direction::RightDown,
                Point::Screen(Coordinate::default()),
                None,
            )
            .map(|c| c.node)
            .collect();
        let mut added = Vec::new();
        for page in pages {
            let id = match self.create_node(page.capacity) {
                Ok(id) => id,
                Err(e) => {
                    for id in added {
                        if let Some(n) = self.detach(id) {
                            self.recycle(n);
                        }
                    }
                    return Err(e);
                }
            };
            self.node_mut(id).ok_or(Error::InvalidId)?.data = page;
            self.insert_after(self.last, id);
            added.push(id);
        }
        for id in old {
            if let Some(n) = self.detach(id) {
                self.recycle(n);
            }
        }
        self.cols = cols;
        self.total_rows = self
            .nodes
            .iter()
            .flatten()
            .map(|n| usize::from(n.data.size.rows))
            .sum();
        for &(h, y, x, garbage) in coords {
            if let Some(mut p) =
                self.pin_at(y.min(self.total_rows.saturating_sub(1)), x.min(cols - 1))
            {
                p.garbage = garbage;
                self.pins[h.index].pin = Some(p);
            }
        }
        self.fixup_viewport();
        Ok(())
    }
    fn trim_blank_rows(&mut self, limit: usize) -> Result<(), Error> {
        for _ in 0..limit {
            if self.total_rows <= 1 {
                break;
            }
            let last = self
                .pin_at(self.total_rows - 1, 0)
                .ok_or(Error::InvalidCoordinate)?;
            let page = &self.node(last.node).ok_or(Error::InvalidId)?.data;
            if page
                .get_cells(usize::from(last.y))
                .is_some_and(|c| c.iter().any(|c| !c.is_empty()))
                || self
                    .tracked_pins()
                    .any(|(_, p)| p.node == last.node && p.y == last.y)
            {
                break;
            }
            self.erase_range(self.total_rows - 1, self.total_rows)?;
        }
        Ok(())
    }
    /// Resize the active area and optionally reflow wrapped logical lines.
    pub fn resize(&mut self, opts: Resize) -> Result<(), Error> {
        let cols = opts.cols.unwrap_or(self.cols);
        let rows = opts.rows.unwrap_or(self.rows);
        if cols == 0 || rows == 0 {
            return Err(Error::InvalidDimensions);
        }
        let cursor = opts.cursor.and_then(|c| {
            self.pin(Point::Active(Coordinate {
                x: c.x,
                y: u32::from(c.y),
            }))
        });
        let cursor_handle = cursor.map(|p| self.track_pin(p)).transpose()?;
        let result = self.resize_inner(opts, cols, rows, cursor_handle);
        if let Some(h) = cursor_handle {
            self.untrack_pin(h);
        }
        result
    }
    fn resize_inner(
        &mut self,
        opts: Resize,
        cols: u16,
        rows: u16,
        cursor_handle: Option<TrackedPin>,
    ) -> Result<(), Error> {
        let old_rows = self.rows;
        self.min_max_size = min_max_size(cols, rows);
        if cols != self.cols {
            if !opts.reflow && cols < self.cols {
                for n in self.nodes.iter_mut().flatten() {
                    let p = &mut n.data;
                    for y in 0..usize::from(p.size.rows) {
                        p.clear_cells(y, usize::from(cols), usize::from(p.size.cols), false)?;
                    }
                    p.size.cols = cols;
                }
                self.cols = cols;
                for slot in &mut self.pins {
                    if let Some(p) = &mut slot.pin {
                        p.x = p.x.min(cols - 1);
                    }
                }
            } else {
                let old = self.snapshot();
                let tracked: Vec<_> = self
                    .tracked_pins()
                    .filter_map(|(h, p)| self.screen_row(p).map(|y| (h, y, p.x, p.garbage)))
                    .collect();
                let mut remapped = tracked.clone();
                let mut output = Vec::<SavedRow>::new();
                if opts.reflow {
                    let mut current = SavedRow {
                        row: Row::default(),
                        cells: Vec::new(),
                    };
                    for (y, row) in old.iter().enumerate() {
                        if current.cells.is_empty() {
                            current.row = row.row;
                            current.row.set_wrap(false);
                            current
                                .row
                                .set_wrap_continuation(y > 0 && old[y - 1].row.wrap());
                        }
                        let mut len = if row.row.wrap() {
                            row.cells.len()
                        } else {
                            row.cells
                                .iter()
                                .rposition(|c| !c.cell.is_empty())
                                .map_or(0, |x| x + 1)
                        };
                        if let Some(h) = cursor_handle
                            && let Some((_, _, x, _)) =
                                tracked.iter().find(|(th, ty, _, _)| *th == h && *ty == y)
                        {
                            len = len.max(usize::from(*x) + 1);
                        }
                        let mut x = 0;
                        while x < len {
                            let c = &row.cells[x];
                            if c.cell.wide() == Wide::SpacerHead {
                                x += 1;
                                continue;
                            }
                            let wide = c.cell.wide() == Wide::Wide;
                            let width = if wide && cols > 1 { 2 } else { 1 };
                            if current.cells.len() + width > usize::from(cols) {
                                if wide && current.cells.len() < usize::from(cols) {
                                    let mut spacer = Cell::default();
                                    spacer.set_wide(Wide::SpacerHead);
                                    current.cells.push(OwnedCell {
                                        cell: spacer,
                                        style: None,
                                        grapheme: None,
                                        hyperlink: None,
                                    });
                                }
                                current.row.set_wrap(true);
                                output.push(current);
                                let mut meta = row.row;
                                meta.set_wrap(false);
                                meta.set_wrap_continuation(true);
                                current = SavedRow {
                                    row: meta,
                                    cells: Vec::new(),
                                };
                            }
                            let newx = current.cells.len() as u16;
                            let newy = output.len();
                            for (index, (_, ty, tx, _)) in tracked.iter().enumerate() {
                                if *ty == y && usize::from(*tx) >= x && usize::from(*tx) < x + width
                                {
                                    remapped[index].1 = newy;
                                    remapped[index].2 = newx + (*tx - x as u16);
                                }
                            }
                            let mut value = c.clone();
                            if wide && cols == 1 {
                                value.cell = Cell::default();
                                value.style = None;
                                value.grapheme = None;
                                value.hyperlink = None;
                            }
                            current.cells.push(value);
                            if wide && cols > 1 {
                                let tail = row
                                    .cells
                                    .get(x + 1)
                                    .filter(|v| v.cell.wide() == Wide::SpacerTail)
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        let mut cell = Cell::default();
                                        cell.set_wide(Wide::SpacerTail);
                                        OwnedCell {
                                            cell,
                                            style: None,
                                            grapheme: None,
                                            hyperlink: None,
                                        }
                                    });
                                current.cells.push(tail);
                            }
                            x += if wide { 2 } else { 1 };
                        }
                        for (index, (_, ty, tx, _)) in tracked.iter().enumerate() {
                            if *ty == y && usize::from(*tx) >= len {
                                remapped[index].1 = output.len();
                                remapped[index].2 = (current.cells.len() + usize::from(*tx) - len)
                                    .min(usize::from(cols - 1))
                                    as u16;
                            }
                        }
                        if !row.row.wrap() {
                            current.row.set_wrap(false);
                            output.push(current);
                            current = SavedRow {
                                row: Row::default(),
                                cells: Vec::new(),
                            };
                        }
                    }
                    if !current.cells.is_empty() {
                        output.push(current);
                    }
                } else {
                    for row in &old {
                        let mut r = row.clone();
                        r.cells.truncate(usize::from(cols));
                        for c in &mut r.cells {
                            if c.cell.wide() == Wide::SpacerHead {
                                c.cell.set_wide(Wide::Narrow);
                                c.cell.set_codepoint(0);
                                c.grapheme = None;
                            }
                        }
                        output.push(r);
                    }
                }
                if opts.reflow {
                    let mut minimum = usize::from(rows);
                    for (h, y, _, _) in &remapped {
                        minimum = minimum.max(y + 1);
                        if Some(*h) == cursor_handle {
                            minimum = minimum.max(
                                y + 1
                                    + opts.cursor.map_or(0, |c| {
                                        usize::from(old_rows.saturating_sub(c.y + 1))
                                    }),
                            );
                        }
                    }
                    while output.len() > minimum
                        && output.last().is_some_and(|r| {
                            r.row.semantic_prompt() == SemanticPrompt::None
                                && r.cells.iter().all(|c| c.cell.is_empty())
                        })
                    {
                        output.pop();
                    }
                }
                while output.len() < usize::from(rows) {
                    output.push(SavedRow {
                        row: Row::default(),
                        cells: Vec::new(),
                    });
                }
                let pages = Self::build_pages(cols, &output, true)?;
                self.install_pages(pages, cols, &remapped)?;
            }
        }
        if rows < old_rows {
            self.trim_blank_rows(usize::from(old_rows - rows))?;
        }
        self.rows = rows;
        if rows > old_rows && opts.cursor.is_some_and(|c| c.y < old_rows - 1) {
            for _ in 0..rows - old_rows {
                self.grow()?;
            }
        }
        while self.total_rows < usize::from(rows) {
            self.grow()?;
        }
        self.fixup_viewport();
        Ok(())
    }
    /// Clone an inclusive cell range into an independent list and remap tracked pins.
    pub fn clone_range(
        &self,
        start: Point,
        end: Option<Point>,
    ) -> Result<(Self, HashMap<TrackedPin, TrackedPin>), Error> {
        let (a, b) = self.iter_bounds(start, end);
        let a = Pin {
            x: 0,
            ..a.ok_or(Error::InvalidCoordinate)?
        };
        let b = Pin {
            x: self.cols - 1,
            ..b.ok_or(Error::InvalidCoordinate)?
        };
        let ay = self.screen_row(a).ok_or(Error::InvalidCoordinate)?;
        let by = self.screen_row(b).ok_or(Error::InvalidCoordinate)?;
        let mut saved = self.snapshot()[ay..=by].to_vec();
        let blank = || OwnedCell {
            cell: Cell::default(),
            style: None,
            grapheme: None,
            hyperlink: None,
        };
        for c in &mut saved[0].cells[..usize::from(a.x)] {
            *c = blank();
        }
        if let Some(last) = saved.last_mut() {
            for c in &mut last.cells[usize::from(b.x) + 1..] {
                *c = blank();
            }
        }
        while saved.len() < usize::from(self.rows) {
            saved.push(SavedRow {
                row: Row::default(),
                cells: Vec::new(),
            });
        }
        let mut result = Self::init(self.cols, self.rows, Some(self.explicit_max_size))?;
        let pages = Self::build_pages(self.cols, &saved, false)?;
        let vh = result.viewport_pin.ok_or(Error::InvalidId)?;
        result.install_pages(pages, self.cols, &[(vh, 0, 0, false)])?;
        let mut remap = HashMap::new();
        for (h, p) in self.tracked_pins() {
            if Some(h) == self.viewport_pin {
                continue;
            }
            if p.is_between(self, a, b) {
                let y = self.screen_row(p).ok_or(Error::InvalidCoordinate)?;
                let mut q = result.pin_at(y - ay, p.x).ok_or(Error::InvalidCoordinate)?;
                q.garbage = p.garbage;
                remap.insert(h, result.track_pin(q)?);
            }
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
    Ok(v.saturating_mul(2).max(1))
}

fn double32(v: u32) -> Result<u32, Error> {
    if v == u32::MAX {
        return Err(Error::OutOfSpace);
    }
    Ok(v.saturating_mul(2).max(1))
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
    fn fill(s: &mut PageList) {
        for y in 0..s.total_rows {
            for x in 0..s.cols {
                *s.get_cell_mut(screen(x, y as u32)).unwrap() = Cell::init(u32::from(b'A'));
            }
        }
    }
    fn write_rows(s: &mut PageList) {
        for y in 0..s.total_rows {
            *s.get_cell_mut(screen(0, y as u32)).unwrap() = Cell::init(y as u32);
        }
    }
    fn wraps(s: &mut PageList) {
        for y in 0..s.total_rows {
            let p = s.pin(screen(0, y as u32)).unwrap();
            let r = s
                .node_mut(p.node)
                .unwrap()
                .data
                .get_row_mut(usize::from(p.y))
                .unwrap();
            r.set_wrap(y % 2 == 0);
            r.set_wrap_continuation(y % 2 != 0);
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
        let s = PageList::init(80, 24, None).unwrap();
        assert_eq!(s.viewport, Viewport::Active);
        assert_eq!(s.total_rows, 24);
        assert_eq!(s.get_top_left(Tag::Active).unwrap().node, s.first.unwrap());
        assert_eq!(
            s.scrollbar(),
            Scrollbar {
                total: 24,
                offset: 0,
                len: 24
            }
        );
        assert_eq!(
            s.tracked_pin(s.viewport_pin.unwrap()).unwrap().node,
            s.first.unwrap()
        );
        finish(&s);
    }
    #[test]
    fn page_list_init_rows_across_two_pages() {
        let mut cols = 50;
        while initial_capacity(cols).rows >= 100 {
            cols += 50;
        }
        let s = PageList::init(cols, 100, None).unwrap();
        assert_eq!(s.total_pages(), 2);
        assert_eq!(s.total_rows, 100);
        assert_eq!(
            s.scrollbar(),
            Scrollbar {
                total: 100,
                offset: 0,
                len: 100
            }
        );
        finish(&s);
    }
    #[test]
    fn page_list_init_more_than_max_cols() {
        let cols = page::STD_CAPACITY.max_cols().unwrap() + 1;
        let s = PageList::init(cols, 24, None).unwrap();
        assert_eq!(s.cols, cols);
        assert_eq!(s.total_rows, 24);
        assert!(s.page_size > std_size());
        finish(&s);
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
        grow(&mut s, 10);
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.get_top_left(Tag::Active).unwrap()),
            Some(screen(0, 10))
        );
        assert_eq!(s.get_bottom_right(Tag::Active).unwrap().y, 33);
        finish(&s);
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
        grow(&mut s, 10);
        assert_eq!(s.viewport_row_offset(), 10);
        s.scroll(Scroll::Top);
        assert_eq!(s.viewport_row_offset(), 0);
        grow(&mut s, 10);
        assert_eq!(
            s.scrollbar(),
            Scrollbar {
                total: 44,
                offset: 0,
                len: 24
            }
        );
        s.scroll(Scroll::Active);
        assert_eq!(s.viewport_row_offset(), 20);
    }
    #[test]
    fn page_list_scroll_delta_row_back() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 10);
        s.scroll(Scroll::DeltaRow(-5));
        assert_eq!(s.viewport_row_offset(), 5);
        assert_eq!(s.viewport, Viewport::Pin);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_delta_row_back_overflow() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 10);
        s.scroll(Scroll::DeltaRow(-100));
        assert_eq!(s.viewport_row_offset(), 0);
        assert_eq!(s.viewport, Viewport::Top);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_delta_row_forward() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 10);
        s.scroll(Scroll::Top);
        s.scroll(Scroll::DeltaRow(5));
        assert_eq!(s.viewport_row_offset(), 5);
        assert_eq!(s.viewport, Viewport::Pin);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_delta_row_forward_into_active() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 10);
        s.scroll(Scroll::Top);
        s.scroll(Scroll::DeltaRow(100));
        assert_eq!(s.viewport_row_offset(), 10);
        assert_eq!(s.viewport, Viewport::Active);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_pin() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 10);
        s.scroll(Scroll::Pin(s.pin(screen(5, 3)).unwrap()));
        assert_eq!(s.viewport_row_offset(), 3);
        assert_eq!(s.viewport, Viewport::Pin);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_pin_in_active() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 10);
        s.scroll(Scroll::Pin(s.pin(active(5, 3)).unwrap()));
        assert_eq!(s.viewport_row_offset(), 10);
        assert_eq!(s.viewport, Viewport::Active);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_pin_at_top() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 10);
        s.scroll(Scroll::Pin(s.pin(screen(0, 0)).unwrap()));
        assert_eq!(s.viewport_row_offset(), 0);
        assert_eq!(s.viewport, Viewport::Top);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_row_0() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 30);
        s.scroll(Scroll::Row(0));
        assert_eq!(s.viewport_row_offset(), 0);
        assert_eq!(s.viewport, Viewport::Top);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_row_in_scrollback() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 30);
        s.scroll(Scroll::Row(10));
        assert_eq!(s.viewport_row_offset(), 10);
        assert_eq!(s.viewport, Viewport::Pin);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_row_in_middle() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 50);
        s.scroll(Scroll::Row(25));
        assert_eq!(s.viewport_row_offset(), 25);
        assert_eq!(s.viewport, Viewport::Pin);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_row_at_active_boundary() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 30);
        s.scroll(Scroll::Row(30));
        assert_eq!(s.viewport_row_offset(), 30);
        assert_eq!(s.viewport, Viewport::Active);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_row_beyond_active() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 30);
        s.scroll(Scroll::Row(999));
        assert_eq!(s.viewport_row_offset(), 30);
        assert_eq!(s.viewport, Viewport::Active);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_row_without_scrollback() {
        let mut s = PageList::init(80, 24, None).unwrap();
        s.scroll(Scroll::Row(10));
        assert_eq!(s.viewport_row_offset(), 0);
        assert_eq!(s.viewport, Viewport::Active);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_row_then_delta() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 30);
        s.scroll(Scroll::Row(10));
        s.scroll(Scroll::DeltaRow(5));
        assert_eq!(s.viewport_row_offset(), 15);
        assert_eq!(s.viewport, Viewport::Pin);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_row_with_cache_fast_path_down() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 60);
        s.scroll(Scroll::Row(10));
        assert_eq!(s.scrollbar().offset, 10);
        s.scroll(Scroll::Row(20));
        assert_eq!(s.viewport_row_offset(), 20);
        assert_eq!(s.viewport, Viewport::Pin);
        finish(&s);
    }
    #[test]
    fn page_list_scroll_to_row_with_cache_fast_path_up() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 60);
        s.scroll(Scroll::Row(20));
        assert_eq!(s.scrollbar().offset, 20);
        s.scroll(Scroll::Row(10));
        assert_eq!(s.viewport_row_offset(), 10);
        assert_eq!(s.viewport, Viewport::Pin);
        finish(&s);
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
        let cap = first_cap(&s);
        grow(&mut s, cap - 24);
        let new = s.grow().unwrap().unwrap();
        assert_eq!(s.last, Some(new));
        assert_eq!(s.node(old).unwrap().next, Some(new));
        assert_eq!(s.pin(active(0, 23)).unwrap().node, new);
        finish(&s);
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
        assert_eq!(s.scrollbar().offset, y);
        let new = s.grow().unwrap().unwrap();
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
        assert_eq!(s.scrollbar().offset, y);
        let new = s.grow().unwrap().unwrap();
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
    fn page_list_increase_capacity_to_increase_styles() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let old = s.first.unwrap();
        let before = s.node(old).unwrap().data.capacity.styles;
        let id = s
            .increase_capacity(old, Some(IncreaseCapacity::Styles))
            .unwrap();
        assert_eq!(s.first, Some(id));
        assert_eq!(s.node(id).unwrap().data.capacity.styles, before * 2);
        assert_eq!(s.total_rows, 24);
        finish(&s);
    }
    #[test]
    fn page_list_increase_capacity_to_increase_graphemes() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let old = s.first.unwrap();
        let before = s.node(old).unwrap().data.capacity.grapheme_bytes;
        let id = s
            .increase_capacity(old, Some(IncreaseCapacity::GraphemeBytes))
            .unwrap();
        assert_eq!(s.first, Some(id));
        assert_eq!(s.node(id).unwrap().data.capacity.grapheme_bytes, before * 2);
        assert_eq!(s.total_rows, 24);
        finish(&s);
    }
    #[test]
    fn page_list_increase_capacity_to_increase_hyperlinks() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let old = s.first.unwrap();
        let before = s.node(old).unwrap().data.capacity.hyperlink_bytes;
        let id = s
            .increase_capacity(old, Some(IncreaseCapacity::HyperlinkBytes))
            .unwrap();
        assert_eq!(s.first, Some(id));
        assert_eq!(
            s.node(id).unwrap().data.capacity.hyperlink_bytes,
            before * 2
        );
        assert_eq!(s.total_rows, 24);
        finish(&s);
    }
    #[test]
    fn page_list_increase_capacity_to_increase_string_bytes() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let old = s.first.unwrap();
        let before = s.node(old).unwrap().data.capacity.string_bytes;
        let id = s
            .increase_capacity(old, Some(IncreaseCapacity::StringBytes))
            .unwrap();
        assert_eq!(s.first, Some(id));
        assert_eq!(s.node(id).unwrap().data.capacity.string_bytes, before * 2);
        assert_eq!(s.total_rows, 24);
        finish(&s);
    }
    #[test]
    fn page_list_increase_capacity_tracked_pins() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let h = s.track_pin(s.pin(active(3, 12)).unwrap()).unwrap();
        let id = s
            .increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::Styles))
            .unwrap();
        assert_eq!(s.tracked_pin(h).unwrap().node, id);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(3, 12))
        );
        finish(&s);
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
        let mut s = PageList::init(80, 24, None).unwrap();
        s.node_mut(s.first.unwrap()).unwrap().data.dirty = true;
        let id = s
            .increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::Styles))
            .unwrap();
        assert!(s.node(id).unwrap().data.dirty);
        finish(&s);
    }
    #[test]
    fn page_list_page_iterator_single_page() {
        let s = PageList::init(80, 24, None).unwrap();
        let chunks: Vec<_> = s
            .page_iterator(Direction::RightDown, active(0, 0), None)
            .collect();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0],
            Chunk {
                node: s.first.unwrap(),
                start: 0,
                end: 24
            }
        );
        assert!(chunks[0].full_page(&s));
    }
    #[test]
    fn page_list_page_iterator_two_pages() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let cap = first_cap(&s);
        grow(&mut s, cap - 12);
        let chunks: Vec<_> = s
            .page_iterator(Direction::RightDown, active(0, 0), None)
            .collect();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].start, cap as u16 - 12);
        assert_eq!(chunks[0].end, cap as u16);
        assert_eq!(chunks[1].start, 0);
        assert_eq!(chunks[1].end, 12);
    }
    #[test]
    fn page_list_cell_iterator() {
        let s = PageList::init(5, 3, None).unwrap();
        let cells: Vec<_> = s
            .cell_iterator(Direction::RightDown, screen(1, 0), Some(screen(3, 2)))
            .collect();
        assert_eq!(cells.len(), 13);
        assert_eq!(cells.first().unwrap().x, 1);
        assert_eq!(cells.last().unwrap().x, 3);
    }
    #[test]
    fn page_list_page_iterator_reverse_single_page() {
        let s = PageList::init(80, 24, None).unwrap();
        let chunks: Vec<_> = s
            .page_iterator(Direction::LeftUp, active(0, 0), None)
            .collect();
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0],
            Chunk {
                node: s.first.unwrap(),
                start: 0,
                end: 24
            }
        );
        assert!(chunks[0].full_page(&s));
    }
    #[test]
    fn page_list_page_iterator_reverse_two_pages() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let cap = first_cap(&s);
        grow(&mut s, cap - 12);
        let mut chunks: Vec<_> = s
            .page_iterator(Direction::LeftUp, active(0, 0), None)
            .collect();
        chunks.reverse();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].start, cap as u16 - 12);
        assert_eq!(chunks[0].end, cap as u16);
        assert_eq!(chunks[1].start, 0);
        assert_eq!(chunks[1].end, 12);
    }
    #[test]
    fn page_list_cell_iterator_reverse() {
        let s = PageList::init(5, 3, None).unwrap();
        let cells: Vec<_> = s
            .cell_iterator(Direction::LeftUp, screen(1, 0), Some(screen(3, 2)))
            .collect();
        assert_eq!(cells.len(), 13);
        assert_eq!(cells.first().unwrap().x, 3);
        assert_eq!(cells.last().unwrap().x, 1);
    }
    #[test]
    fn page_list_resize_no_reflow_more_rows() {
        let mut s = PageList::init(10, 3, None).unwrap();
        grow(&mut s, 0);
        let h = s.track_pin(s.pin(active(0, 2)).unwrap()).unwrap();
        s.resize(Resize {
            rows: Some(10),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 10);
        assert_eq!(s.total_rows, 10);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(0, 2))
        );
        finish(&s);
    }
    #[test]
    fn page_list_resize_no_reflow_more_rows_with_history() {
        let mut s = PageList::init(10, 3, None).unwrap();
        grow(&mut s, 50);
        let h = s.track_pin(s.pin(active(0, 2)).unwrap()).unwrap();
        s.resize(Resize {
            rows: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 5);
        assert_eq!(s.total_rows, 53);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(0, 4))
        );
        finish(&s);
    }
    #[test]
    fn page_list_resize_no_reflow_less_rows() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        fill(&mut s);
        s.resize(Resize {
            rows: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 5);
        assert_eq!(s.total_rows, 10);
        assert_eq!(s.active_offset(), 10 - 5);
        finish(&s);
    }
    #[test]
    fn page_list_resize_no_reflow_one_rows() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        fill(&mut s);
        s.resize(Resize {
            rows: Some(1),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.rows, 1);
        assert_eq!(s.total_rows, 10);
        assert_eq!(s.active_offset(), 10 - 1);
        finish(&s);
    }
    #[test]
    fn page_list_resize_no_reflow_less_cols() {
        let mut s = PageList::init(10, 3, None).unwrap();
        fill(&mut s);
        s.resize(Resize {
            cols: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 5);
        assert_eq!(s.total_rows, 3);
        for n in s.nodes.iter().flatten() {
            for y in 0..usize::from(n.data.size.rows) {
                assert_eq!(n.data.get_cells(y).unwrap().len(), 5);
            }
        }
        finish(&s);
    }
    #[test]
    fn page_list_resize_no_reflow_less_cols_pin_in_trimmed_cols() {
        let mut s = PageList::init(10, 3, None).unwrap();
        let h = s.track_pin(s.pin(active(9, 1)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(4, 1))
        );
        finish(&s);
    }
    #[test]
    fn page_list_resize_no_reflow_more_cols() {
        let mut s = PageList::init(5, 3, None).unwrap();
        fill(&mut s);
        s.resize(Resize {
            cols: Some(10),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 3);
        for y in 0..3 {
            for x in 0..10 {
                assert_eq!(
                    s.get_cell(screen(x, y)).unwrap().codepoint(),
                    if x < 5 { 65 } else { 0 }
                );
            }
        }
        finish(&s);
    }
    #[test]
    fn page_list_resize_no_reflow_less_cols_then_more_cols() {
        let mut s = PageList::init(10, 3, None).unwrap();
        fill(&mut s);
        s.resize(Resize {
            cols: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        s.resize(Resize {
            cols: Some(10),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        for y in 0..3 {
            for x in 0..10 {
                assert_eq!(
                    s.get_cell(screen(x, y)).unwrap().codepoint(),
                    if x < 5 { 65 } else { 0 }
                );
            }
        }
        finish(&s);
    }
    #[test]
    fn page_list_resize_reflow_more_cols_no_wrapped_rows() {
        let mut s = PageList::init(5, 3, Some(0)).unwrap();
        fill(&mut s);
        s.resize(Resize {
            cols: Some(10),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.cols, 10);
        assert_eq!(s.total_rows, 3);
        for p in s.row_iterator(Direction::RightDown, screen(0, 0), None) {
            assert_eq!(p.row_and_cell(&s).unwrap().1.codepoint(), 65);
        }
        finish(&s);
    }
    #[test]
    fn page_list_resize_reflow_more_cols_wrapped_rows() {
        let mut s = PageList::init(2, 4, Some(0)).unwrap();
        fill(&mut s);
        wraps(&mut s);
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 4);
        assert_eq!(s.active_offset(), 0);
        assert_eq!(s.diagram(), "AAAA\nAAAA\n    \n    \n");
        assert!(
            !s.get_top_left(Tag::Screen)
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .wrap()
        );
        finish(&s);
    }
    #[test]
    fn page_list_resize_reflow_less_cols_no_wrapped_rows() {
        let mut s = PageList::init(10, 3, Some(0)).unwrap();
        for y in 0..3 {
            for x in 0..4 {
                *s.get_cell_mut(screen(x, y)).unwrap() = Cell::init(u32::from(x));
            }
        }
        s.resize(Resize {
            cols: Some(5),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 3);
        for y in 0..3 {
            for x in 0..4 {
                assert_eq!(s.get_cell(screen(x, y)).unwrap().codepoint(), u32::from(x));
            }
        }
        finish(&s);
    }
    #[test]
    fn page_list_resize_reflow_less_cols_wrapped_rows() {
        let mut s = PageList::init(4, 2, None).unwrap();
        for y in 0..2 {
            for x in 0..4 {
                *s.get_cell_mut(screen(x, y)).unwrap() = Cell::init(u32::from(x));
            }
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 4);
        assert_eq!(s.active_offset(), 2);
        for y in 0..4 {
            let p = s.pin(screen(0, y)).unwrap();
            assert_eq!(p.row_and_cell(&s).unwrap().0.wrap(), y % 2 == 0);
            assert_eq!(p.row_and_cell(&s).unwrap().1.codepoint(), (y % 2) * 2);
        }
        finish(&s);
    }
    #[test]
    fn page_list_reset() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 0);
        s.reset().unwrap();
        assert_eq!(s.viewport, Viewport::Active);
        assert_eq!(s.total_rows, 24);
        assert_eq!(s.get_top_left(Tag::Active).unwrap().node, s.first.unwrap());
        finish(&s);
    }
    #[test]
    fn page_list_reset_across_two_pages() {
        let mut s = PageList::init(1000, 100, None).unwrap();
        grow(&mut s, 0);
        s.reset().unwrap();
        assert_eq!(s.viewport, Viewport::Active);
        assert_eq!(s.total_rows, 100);
        assert_eq!(s.get_top_left(Tag::Active).unwrap().node, s.first.unwrap());
        finish(&s);
    }
    #[test]
    fn page_list_clears_history() {
        let mut s = PageList::init(80, 24, None).unwrap();
        grow(&mut s, 30);
        s.reset().unwrap();
        assert_eq!(s.viewport, Viewport::Active);
        assert_eq!(s.total_rows, 24);
        assert_eq!(s.get_top_left(Tag::Active).unwrap().node, s.first.unwrap());
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
        let h = s.track_pin(s.pin(screen(0, 5)).unwrap()).unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        let p = s.tracked_pin(h).unwrap();
        assert_eq!(p.node, s.last.unwrap());
        assert_eq!(p.y, 0);
        assert_eq!(p.x, 0);
        finish(&s);
    }
    #[test]
    fn page_list_split_multiple_tracked_pins_across_regions() {
        let mut s = PageList::init(10, 10, None).unwrap();
        let handles: Vec<_> = (0..10)
            .map(|y| s.track_pin(s.pin(screen(y, y as u32)).unwrap()).unwrap())
            .collect();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        for (y, h) in handles.iter().enumerate() {
            let p = s.tracked_pin(*h).unwrap();
            assert_eq!(
                s.point_from_pin(Tag::Screen, p),
                Some(screen(y as u16, y as u32))
            );
            assert_eq!(
                p.node,
                if y < 5 {
                    s.first.unwrap()
                } else {
                    s.last.unwrap()
                }
            );
        }
        finish(&s);
    }
    #[test]
    fn page_list_split_tracked_viewport_pin_in_split_region_moves_correctly() {
        let mut s = PageList::init(10, 3, None).unwrap();
        grow(&mut s, 7);
        s.scroll(Scroll::Row(6));
        let before = s.scrollbar();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        assert_eq!(s.scrollbar(), before);
        assert_eq!(
            s.tracked_pin(s.viewport_pin.unwrap()).unwrap().node,
            s.last.unwrap()
        );
        finish(&s);
    }
    #[test]
    fn page_list_split_preserves_styled_cells() {
        let mut s = PageList::init(10, 10, None).unwrap();
        write_rows(&mut s);
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        for y in 0..10 {
            page.set_style(
                0,
                y,
                crate::style::Style {
                    flags: crate::style::Flags {
                        bold: true,
                        ..crate::style::Flags::default()
                    },
                    ..crate::style::Style::default()
                },
            )
            .unwrap();
        }
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        for y in 0..10 {
            let p = s.pin(screen(0, y)).unwrap();
            assert!(p.style(&s).unwrap().flags.bold);
        }
        finish(&s);
    }
    #[test]
    fn page_list_split_preserves_grapheme_clusters() {
        let mut s = PageList::init(10, 10, None).unwrap();
        write_rows(&mut s);
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        for y in 1..10 {
            page.append_grapheme(0, y, 0x301).unwrap();
        }
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        for y in 0..10 {
            let p = s.pin(screen(0, y)).unwrap();
            if y > 0 {
                assert_eq!(p.grapheme(&s).unwrap(), [0x301]);
            }
        }
        finish(&s);
    }
    #[test]
    fn page_list_split_preserves_hyperlinks() {
        let mut s = PageList::init(10, 10, None).unwrap();
        write_rows(&mut s);
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        let id = page
            .insert_hyperlink(crate::hyperlink::Hyperlink {
                id: crate::hyperlink::HyperlinkId::Implicit(1),
                uri: b"https://example.com".to_vec(),
            })
            .unwrap();
        for y in 0..10 {
            if y > 0 {
                page.hyperlink_set.r#use(id).unwrap();
            }
            page.set_hyperlink(0, y, id).unwrap();
        }
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        for y in 0..10 {
            let p = s.pin(screen(0, y)).unwrap();
            assert!(p.row_and_cell(&s).unwrap().1.hyperlink());
        }
        finish(&s);
    }
    #[test]
    fn page_list_split_preserves_wrap_flags() {
        let mut s = PageList::init(10, 10, None).unwrap();
        write_rows(&mut s);
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        for y in 0..10 {
            page.rows[y].set_wrap(y % 2 == 0);
            page.rows[y].set_wrap_continuation(y % 2 != 0);
        }
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        for y in 0..10 {
            let p = s.pin(screen(0, y)).unwrap();
            assert_eq!(p.row_and_cell(&s).unwrap().0.wrap(), y % 2 == 0);
            assert_eq!(
                p.row_and_cell(&s).unwrap().0.wrap_continuation(),
                y % 2 != 0
            );
        }
        finish(&s);
    }
    #[test]
    fn page_list_split_middle_page_preserves_linked_list_order() {
        let mut s = PageList::init(10, 10, None).unwrap();
        write_rows(&mut s);
        let original = s.first;
        s.split(s.pin(screen(0, 3)).unwrap()).unwrap();
        s.split(s.pin(screen(0, 7)).unwrap()).unwrap();
        s.split(s.pin(screen(0, 5)).unwrap()).unwrap();
        assert_eq!(s.total_pages(), 4);
        assert_eq!(s.first, original);
        for y in 0..10 {
            assert_eq!(s.get_cell(screen(0, y)).unwrap().codepoint(), y);
        }
        finish(&s);
    }
    #[test]
    fn page_list_split_last_page_makes_new_page_the_last() {
        let mut s = PageList::init(10, 10, None).unwrap();
        write_rows(&mut s);
        let original = s.first;
        s.split(s.pin(screen(0, 3)).unwrap()).unwrap();
        s.split(s.pin(screen(0, 7)).unwrap()).unwrap();
        assert_eq!(s.total_pages(), 3);
        assert_eq!(s.first, original);
        for y in 0..10 {
            assert_eq!(s.get_cell(screen(0, y)).unwrap().codepoint(), y);
        }
        finish(&s);
    }
    #[test]
    fn page_list_split_first_page_keeps_original_as_first() {
        let mut s = PageList::init(10, 10, None).unwrap();
        write_rows(&mut s);
        let original = s.first;
        s.split(s.pin(screen(0, 7)).unwrap()).unwrap();
        s.split(s.pin(screen(0, 3)).unwrap()).unwrap();
        assert_eq!(s.total_pages(), 3);
        assert_eq!(s.first, original);
        for y in 0..10 {
            assert_eq!(s.get_cell(screen(0, y)).unwrap().codepoint(), y);
        }
        finish(&s);
    }
    #[test]
    fn page_list_compact_std_size_page_returns_null() {
        let mut s = PageList::init(80, 24, None).unwrap();
        assert_eq!(s.compact(s.first.unwrap()).unwrap(), None);
        finish(&s);
    }
    #[test]
    fn page_list_compact_oversized_page() {
        let mut s = PageList::init(80, 24, None).unwrap();
        write_rows(&mut s);
        let h = s.track_pin(s.pin(screen(2, 10)).unwrap()).unwrap();
        let id = s
            .increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::StringBytes))
            .unwrap();
        let bytes = s.page_size;
        let compact = s.compact(id).unwrap().unwrap();
        assert!(s.page_size < bytes);
        assert_eq!(s.tracked_pin(h).unwrap().node, compact);
        for y in 0..24 {
            assert_eq!(s.get_cell(screen(0, y)).unwrap().codepoint(), y);
        }
        finish(&s);
    }

    #[test]
    fn page_list_clone_partial_trimmed_right() {
        let mut s = PageList::init(80, 20, None).unwrap();
        grow(&mut s, 30);
        let (q, _) = s.clone_range(screen(0, 0), Some(screen(0, 39))).unwrap();
        assert_eq!(q.total_rows, 40);
        finish(&q);
    }

    #[test]
    fn page_list_clone_partial_trimmed_left() {
        let mut s = PageList::init(80, 20, None).unwrap();
        grow(&mut s, 30);
        let (q, _) = s.clone_range(screen(0, 10), None).unwrap();
        assert_eq!(q.total_rows, 40);
        finish(&q);
    }

    #[test]
    fn page_list_clone_partial_trimmed_both() {
        let mut s = PageList::init(80, 20, None).unwrap();
        grow(&mut s, 30);
        let (q, _) = s.clone_range(screen(0, 10), Some(screen(0, 35))).unwrap();
        assert_eq!(q.total_rows, 26);
        finish(&q);
    }

    #[test]
    fn page_list_clone() {
        let s = PageList::init(80, 24, None).unwrap();
        let q = s.try_clone().unwrap();
        assert_eq!(q.total_rows, 24);
        finish(&q);
    }

    #[test]
    fn page_list_clone_less_than_active() {
        let s = PageList::init(80, 24, None).unwrap();
        let (q, _) = s.clone_range(active(0, 5), None).unwrap();
        assert_eq!(q.total_rows, 24);
        finish(&q);
    }

    #[test]
    fn page_list_clone_remap_tracked_pin() {
        let mut s = PageList::init(80, 24, None).unwrap();
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
        let h = s.track_pin(s.pin(active(0, 3)).unwrap()).unwrap();
        let (q, map) = s.clone_range(active(0, 5), None).unwrap();
        assert!(!map.contains_key(&h));
        finish(&q);
    }

    #[test]
    fn page_list_clone_full_dirty() {
        let mut s = PageList::init(80, 24, None).unwrap();
        for y in [0, 12, 23] {
            s.pin(active(0, y)).unwrap().mark_dirty(&mut s);
        }
        let q = s.try_clone().unwrap();
        for y in 0..24 {
            assert_eq!(q.is_dirty(active(0, y)), [0, 12, 23].contains(&y));
        }
        finish(&q);
    }

    #[test]
    fn page_list_clone_partial_trimmed_left_reclaims_styles() {
        let mut s = PageList::init(80, 20, None).unwrap();
        grow(&mut s, 30);
        let p = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        for y in 0..10 {
            p.set_style(
                0,
                y,
                crate::style::Style {
                    flags: crate::style::Flags {
                        bold: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let (q, _) = s.clone_range(screen(0, 10), None).unwrap();
        assert_eq!(q.total_rows, 40);
        assert_eq!(q.node(q.first.unwrap()).unwrap().data.styles.count(), 0);
        finish(&q);
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
        let h = s.track_pin(s.pin(active(2, 4)).unwrap()).unwrap();
        s.erase_rows(active(0, 0), Some(active(0, 3))).unwrap();
        assert_eq!(s.total_rows, 24);
        let p = s.tracked_pin(h).unwrap();
        assert_eq!((p.x, p.y), (2, 0));
        finish(&s);
    }

    #[test]
    fn page_list_erase_row_with_tracked_pin_is_erased() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let h = s.track_pin(s.pin(active(2, 2)).unwrap()).unwrap();
        s.erase_rows(active(0, 0), Some(active(0, 3))).unwrap();
        assert_eq!(s.total_rows, 24);
        let p = s.tracked_pin(h).unwrap();
        assert_eq!((p.x, p.y), (0, 0));
        finish(&s);
    }

    #[test]
    fn page_list_erase_active_regrows_automatically() {
        let mut s = PageList::init(80, 24, None).unwrap();
        s.erase_rows(active(0, 0), None).unwrap();
        assert_eq!(s.total_rows, 24);
        finish(&s);
    }

    #[test]
    fn page_list_erase_a_one_row_active() {
        let mut s = PageList::init(80, 1, None).unwrap();
        s.erase_rows(active(0, 0), None).unwrap();
        assert_eq!(s.total_rows, 1);
        finish(&s);
    }

    #[test]
    fn page_list_erase_row_bounded_less_than_full_row() {
        let mut s = PageList::init(80, 10, None).unwrap();
        let handles: Vec<_> = [5, 8, 9]
            .into_iter()
            .map(|y| s.track_pin(s.pin(active(0, y)).unwrap()).unwrap())
            .collect();
        s.erase_row_bounded(active(0, 5), 3).unwrap();
        for (h, expected) in handles.iter().zip([4, 7, 9]) {
            assert_eq!(s.tracked_pin(*h).unwrap().y, expected);
        }
        for y in 5..8 {
            assert!(s.is_dirty(active(0, y)));
        }
        finish(&s);
    }

    #[test]
    fn page_list_erase_row_bounded_with_pin_at_top() {
        let mut s = PageList::init(80, 10, None).unwrap();
        let h = s.track_pin(s.pin(active(5, 0)).unwrap()).unwrap();
        s.erase_row_bounded(active(0, 0), 3).unwrap();
        let p = s.tracked_pin(h).unwrap();
        assert_eq!((p.x, p.y), (0, 0));
        assert_eq!(s.total_rows, 10);
        finish(&s);
    }

    #[test]
    fn page_list_erase_row_bounded_full_rows_single_page() {
        let mut s = PageList::init(80, 10, None).unwrap();
        let a = s.track_pin(s.pin(active(0, 7)).unwrap()).unwrap();
        let b = s.track_pin(s.pin(active(0, 9)).unwrap()).unwrap();
        s.erase_row_bounded(active(0, 5), 10).unwrap();
        assert_eq!(s.tracked_pin(a).unwrap().y, 6);
        assert_eq!(s.tracked_pin(b).unwrap().y, 8);
        for y in 5..10 {
            assert!(s.is_dirty(active(0, y)));
        }
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_in_unchanged_row() {
        let mut s = PageList::init(4, 2, None).unwrap();
        for y in 0..2 {
            for x in 0..2 {
                *s.get_cell_mut(active(x, y)).unwrap() = Cell::init(u32::from(x));
            }
        }
        let h = s.track_pin(s.pin(active(1, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(1, 0))
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_in_blank_cell() {
        let mut s = PageList::init(6, 2, None).unwrap();
        for y in 0..2 {
            for x in 0..2 {
                *s.get_cell_mut(active(x, y)).unwrap() = Cell::init(u32::from(x));
            }
        }
        let h = s.track_pin(s.pin(active(2, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(2, 0))
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_in_final_blank_cell() {
        let mut s = PageList::init(6, 2, None).unwrap();
        for y in 0..2 {
            for x in 0..2 {
                *s.get_cell_mut(active(x, y)).unwrap() = Cell::init(u32::from(x));
            }
        }
        let h = s.track_pin(s.pin(active(3, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(3, 0))
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_in_wrapped_blank_cell() {
        let mut s = PageList::init(6, 2, None).unwrap();
        for y in 0..2 {
            for x in 0..2 {
                *s.get_cell_mut(active(x, y)).unwrap() = Cell::init(u32::from(x));
            }
        }
        let h = s.track_pin(s.pin(active(5, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(4),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(3, 0))
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_in_wrapped_row() {
        let mut s = PageList::init(4, 2, None).unwrap();
        for y in 0..2 {
            for x in 0..4 {
                *s.get_cell_mut(active(x, y)).unwrap() = Cell::init(u32::from(x));
            }
        }
        let h = s.track_pin(s.pin(active(2, 1)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 4);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(0, 1))
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_blank_lines() {
        let mut s = PageList::init(4, 3, Some(0)).unwrap();
        for y in [0, 0] {
            for x in 0..4 {
                *s.get_cell_mut(active(x, y)).unwrap() = Cell::init(u32::from(x));
            }
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 3);
        assert_eq!(s.get_cell(active(0, 1)).unwrap().codepoint(), 2);
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_blank_lines_between() {
        let mut s = PageList::init(4, 3, Some(0)).unwrap();
        for y in [0, 2] {
            for x in 0..4 {
                *s.get_cell_mut(active(x, y)).unwrap() = Cell::init(u32::from(x));
            }
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 5);
        assert_eq!(s.get_cell(active(0, 1)).unwrap().codepoint(), 0);
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_blank_lines_between_no_scrollback() {
        let mut s = PageList::init(5, 3, Some(0)).unwrap();
        *s.get_cell_mut(active(0, 0)).unwrap() = Cell::init(65);
        *s.get_cell_mut(active(0, 2)).unwrap() = Cell::init(67);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 3);
        assert_eq!(s.get_cell(active(0, 0)).unwrap().codepoint(), 65);
        assert_eq!(s.get_cell(active(0, 1)).unwrap().codepoint(), 0);
        assert_eq!(s.get_cell(active(0, 2)).unwrap().codepoint(), 67);
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_cursor_not_on_last_line_preserves_location() {
        let mut s = PageList::init(5, 5, Some(1)).unwrap();
        for y in 0..5 {
            for x in 0..2 {
                *s.get_cell_mut(active(x, y)).unwrap() = Cell::init(u32::from(x));
            }
        }
        grow(&mut s, 5);
        let h = s.track_pin(s.pin(active(0, 0)).unwrap()).unwrap();
        s.resize(Resize {
            cols: Some(4),
            cursor: Some(Cursor { x: 1, y: 1 }),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 10);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(0, 0))
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_no_reflow_preserves_semantic_prompt_on_first_line() {
        let mut s = PageList::init(4, 4, Some(0)).unwrap();
        s.node_mut(s.first.unwrap()).unwrap().data.rows[0]
            .set_semantic_prompt(SemanticPrompt::Prompt);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 4);
        assert_eq!(
            s.get_top_left(Tag::Active)
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .semantic_prompt(),
            SemanticPrompt::Prompt
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_wrap_preserves_semantic_prompt() {
        let mut s = PageList::init(4, 4, Some(0)).unwrap();
        s.node_mut(s.first.unwrap()).unwrap().data.rows[0]
            .set_semantic_prompt(SemanticPrompt::Prompt);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 4);
        assert_eq!(
            s.get_top_left(Tag::Active)
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .semantic_prompt(),
            SemanticPrompt::Prompt
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_no_reflow_preserves_semantic_prompt() {
        let mut s = PageList::init(4, 4, Some(0)).unwrap();
        s.node_mut(s.first.unwrap()).unwrap().data.rows[1]
            .set_semantic_prompt(SemanticPrompt::Prompt);
        for x in 0..4 {
            *s.get_cell_mut(active(x, 1)).unwrap() = Cell::init(u32::from(x));
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 4);
        for y in [1, 2] {
            assert_eq!(
                s.pin(active(0, y))
                    .unwrap()
                    .row_and_cell(&s)
                    .unwrap()
                    .0
                    .semantic_prompt(),
                SemanticPrompt::Prompt
            );
        }
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_to_wrap_a_wide_char() {
        let mut s = PageList::init(3, 1, Some(0)).unwrap();
        *s.get_cell_mut(active(0, 0)).unwrap() = Cell::init(120);
        let c = s.get_cell_mut(active(1, 0)).unwrap();
        *c = Cell::init(0x1f600);
        c.set_wide(Wide::Wide);
        s.get_cell_mut(active(2, 0))
            .unwrap()
            .set_wide(Wide::SpacerTail);
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 2);
        assert_eq!(s.get_cell(screen(1, 0)).unwrap().wide(), Wide::SpacerHead);
        assert_eq!(s.get_cell(screen(0, 1)).unwrap().codepoint(), 0x1f600);
        assert_eq!(s.get_cell(screen(1, 1)).unwrap().wide(), Wide::SpacerTail);
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_more_cols_unwrap_still_requires_wide_spacer_head() {
        let mut s = PageList::init(2, 2, Some(0)).unwrap();
        for x in 0..2 {
            *s.get_cell_mut(active(x, 0)).unwrap() = Cell::init(120);
        }
        wraps(&mut s);
        let c = s.get_cell_mut(active(0, 1)).unwrap();
        *c = Cell::init(0x1f600);
        c.set_wide(Wide::Wide);
        s.get_cell_mut(active(1, 1))
            .unwrap()
            .set_wide(Wide::SpacerTail);
        s.resize(Resize {
            cols: Some(3),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 2);
        assert_eq!(s.get_cell(screen(2, 0)).unwrap().wide(), Wide::SpacerHead);
        assert_eq!(s.get_cell(screen(0, 1)).unwrap().codepoint(), 0x1f600);
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_to_wrap_a_multi_codepoint_grapheme_with_a_spacer_head() {
        let mut s = PageList::init(4, 2, Some(0)).unwrap();
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        let cps = [0x200d, 0x1f468, 0x200d, 0x1f466, 0x200d, 0x1f466];
        for x in [0, 2] {
            page.cells[x] = Cell::init(0x1f468);
            page.cells[x].set_wide(Wide::Wide);
            page.cells[x + 1].set_wide(Wide::SpacerTail);
            page.set_graphemes(x, 0, &cps).unwrap();
        }
        s.resize(Resize {
            cols: Some(3),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 2);
        for y in 0..2 {
            let p = s.pin(screen(0, y)).unwrap();
            assert_eq!(p.row_and_cell(&s).unwrap().1.wide(), Wide::Wide);
            assert_eq!(p.grapheme(&s).unwrap(), cps);
        }
        assert_eq!(s.get_cell(screen(2, 0)).unwrap().wide(), Wide::SpacerHead);
        finish(&s);
    }

    #[test]
    fn page_list_prompt_iterator_left_up() {
        let mut s = PageList::init(2, 20, Some(0)).unwrap();
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        for y in [3, 6] {
            page.rows[y].set_semantic_prompt(SemanticPrompt::Prompt);
        }
        for y in [7, 8, 12] {
            page.rows[y].set_semantic_prompt(SemanticPrompt::PromptContinuation);
        }
        let ys: Vec<_> = s
            .prompt_iterator(Direction::LeftUp, screen(0, 0), None)
            .map(|p| s.screen_row(p).unwrap())
            .collect();
        assert_eq!(ys, [12, 6, 3]);
    }

    #[test]
    fn page_list_prompt_iterator_right_down() {
        let mut s = PageList::init(2, 20, Some(0)).unwrap();
        let page = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        for y in [3, 6] {
            page.rows[y].set_semantic_prompt(SemanticPrompt::Prompt);
        }
        for y in [7, 8, 12] {
            page.rows[y].set_semantic_prompt(SemanticPrompt::PromptContinuation);
        }
        let ys: Vec<_> = s
            .prompt_iterator(Direction::RightDown, screen(0, 0), None)
            .map(|p| s.screen_row(p).unwrap())
            .collect();
        assert_eq!(ys, [3, 6, 12]);
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
        *s.get_cell_mut(active(0, 0)).unwrap() = Cell::init(65);
        let h = s.track_pin(s.pin(active(0, 0)).unwrap()).unwrap();
        s.resize(Resize {
            rows: Some(2),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 2);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(0, 0))
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_no_reflow_less_rows_trims_blank_lines_cursor_in_blank_line() {
        let mut s = PageList::init(10, 5, Some(0)).unwrap();
        *s.get_cell_mut(active(0, 0)).unwrap() = Cell::init(65);
        let h = s.track_pin(s.pin(active(0, 3)).unwrap()).unwrap();
        s.resize(Resize {
            rows: Some(2),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 4);
        assert_eq!(
            s.point_from_pin(Tag::Active, s.tracked_pin(h).unwrap()),
            Some(active(0, 1))
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_no_reflow_less_rows_trims_blank_lines_erases_pages() {
        let mut s = PageList::init(100, 5, Some(0)).unwrap();
        let rows = first_cap(&s) as u16 + 5;
        s.resize(Resize {
            rows: Some(rows),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_pages(), 2);
        *s.get_cell_mut(active(0, 0)).unwrap() = Cell::init(65);
        s.resize(Resize {
            rows: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_pages(), 1);
        assert_eq!(s.total_rows, 5);
        finish(&s);
    }

    #[test]
    fn page_list_resize_no_reflow_more_rows_extends_blank_lines() {
        let mut s = PageList::init(10, 3, Some(0)).unwrap();
        *s.get_cell_mut(active(0, 0)).unwrap() = Cell::init(65);
        s.resize(Resize {
            rows: Some(7),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 7);
        assert_eq!(s.active_offset(), 0);
        assert_eq!(s.get_cell(active(0, 0)).unwrap().codepoint(), 65);
        finish(&s);
    }

    #[test]
    fn page_list_resize_no_reflow_less_cols_clears_graphemes() {
        let mut s = PageList::init(10, 10, Some(0)).unwrap();
        let p = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        p.cells[9] = Cell::init(9);
        p.append_grapheme(9, 0, 10).unwrap();
        assert_eq!(p.grapheme_count(), 1);
        s.resize(Resize {
            cols: Some(5),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
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
        finish(&s);
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
        finish(&s);
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
        finish(&s);
    }

    #[test]
    fn page_list_increase_capacity_after_col_shrink() {
        let mut s = PageList::init(80, 24, None).unwrap();
        s.resize(Resize {
            cols: Some(40),
            reflow: false,
            ..Resize::default()
        })
        .unwrap();
        let id = s
            .increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::Styles))
            .unwrap();
        assert_eq!(s.node(id).unwrap().data.size.cols, 40);
        assert_eq!(s.node(id).unwrap().data.capacity.cols, 80);
        finish(&s);
    }

    #[test]
    fn page_list_increase_capacity_multi_page() {
        let mut s = PageList::init(80, 24, None).unwrap();
        fill_pages(&mut s, 2);
        let old = s.first.unwrap();
        let second = s.node(old).unwrap().next.unwrap();
        let h = s
            .track_pin(Pin {
                node: old,
                y: 2,
                x: 3,
                garbage: false,
            })
            .unwrap();
        let id = s
            .increase_capacity(old, Some(IncreaseCapacity::Styles))
            .unwrap();
        assert_eq!(s.node(id).unwrap().next, Some(second));
        assert_eq!(s.node(second).unwrap().prev, Some(id));
        assert_eq!(s.tracked_pin(h).unwrap().node, id);
        finish(&s);
    }

    #[test]
    fn page_list_compact_insufficient_savings_returns_null() {
        let mut s = PageList::init(80, 24, Some(0)).unwrap();
        let id = s
            .increase_capacity(s.first.unwrap(), Some(IncreaseCapacity::GraphemeBytes))
            .unwrap();
        let bytes = s.page_size;
        let result = s.compact(id).unwrap();
        if result.is_some() {
            assert!(s.page_size < bytes);
        }
        finish(&s);
    }

    #[test]
    fn page_list_page_iterator_history_two_pages() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let cap = first_cap(&s);
        fill_pages(&mut s, 2);
        let chunks: Vec<_> = s
            .page_iterator(
                Direction::RightDown,
                Point::History(Coordinate::default()),
                None,
            )
            .collect();
        assert_eq!(chunks.len(), 2);
        assert_eq!((chunks[0].start, chunks[0].end), (0, cap as u16));
        assert_eq!((chunks[1].start, chunks[1].end), (0, cap as u16 - 24));
    }

    #[test]
    fn page_list_page_iterator_reverse_history_two_pages() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let cap = first_cap(&s);
        fill_pages(&mut s, 2);
        let mut chunks: Vec<_> = s
            .page_iterator(
                Direction::LeftUp,
                Point::History(Coordinate::default()),
                None,
            )
            .collect();
        chunks.reverse();
        assert_eq!(chunks.len(), 2);
        assert_eq!((chunks[0].start, chunks[0].end), (0, cap as u16));
        assert_eq!((chunks[1].start, chunks[1].end), (0, cap as u16 - 24));
    }

    #[test]
    fn page_list_erase_row_bounded_full_rows_two_pages() {
        let mut s = PageList::init(80, 24, None).unwrap();
        let cap = first_cap(&s);
        fill_pages(&mut s, 2);
        write_rows(&mut s);
        let h = s.track_pin(s.pin(screen(2, cap as u32)).unwrap()).unwrap();
        let counts: Vec<_> = s.nodes.iter().flatten().map(|n| n.data.size.rows).collect();
        s.erase_row_bounded(screen(0, cap as u32 - 2), 10).unwrap();
        assert_eq!(
            s.nodes
                .iter()
                .flatten()
                .map(|n| n.data.size.rows)
                .collect::<Vec<_>>(),
            counts
        );
        assert_eq!(
            s.point_from_pin(Tag::Screen, s.tracked_pin(h).unwrap()),
            Some(screen(2, cap as u32 - 1))
        );
        assert_eq!(
            s.get_cell(screen(0, cap as u32 - 2)).unwrap().codepoint(),
            cap as u32 - 1
        );
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_wrapped_rows_with_graphemes() {
        let mut s = PageList::init(4, 2, None).unwrap();
        let p = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        for y in 0..2 {
            for x in 0..4 {
                let i = p.cell_index(x, y).unwrap();
                p.cells[i] = Cell::init(x as u32 + 1);
                p.append_grapheme(x, y, 10).unwrap();
            }
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 4);
        for pin in s.cell_iterator(Direction::RightDown, screen(0, 0), None) {
            assert_eq!(pin.grapheme(&s).unwrap(), [10]);
        }
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_copy_style() {
        let mut s = PageList::init(4, 2, None).unwrap();
        fill(&mut s);
        let p = &mut s.node_mut(s.first.unwrap()).unwrap().data;
        let style = crate::style::Style {
            fg_color: crate::style::Color::Palette(2),
            ..Default::default()
        };
        for y in 0..2 {
            for x in 0..4 {
                p.set_style(x, y, style).unwrap();
            }
        }
        s.resize(Resize {
            cols: Some(2),
            ..Resize::default()
        })
        .unwrap();
        for pin in s.cell_iterator(Direction::RightDown, screen(0, 0), None) {
            assert_eq!(pin.style(&s).unwrap(), style);
        }
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_less_cols_wraps_spacer_head() {
        let mut s = PageList::init(4, 3, Some(0)).unwrap();
        for x in 0..3 {
            *s.get_cell_mut(active(x, 0)).unwrap() = Cell::init(120);
        }
        s.get_cell_mut(active(3, 0))
            .unwrap()
            .set_wide(Wide::SpacerHead);
        s.node_mut(s.first.unwrap()).unwrap().data.rows[0].set_wrap(true);
        s.node_mut(s.first.unwrap()).unwrap().data.rows[1].set_wrap_continuation(true);
        let c = s.get_cell_mut(active(0, 1)).unwrap();
        *c = Cell::init(0x1f600);
        c.set_wide(Wide::Wide);
        s.get_cell_mut(active(1, 1))
            .unwrap()
            .set_wide(Wide::SpacerTail);
        s.resize(Resize {
            cols: Some(3),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 3);
        for x in 0..3 {
            assert_eq!(s.get_cell(screen(x, 0)).unwrap().codepoint(), 120);
        }
        assert_eq!(s.get_cell(screen(0, 1)).unwrap().codepoint(), 0x1f600);
        finish(&s);
    }

    #[test]
    fn page_list_resize_reflow_more_cols_no_reflow_preserves_semantic_prompt() {
        let mut s = PageList::init(4, 4, Some(0)).unwrap();
        s.node_mut(s.first.unwrap()).unwrap().data.rows[0]
            .set_semantic_prompt(SemanticPrompt::Prompt);
        s.resize(Resize {
            cols: Some(8),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 4);
        assert_eq!(
            s.get_top_left(Tag::Active)
                .unwrap()
                .row_and_cell(&s)
                .unwrap()
                .0
                .semantic_prompt(),
            SemanticPrompt::Prompt
        );
        finish(&s);
    }
    #[test]
    fn page_list_scroll_clear() {
        let mut s = PageList::init(80, 24, None).unwrap();
        *s.get_cell_mut(active(0, 0)).unwrap() = Cell::init(65);
        s.scroll_clear().unwrap();
        assert_eq!(s.total_rows, 25);
        assert_eq!(s.get_cell(active(0, 0)).unwrap().codepoint(), 0);
        s.scroll_clear().unwrap();
        assert_eq!(s.total_rows, 25);
        finish(&s);
    }
    #[test]
    fn page_list_resize_reflow_less_cols_to_eliminate_a_wide_char() {
        let mut s = PageList::init(2, 1, Some(0)).unwrap();
        let c = s.get_cell_mut(active(0, 0)).unwrap();
        *c = Cell::init(0x1f600);
        c.set_wide(Wide::Wide);
        s.get_cell_mut(active(1, 0))
            .unwrap()
            .set_wide(Wide::SpacerTail);
        s.resize(Resize {
            cols: Some(1),
            ..Resize::default()
        })
        .unwrap();
        assert_eq!(s.total_rows, 1);
        assert_eq!(s.get_cell(active(0, 0)).unwrap(), Cell::default());
        finish(&s);
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
}
