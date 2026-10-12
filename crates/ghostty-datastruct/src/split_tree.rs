//! `SplitTree` represents a tree of view types that can be divided.
//!
//! Concretely for Ghostty, it represents a tree of terminal views. In
//! its basic state, there are no splits and it is a single full-sized
//! terminal. However, it can be split arbitrarily many times among two
//! axes (horizontal and vertical) to create a tree of terminal views.
//!
//! This is an immutable tree structure, meaning all operations on it
//! will return a new tree with the operation applied. This allows us to
//! store versions of the tree in a history for easy undo/redo. To facilitate
//! this, the stored View type must implement reference counting; in Rust that
//! is expressed with [`Clone`] (the Zig `ref`; e.g. `Rc<T>`, a GObject
//! handle) and [`Drop`] (the Zig `unref`).
//!
//! Optionally a view can implement [`SplitTreeView::split_tree_label`] to
//! provide a label used by the debug view. If this isn't specified then the
//! node handle will be used.
//!
//! Deviations from the Zig version: allocation is infallible (no allocator
//! parameters / `Allocator.Error`), the arena is replaced by a `Vec`, and the
//! GObject boxed-type glue (`getGObjectType`) is left to the GTK crate.

use std::fmt;

use half::f16;

/// The requirements on the view type stored in a [`SplitTree`].
pub trait SplitTreeView: Clone {
    /// Return a label that is used for the debug view. If this returns
    /// `None` then the node handle will be used.
    fn split_tree_label(&self) -> Option<String> {
        None
    }
}

/// Error returned by [`SplitTree::split`] when the resulting tree would have
/// more nodes than a [`Handle`] can address (`error.OutOfMemory` in Zig).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SplitError {
    #[error("out of memory: too many nodes in split tree")]
    OutOfMemory,
}

/// A handle into the nodes array. This lets us keep track of nodes with
/// 16-bit handles rather than full pointer-width values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(u16);

impl Handle {
    /// The root node (index 0).
    pub const ROOT: Handle = Handle(0);

    /// Create a handle from an index.
    ///
    /// # Panics
    /// If the index doesn't fit in the handle backing type.
    #[must_use]
    pub fn from_idx(idx: usize) -> Handle {
        Handle(u16::try_from(idx).expect("split tree handle out of range"))
    }

    #[inline]
    #[must_use]
    pub fn idx(self) -> usize {
        usize::from(self.0)
    }

    /// Offset the handle by a given amount.
    #[must_use]
    pub fn offset(self, v: usize) -> Handle {
        let final_ = self.idx() + v;
        assert!(final_ < usize::from(u16::MAX));
        Handle::from_idx(final_)
    }
}

#[derive(Debug, Clone)]
pub enum Node<V> {
    Leaf(V),
    Split(Split),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Split {
    pub layout: Layout,
    pub ratio: f16,
    pub left: Handle,
    pub right: Handle,
}

/// Alias matching the Zig `Split.Layout` / `Split.Direction` names.
pub type SplitLayout = Layout;
pub type SplitDirection = Direction;

/// A view in the tree with its handle.
#[derive(Debug, Clone, Copy)]
pub struct ViewEntry<'a, V> {
    pub handle: Handle,
    pub view: &'a V,
}

/// An iterator over all the views in the tree.
pub struct Iter<'a, V> {
    i: usize,
    nodes: &'a [Node<V>],
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = ViewEntry<'a, V>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // If we have no nodes, return None.
            if self.i >= self.nodes.len() {
                return None;
            }

            // Get the current node and increment the index.
            let handle = Handle::from_idx(self.i);
            self.i += 1;
            if let Node::Leaf(v) = &self.nodes[handle.idx()] {
                return Some(ViewEntry { handle, view: v });
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goto {
    /// Previous view, null if we're the first view.
    Previous,

    /// Next view, null if we're the last view.
    Next,

    /// Previous view, but wrapped around to the last view. May
    /// return the same view if this is the first view.
    PreviousWrapped,

    /// Next view, but wrapped around to the first view. May return
    /// the same view if this is the last view.
    NextWrapped,

    /// A spatial direction. "Spatial" means that the direction is
    /// based on the nearest surface in the given direction visually
    /// as the surfaces are laid out on a 2D grid.
    Spatial(SpatialDirection),
}

pub type SpatialDirection = Direction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum Backtrack {
    Deadend,
    Backtrack,
    Result(Handle),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub x: f16,
    pub y: f16,
    pub width: f16,
    pub height: f16,
}

impl Slot {
    fn max_x(self) -> f16 {
        self.x + self.width
    }

    fn max_y(self) -> f16 {
        self.y + self.height
    }
}

/// Spatial representation of the split tree. See [`SplitTree::spatial`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spatial {
    /// The slots of the spatial representation in the same order
    /// as the tree it was created from.
    pub slots: Vec<Slot>,
}

impl Spatial {
    pub const EMPTY: Spatial = Spatial { slots: Vec::new() };
}

/// Unsigned integer to f16 (`@floatFromInt`).
fn f16_from_usize(v: usize) -> f16 {
    f16::from_f32(v as f32)
}

fn f16_sqrt(v: f16) -> f16 {
    f16::from_f32(v.to_f32().sqrt())
}

fn f16_min(a: f16, b: f16) -> f16 {
    if b < a { b } else { a }
}

fn f16_max(a: f16, b: f16) -> f16 {
    if b > a { b } else { a }
}

/// See the module documentation.
#[derive(Debug, Clone)]
pub struct SplitTree<V> {
    /// All the nodes in the tree. Node at index 0 is always the root.
    nodes: Vec<Node<V>>,

    /// The handle of the zoomed node. A "zoomed" node is one that is
    /// expected to be made the full size of the split tree. Various
    /// operations may unzoom (e.g. resize).
    zoomed: Option<Handle>,
}

impl<V> Default for SplitTree<V> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<V> SplitTree<V> {
    /// An empty tree.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            zoomed: None,
        }
    }

    /// All nodes of the tree. Node at index 0 is the root.
    #[must_use]
    pub fn nodes(&self) -> &[Node<V>] {
        &self.nodes
    }

    /// The zoomed node, if any.
    #[must_use]
    pub fn zoomed(&self) -> Option<Handle> {
        self.zoomed
    }

    /// Returns true if this is an empty tree.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        // An empty tree has no nodes.
        self.nodes.is_empty()
    }

    /// Returns true if this tree has more than one split (i.e., the root
    /// is a split node). This is useful for determining if actions like
    /// resize_split or toggle_split_zoom are performable.
    #[must_use]
    pub fn is_split(&self) -> bool {
        // An empty tree is not split.
        if self.is_empty() {
            return false;
        }
        // The root node is at index 0. If it's a split, we have multiple splits.
        matches!(self.nodes[0], Node::Split(_))
    }

    /// An iterator over all the views in the tree.
    #[must_use]
    pub fn iterator(&self) -> Iter<'_, V> {
        Iter {
            i: 0,
            nodes: &self.nodes,
        }
    }

    /// Change the zoomed state to the given node. Assumes the handle
    /// is valid.
    pub fn zoom(&mut self, handle: Option<Handle>) {
        if let Some(v) = handle {
            assert!(v.idx() < self.nodes.len());
        }
        self.zoomed = handle;
    }

    fn split_node(&self, h: Handle) -> Option<&Split> {
        match &self.nodes[h.idx()] {
            Node::Split(s) => Some(s),
            Node::Leaf(_) => None,
        }
    }

    /// Goto a view from a certain point in the split tree. Returns `None`
    /// if the direction results in no visitable view.
    #[must_use]
    pub fn goto(&self, from: Handle, to: Goto) -> Option<Handle> {
        match to {
            Goto::Previous => self.previous(from),
            Goto::Next => self.next(from),
            Goto::PreviousWrapped => self
                .previous(from)
                .or_else(|| Some(self.deepest(Side::Right, Handle::ROOT))),
            Goto::NextWrapped => self
                .next(from)
                .or_else(|| Some(self.deepest(Side::Left, Handle::ROOT))),
            Goto::Spatial(d) => {
                // Get our spatial representation.
                let sp = self.spatial();
                self.nearest_wrapped(&sp, from, d)
            }
        }
    }

    /// Returns the deepest view in the tree in the given direction.
    /// This can be used to find the leftmost/rightmost surface within
    /// a given split structure.
    #[must_use]
    pub fn deepest(&self, side: Side, from: Handle) -> Handle {
        let mut current = from;
        loop {
            match &self.nodes[current.idx()] {
                Node::Leaf(_) => return current,
                Node::Split(s) => {
                    current = match side {
                        Side::Left => s.left,
                        Side::Right => s.right,
                    };
                }
            }
        }
    }

    /// Returns the previous view from the given node handle (which itself
    /// doesn't need to be a view). If there is no previous (this is the
    /// most previous view) then this will return `None`.
    ///
    /// "Previous" is defined as the previous node in an in-order
    /// traversal of the tree. This isn't a perfect definition and we
    /// may want to change this to something that better matches a
    /// spatial view of the tree later.
    fn previous(&self, from: Handle) -> Option<Handle> {
        match self.previous_backtrack(from, Handle::ROOT) {
            Backtrack::Result(v) => Some(v),
            Backtrack::Backtrack | Backtrack::Deadend => None,
        }
    }

    /// Same as `previous`, but returns the next view instead.
    fn next(&self, from: Handle) -> Option<Handle> {
        match self.next_backtrack(from, Handle::ROOT) {
            Backtrack::Result(v) => Some(v),
            Backtrack::Backtrack | Backtrack::Deadend => None,
        }
    }

    // Design note: we use a recursive backtracking search because
    // split trees are never that deep, so we can abuse the stack as
    // a safe allocator (stack overflow unlikely unless the kernel is
    // tuned in some really weird way).
    fn previous_backtrack(&self, from: Handle, current: Handle) -> Backtrack {
        // If we reached the point that we're trying to find the previous
        // value of, then we need to backtrack from here.
        if from == current {
            return Backtrack::Backtrack;
        }

        match &self.nodes[current.idx()] {
            // If we hit a leaf that isn't our target, then deadend.
            Node::Leaf(_) => Backtrack::Deadend,

            Node::Split(s) => match self.previous_backtrack(from, s.left) {
                Backtrack::Result(v) => Backtrack::Result(v),

                // Backtrack from the left means we have to continue
                // backtracking because we can't see what's before the left.
                Backtrack::Backtrack => Backtrack::Backtrack,

                // If we hit a deadend on the left then let's move right.
                Backtrack::Deadend => match self.previous_backtrack(from, s.right) {
                    Backtrack::Result(v) => Backtrack::Result(v),

                    // Deadend means its not in this split at all since
                    // we already tracked the left.
                    Backtrack::Deadend => Backtrack::Deadend,

                    // Backtrack means that its in our left view because
                    // we can see the immediate previous and there MUST
                    // be leaves (we can't have split-only leaves).
                    Backtrack::Backtrack => Backtrack::Result(self.deepest(Side::Right, s.left)),
                },
            },
        }
    }

    // See previous_backtrack for detailed comments. This is a mirror of that.
    fn next_backtrack(&self, from: Handle, current: Handle) -> Backtrack {
        if from == current {
            return Backtrack::Backtrack;
        }
        match &self.nodes[current.idx()] {
            Node::Leaf(_) => Backtrack::Deadend,
            Node::Split(s) => match self.next_backtrack(from, s.right) {
                Backtrack::Result(v) => Backtrack::Result(v),
                Backtrack::Backtrack => Backtrack::Backtrack,
                Backtrack::Deadend => match self.next_backtrack(from, s.left) {
                    Backtrack::Result(v) => Backtrack::Result(v),
                    Backtrack::Deadend => Backtrack::Deadend,
                    Backtrack::Backtrack => Backtrack::Result(self.deepest(Side::Left, s.right)),
                },
            },
        }
    }

    /// Returns the nearest leaf node (view) in the given direction.
    /// This does not handle wrapping and will return `None` if there
    /// is no node in that direction.
    fn nearest(
        &self,
        sp: &Spatial,
        from: Handle,
        direction: SpatialDirection,
        target: Slot,
    ) -> Option<Handle> {
        let mut result: Option<(Handle, f16)> = None;
        for (handle, slot) in sp.slots.iter().enumerate() {
            // Never match ourself
            if handle == from.idx() {
                continue;
            }

            // Only match leaves
            if matches!(self.nodes[handle], Node::Split(_)) {
                continue;
            }

            // Ensure it is in the proper direction
            let ok = match direction {
                Direction::Left => slot.max_x() <= target.x,
                Direction::Right => slot.x >= target.max_x(),
                Direction::Up => slot.max_y() <= target.y,
                Direction::Down => slot.y >= target.max_y(),
            };
            if !ok {
                continue;
            }

            // Track our distance
            let dx = slot.x - target.x;
            let dy = slot.y - target.y;
            let distance = f16_sqrt(dx * dx + dy * dy);

            // If we have a nearest it must be closer.
            if let Some((_, d)) = result
                && distance >= d
            {
                continue;
            }
            result = Some((Handle::from_idx(handle), distance));
        }

        result.map(|(h, _)| h)
    }

    /// Same as nearest but supports wrapping.
    fn nearest_wrapped(
        &self,
        sp: &Spatial,
        from: Handle,
        direction: SpatialDirection,
    ) -> Option<Handle> {
        // If we can find a nearest value without wrapping, then use that.
        let mut target = sp.slots[from.idx()];
        if let Some(v) = self.nearest(sp, from, direction, target) {
            return Some(v);
        }

        // The spatial grid is normalized to 1x1, so wrapping is modeled
        // by shifting the target slot by one full grid in the opposite
        // direction and reusing the same nearest distance logic.
        // We don't actually modify the grid or spatial representation,
        // this just fakes it.
        let one = f16::ONE;
        assert!(target.x >= f16::ZERO && target.y >= f16::ZERO);
        assert!(target.max_x() <= one && target.max_y() <= one);
        match direction {
            Direction::Left => target.x += one,
            Direction::Right => target.x -= one,
            Direction::Up => target.y += one,
            Direction::Down => target.y -= one,
        }

        self.nearest(sp, from, direction, target)
    }

    /// Resize the given node in place. The node MUST be a split (asserted).
    ///
    /// In general, this is an immutable data structure so this is
    /// heavily discouraged. However, this is provided for convenience
    /// and performance reasons where its very important for GUIs to
    /// update the ratio during a live resize than to redraw the entire
    /// widget tree.
    pub fn resize_in_place(&mut self, at: Handle, ratio: f16) {
        match &mut self.nodes[at.idx()] {
            Node::Split(s) => s.ratio = ratio,
            Node::Leaf(_) => panic!("resize_in_place on a leaf node"),
        }
    }

    /// Returns the number of nodes that would be needed to store
    /// the tree if the target node is removed.
    fn count_after_removal(&self, current: Handle, target: Handle, acc: usize) -> usize {
        assert_ne!(current, target);

        match &self.nodes[current.idx()] {
            // Leaf is simple, always takes one node.
            Node::Leaf(_) => acc + 1,

            // Split is slightly more complicated. If either side is the
            // target to remove, then we remove the split node as well
            // so our count is just the count of the other side.
            //
            // If neither side is the target, then we count both sides
            // and add one to account for the split node itself.
            Node::Split(s) => {
                if s.left == target {
                    self.count_after_removal(s.right, target, acc)
                } else if s.right == target {
                    self.count_after_removal(s.left, target, acc)
                } else {
                    self.count_after_removal(s.left, target, acc)
                        + self.count_after_removal(s.right, target, acc)
                        + 1
                }
            }
        }
    }

    fn weight(&self, from: Handle, layout: Layout, acc: usize) -> usize {
        match &self.nodes[from.idx()] {
            Node::Leaf(_) => acc + 1,
            Node::Split(s) => {
                if s.layout == layout {
                    self.weight(s.left, layout, acc) + self.weight(s.right, layout, acc)
                } else {
                    1
                }
            }
        }
    }

    fn find_parent_split(&self, layout: Layout, from: Handle, current: Handle) -> Backtrack {
        if from == current {
            return Backtrack::Backtrack;
        }
        match &self.nodes[current.idx()] {
            Node::Leaf(_) => Backtrack::Deadend,
            Node::Split(s) => match self.find_parent_split(layout, from, s.left) {
                Backtrack::Result(v) => Backtrack::Result(v),
                Backtrack::Backtrack => {
                    if s.layout == layout {
                        Backtrack::Result(current)
                    } else {
                        Backtrack::Backtrack
                    }
                }
                Backtrack::Deadend => match self.find_parent_split(layout, from, s.right) {
                    Backtrack::Deadend => Backtrack::Deadend,
                    Backtrack::Result(v) => Backtrack::Result(v),
                    Backtrack::Backtrack => {
                        if s.layout == layout {
                            Backtrack::Result(current)
                        } else {
                            Backtrack::Backtrack
                        }
                    }
                },
            },
        }
    }

    /// Spatial representation of the split tree. This can be used to
    /// better understand the layout of the tree in a 2D space.
    ///
    /// The bounds of the representation are always based on the total
    /// 2D space being 1x1. The x/y coordinates and width/height dimensions
    /// of each individual split and leaf are relative to this.
    /// This means that the spatial representation is a normalized
    /// representation of the actual space.
    ///
    /// The top-left corner of the tree is always (0, 0).
    ///
    /// We use a normalized form because we can calculate it without
    /// accessing to the actual rendered view sizes. These actual sizes
    /// may not be available at various times because GUI toolkits often
    /// only make them available once they're part of a widget tree and
    /// a SplitTree can represent views that aren't currently visible.
    #[must_use]
    pub fn spatial(&self) -> Spatial {
        // No nodes, empty spatial representation.
        if self.nodes.is_empty() {
            return Spatial::EMPTY;
        }

        // Get our total dimensions.
        let (dim_w, dim_h) = self.dimensions(Handle::ROOT);

        // Create our slots which will match our nodes exactly.
        let zero = Slot {
            x: f16::ZERO,
            y: f16::ZERO,
            width: f16::ZERO,
            height: f16::ZERO,
        };
        let mut slots = vec![zero; self.nodes.len()];
        slots[0] = Slot {
            x: f16::ZERO,
            y: f16::ZERO,
            width: f16_from_usize(usize::from(dim_w)),
            height: f16_from_usize(usize::from(dim_h)),
        };
        self.fill_spatial_slots(&mut slots, Handle::ROOT);

        // Normalize the dimensions to 1x1 grid.
        let fw = f16_from_usize(usize::from(dim_w));
        let fh = f16_from_usize(usize::from(dim_h));
        for slot in &mut slots {
            slot.x /= fw;
            slot.y /= fh;
            slot.width /= fw;
            slot.height /= fh;
        }

        Spatial { slots }
    }

    fn fill_spatial_slots(&self, slots: &mut [Slot], current_: Handle) {
        let current = current_.idx();
        assert!(slots[current].width >= f16::ZERO && slots[current].height >= f16::ZERO);
        match &self.nodes[current] {
            // Leaf node, current slot is already filled by caller.
            Node::Leaf(_) => {}

            Node::Split(s) => {
                let cur = slots[current];
                let one = f16::ONE;
                match s.layout {
                    Layout::Horizontal => {
                        slots[s.left.idx()] = Slot {
                            x: cur.x,
                            y: cur.y,
                            width: cur.width * s.ratio,
                            height: cur.height,
                        };
                        slots[s.right.idx()] = Slot {
                            x: cur.x + cur.width * s.ratio,
                            y: cur.y,
                            width: cur.width * (one - s.ratio),
                            height: cur.height,
                        };
                    }

                    Layout::Vertical => {
                        slots[s.left.idx()] = Slot {
                            x: cur.x,
                            y: cur.y,
                            width: cur.width,
                            height: cur.height * s.ratio,
                        };
                        slots[s.right.idx()] = Slot {
                            x: cur.x,
                            y: cur.y + cur.height * s.ratio,
                            width: cur.width,
                            height: cur.height * (one - s.ratio),
                        };
                    }
                }

                self.fill_spatial_slots(slots, s.left);
                self.fill_spatial_slots(slots, s.right);
            }
        }
    }

    /// Get the dimensions of the tree starting from the given node.
    ///
    /// This creates relative dimensions (see Spatial) by assuming each
    /// leaf is exactly 1x1 unit in size. Returns `(width, height)`.
    fn dimensions(&self, current: Handle) -> (u16, u16) {
        match &self.nodes[current.idx()] {
            Node::Leaf(_) => (1, 1),
            Node::Split(s) => {
                let left = self.dimensions(s.left);
                let right = self.dimensions(s.right);
                match s.layout {
                    Layout::Horizontal => (left.0 + right.0, left.1.max(right.1)),
                    Layout::Vertical => (left.0.max(right.0), left.1 + right.1),
                }
            }
        }
    }
}

impl<V: SplitTreeView> SplitTree<V> {
    /// Initialize a new tree with a single view. The view is referenced
    /// (cloned) by the tree.
    #[must_use]
    pub fn new(view: &V) -> Self {
        Self {
            nodes: vec![Node::Leaf(view.clone())],
            zoomed: None,
        }
    }

    /// Insert another tree into this tree at the given node in the
    /// specified direction. The other tree will be inserted in the
    /// new direction. For example, if the direction is "right" then
    /// `insert` is inserted right of the existing node.
    ///
    /// The previous trees are not modified, but reference counts for the
    /// views will be increased accordingly for the new tree.
    pub fn split(
        &self,
        at: Handle,
        direction: Direction,
        ratio: f16,
        insert: &Self,
    ) -> Result<Self, SplitError> {
        // We know we're going to need the sum total of the nodes
        // between the two trees plus one for the new split node.
        let total = self.nodes.len() + insert.nodes.len() + 1;
        if total > usize::from(u16::MAX) {
            return Err(SplitError::OutOfMemory);
        }

        // We can copy our nodes exactly as they are, since they're
        // mostly not changing (only `at` is changing). We can copy the
        // destination nodes as well directly next to the source nodes. We
        // just have to go through and offset all the handles in the
        // destination tree to account for the shift.
        let mut nodes: Vec<Node<V>> = Vec::with_capacity(total);
        nodes.extend(self.nodes.iter().cloned());
        for node in &insert.nodes {
            nodes.push(match node {
                Node::Leaf(v) => Node::Leaf(v.clone()),
                Node::Split(s) => Node::Split(Split {
                    left: s.left.offset(self.nodes.len()),
                    right: s.right.offset(self.nodes.len()),
                    ..*s
                }),
            });
        }

        // Determine our split layout and if we're on the left
        let (layout, left) = match direction {
            Direction::Left => (Layout::Horizontal, true),
            Direction::Right => (Layout::Horizontal, false),
            Direction::Up => (Layout::Vertical, true),
            Direction::Down => (Layout::Vertical, false),
        };

        // Copy our previous value to the end of the nodes list and
        // create our new split node.
        let prev = nodes[at.idx()].clone();
        nodes.push(prev);
        let last = nodes.len() - 1;
        let (l, r) = if left {
            (self.nodes.len(), last)
        } else {
            (last, self.nodes.len())
        };
        nodes[at.idx()] = Node::Split(Split {
            layout,
            ratio,
            left: Handle::from_idx(l),
            right: Handle::from_idx(r),
        });

        Ok(Self {
            nodes,
            // Splitting always resets zoom state.
            zoomed: None,
        })
    }

    /// Remove a node from the tree.
    #[must_use]
    pub fn remove(&self, at: Handle) -> Self {
        assert!(at.idx() < self.nodes.len());

        // If we're removing node zero then we're clearing the tree.
        if at == Handle::ROOT {
            return Self::empty();
        }

        // Allocate our new nodes list with the number of nodes we'll
        // need after the removal.
        let count = self.count_after_removal(Handle::ROOT, at, 0);
        let mut new_nodes: Vec<Option<Node<V>>> = (0..count).map(|_| None).collect();
        let mut new_zoomed = None;

        // Traverse the tree and copy all our nodes into place.
        assert!(self.remove_node(&mut new_nodes, &mut new_zoomed, 0, Handle::ROOT, at) != 0);

        Self {
            nodes: new_nodes
                .into_iter()
                .map(|n| n.expect("all nodes filled by remove"))
                .collect(),
            zoomed: new_zoomed,
        }
    }

    fn remove_node(
        &self,
        new_nodes: &mut [Option<Node<V>>],
        new_zoomed: &mut Option<Handle>,
        new_offset: usize,
        current: Handle,
        target: Handle,
    ) -> usize {
        assert_ne!(current, target);

        // If we have a zoomed node and this is it then we migrate it.
        if let Some(v) = self.zoomed
            && v == current
        {
            assert!(new_zoomed.is_none());
            *new_zoomed = Some(Handle::from_idx(new_offset));
        }

        match &self.nodes[current.idx()] {
            // Leaf is simple, just copy it over (cloning is the ref).
            Node::Leaf(view) => {
                new_nodes[new_offset] = Some(Node::Leaf(view.clone()));
                1
            }

            Node::Split(s) => {
                // If we're removing one of the split node sides then
                // we remove the split node itself as well and only add
                // the other (non-removed) side.
                if s.left == target {
                    return self.remove_node(new_nodes, new_zoomed, new_offset, s.right, target);
                }
                if s.right == target {
                    return self.remove_node(new_nodes, new_zoomed, new_offset, s.left, target);
                }

                // Neither side is being directly removed, so we traverse.
                let left = self.remove_node(new_nodes, new_zoomed, new_offset + 1, s.left, target);
                assert!(left != 0);
                let right = self.remove_node(
                    new_nodes,
                    new_zoomed,
                    new_offset + left + 1,
                    s.right,
                    target,
                );
                assert!(right != 0);
                new_nodes[new_offset] = Some(Node::Split(Split {
                    layout: s.layout,
                    ratio: s.ratio,
                    left: Handle::from_idx(new_offset + 1),
                    right: Handle::from_idx(new_offset + 1 + left),
                }));

                left + right + 1
            }
        }
    }

    /// Equalize this node and all its children, returning a new node with splits
    /// adjusted so that each split's ratio is based on the relative weight
    /// (number of leaves) of its children.
    #[must_use]
    pub fn equalize(&self) -> Self {
        if self.is_empty() {
            return Self::empty();
        }

        // Copy the existing nodes (cloning the views).
        let mut nodes = self.nodes.clone();

        // Go through and equalize our ratios based on weights.
        for node in &mut nodes {
            if let Node::Split(s) = node {
                let weight_left = self.weight(s.left, s.layout, 0);
                let weight_right = self.weight(s.right, s.layout, 0);
                assert!(weight_left > 0);
                assert!(weight_right > 0);
                let total_f16 = f16_from_usize(weight_left + weight_right);
                let weight_left_f16 = f16_from_usize(weight_left);
                s.ratio = weight_left_f16 / total_f16;
            }
        }

        Self {
            nodes,
            zoomed: self.zoomed,
        }
    }

    /// Resize the nearest split matching the layout by the given ratio.
    /// Positive is right and down.
    ///
    /// The ratio is a signed delta representing the percentage to move
    /// the divider. The percentage is of the entire grid size, not just
    /// the specific split size.
    /// We use the entire grid size because that's what Ghostty's
    /// `resize_split` keybind does, because it maps to a general human
    /// understanding of moving a split relative to the entire window
    /// (generally).
    ///
    /// For example, a ratio of 0.1 and a layout of `vertical` will find
    /// the nearest vertical split and move the divider down by 10% of
    /// the total grid height.
    ///
    /// If no matching split is found, this does nothing, but will always
    /// still return a cloned tree.
    #[must_use]
    pub fn resize(&self, from: Handle, layout: Layout, ratio: f16) -> Self {
        assert!(ratio >= f16::from_f32(-1.0) && ratio <= f16::ONE);
        assert!(!ratio.is_nan());
        assert!(!ratio.is_infinite());

        // Fast path empty trees.
        if self.is_empty() {
            return Self::empty();
        }

        // From this point forward worst case we return a clone.
        let mut result = self.clone();

        // Find our nearest parent split node matching the layout.
        let parent_handle = match self.find_parent_split(layout, from, Handle::ROOT) {
            Backtrack::Deadend | Backtrack::Backtrack => return result,
            Backtrack::Result(v) => v,
        };

        // Get our spatial layout, because we need the dimensions of this
        // split with regards to the entire grid.
        let sp = result.spatial();

        // Get the ratio of the split relative to the full grid.
        let full_ratio = {
            // Our scale is the amount we need to multiply our individual
            // ratio by to get the full ratio. Its actually a ratio on its
            // own but I'm trying to avoid that word: its the ratio of
            // our spatial width/height to the total.
            let scale = match layout {
                Layout::Horizontal => sp.slots[parent_handle.idx()].width / sp.slots[0].width,
                Layout::Vertical => sp.slots[parent_handle.idx()].height / sp.slots[0].height,
            };

            let current = result
                .split_node(parent_handle)
                .expect("parent is a split")
                .ratio;
            current * scale
        };

        // Set the final new ratio, clamping it to [0, 1]
        result.resize_in_place(
            parent_handle,
            f16_min(f16_max(full_ratio + ratio, f16::ZERO), f16::ONE),
        );
        result
    }

    /// Format the tree in a human-readable format. By default this will
    /// output a diagram followed by a textual representation.
    fn format_all(&self, w: &mut dyn fmt::Write) -> fmt::Result {
        if self.nodes.is_empty() {
            return w.write_str("empty");
        }
        let _ = self.format_diagram(w);
        self.format_text(w)
    }

    /// Text representation of the tree.
    pub fn format_text(&self, w: &mut dyn fmt::Write) -> fmt::Result {
        if self.nodes.is_empty() {
            return w.write_str("empty");
        }
        self.format_text_inner(w, Handle::ROOT, 0)
    }

    fn format_text_inner(
        &self,
        w: &mut dyn fmt::Write,
        current: Handle,
        depth: usize,
    ) -> fmt::Result {
        for _ in 0..depth {
            w.write_str("  ")?;
        }

        if let Some(zoomed) = self.zoomed
            && zoomed == current
        {
            w.write_str("(zoomed) ")?;
        }

        match &self.nodes[current.idx()] {
            Node::Leaf(v) => match v.split_tree_label() {
                Some(label) => writeln!(w, "leaf: {label}"),
                None => writeln!(w, "leaf: {}", current.idx()),
            },

            Node::Split(s) => {
                let layout = match s.layout {
                    Layout::Horizontal => "horizontal",
                    Layout::Vertical => "vertical",
                };
                writeln!(
                    w,
                    "split (layout: {layout}, ratio: {:.2})",
                    s.ratio.to_f32()
                )?;
                self.format_text_inner(w, s.left, depth + 1)?;
                self.format_text_inner(w, s.right, depth + 1)
            }
        }
    }

    /// ASCII-art diagram of the tree.
    #[allow(clippy::too_many_lines)]
    pub fn format_diagram(&self, w: &mut dyn fmt::Write) -> fmt::Result {
        if self.nodes.is_empty() {
            return w.write_str("empty");
        }

        // Get our spatial representation.
        let sp = {
            let sp = self.spatial();

            // Scale our spatial representation to have minimum width/height 1.
            let mut min_w = f16::ONE;
            let mut min_h = f16::ONE;
            for slot in &sp.slots {
                if slot.width > f16::ZERO {
                    min_w = f16_min(min_w, slot.width);
                }
                if slot.height > f16::ZERO {
                    min_h = f16_min(min_h, slot.height);
                }
            }

            let ratio_w = f16::ONE / min_w;
            let ratio_h = f16::ONE / min_h;
            let mut slots = sp.slots.clone();
            for slot in &mut slots {
                slot.x *= ratio_w;
                slot.y *= ratio_h;
                slot.width *= ratio_w;
                slot.height *= ratio_h;
            }

            Spatial { slots }
        };

        let label_of = |handle: usize, node: &Node<V>| -> Option<String> {
            match node {
                Node::Split(_) => None,
                Node::Leaf(v) => Some(v.split_tree_label().unwrap_or_else(|| handle.to_string())),
            }
        };

        // The width we need for the largest label.
        let max_label_width: usize = {
            let mut max = 0;
            let mut any_label = false;
            for node in &self.nodes {
                if let Node::Leaf(v) = node
                    && let Some(label) = v.split_tree_label()
                {
                    any_label = true;
                    max = max.max(label.len());
                }
            }
            if any_label {
                max
            } else {
                (sp.slots.len().ilog10() + 1) as usize
            }
        };

        // We need space for whitespace and ASCII art so add that.
        // Border + whitespace + label + whitespace + border.
        let cell_width = 2 + max_label_width + 2;
        // Border + label + border. No whitespace needed on the vertical axis.
        let cell_height = 1 + 1 + 1;

        // Make a grid that can fit our entire ASCII diagram. We know
        // the width/height based on node 0.
        let mut grid: Vec<Vec<u8>> = {
            // Get our initial width/height. Each leaf is 1x1 in this.
            // We round up for this because partial widths/heights should
            // take up an extra cell.
            let mut width = sp.slots[0].width.to_f32().ceil() as usize;
            let mut height = sp.slots[0].height.to_f32().ceil() as usize;

            width *= cell_width;
            height *= cell_height;

            (0..height)
                .map(|_| {
                    let mut row = vec![b' '; width + 1];
                    row[width] = b'\n';
                    row
                })
                .collect()
        };

        // Draw each node
        for (handle, slot) in sp.slots.iter().enumerate() {
            // We only draw leaf nodes. Splits are only used for layout.
            let node = &self.nodes[handle];
            let Some(label) = label_of(handle, node) else {
                continue;
            };

            // If our width/height is zero then we skip this.
            if slot.width == f16::ZERO || slot.height == f16::ZERO {
                continue;
            }

            let mut x = slot.x.to_f32().floor() as usize;
            let mut y = slot.y.to_f32().floor() as usize;
            let mut width = slot.width.to_f32().floor().max(1.0) as usize;
            let mut height = slot.height.to_f32().floor().max(1.0) as usize;
            x *= cell_width;
            y *= cell_height;
            width *= cell_width;
            height *= cell_height;

            // Top border
            {
                let top = &mut grid[y][x..x + width];
                top[0] = b'+';
                for c in &mut top[1..width - 1] {
                    *c = b'-';
                }
                top[width - 1] = b'+';
            }

            // Bottom border
            {
                let bottom = &mut grid[y + height - 1][x..x + width];
                bottom[0] = b'+';
                for c in &mut bottom[1..width - 1] {
                    *c = b'-';
                }
                bottom[width - 1] = b'+';
            }

            // Left and right borders
            for row in &mut grid[y + 1..y + height - 1] {
                row[x] = b'|';
            }
            for row in &mut grid[y + 1..y + height - 1] {
                row[x + width - 1] = b'|';
            }

            // Draw the handle in the center
            let x_mid = width / 2 + x;
            let y_mid = height / 2 + y;
            let label_width = label.len();
            let label_start = x_mid - label_width / 2;
            let row = &mut grid[y_mid][label_start..];
            if label.len() > row.len() {
                return Err(fmt::Error);
            }
            row[..label.len()].copy_from_slice(label.as_bytes());
        }

        // Output every row
        for row in &grid {
            // We currently have a bug in our height calculation that
            // results in trailing blank lines. Ignore those. We should
            // really fix our height calculation instead. If someone wants
            // to do that just remove this line and see the tests that fail
            // and go from there.
            if row[0] == b' ' {
                break;
            }
            w.write_str(&String::from_utf8_lossy(row))?;
        }
        Ok(())
    }

    /// The diagram as a string (the Zig `{f}` of `std.fmt.alt(t, .formatDiagram)`).
    #[must_use]
    pub fn diagram_string(&self) -> String {
        let mut s = String::new();
        let _ = self.format_diagram(&mut s);
        s
    }

    /// The text representation as a string (`.formatText`).
    #[must_use]
    pub fn text_string(&self) -> String {
        let mut s = String::new();
        let _ = self.format_text(&mut s);
        s
    }
}

impl<V: SplitTreeView> fmt::Display for SplitTree<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        self.format_all(&mut s)?;
        f.write_str(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestView {
        label: &'static str,
    }

    impl SplitTreeView for TestView {
        fn split_tree_label(&self) -> Option<String> {
            Some(self.label.to_string())
        }
    }

    type TestTree = SplitTree<TestView>;

    fn v(label: &'static str) -> TestView {
        TestView { label }
    }

    fn h(f: f32) -> f16 {
        f16::from_f32(f)
    }

    fn find(t: &TestTree, label: &str) -> Handle {
        t.iterator()
            .find(|e| e.view.label == label)
            .expect("not found")
            .handle
    }

    fn leaf_label(t: &TestTree, handle: Handle) -> &'static str {
        match &t.nodes()[handle.idx()] {
            Node::Leaf(v) => v.label,
            Node::Split(_) => panic!("not a leaf"),
        }
    }

    #[test]
    fn split_tree_is_split() {
        // Empty tree should not be split
        let empty = TestTree::empty();
        assert!(!empty.is_split());

        // Single node tree should not be split
        let single = TestTree::new(&v("A"));
        assert!(!single.is_split());

        // Split tree should be split
        let tree2 = TestTree::new(&v("B"));
        let split = single
            .split(Handle::ROOT, Direction::Right, h(0.5), &tree2)
            .unwrap();
        assert!(split.is_split());
    }

    #[test]
    fn split_tree_empty_tree() {
        let t = TestTree::empty();
        assert_eq!(t.to_string(), "empty");
    }

    #[test]
    fn split_tree_single_node() {
        let t = TestTree::new(&v("A"));
        assert_eq!(t.diagram_string(), "+---+\n| A |\n+---+\n");
    }

    #[test]
    fn split_tree_split_horizontal() {
        let t1 = TestTree::new(&v("A"));
        let t2 = TestTree::new(&v("B"));
        let t3 = t1
            .split(Handle::ROOT, Direction::Right, h(0.5), &t2)
            .unwrap();

        assert_eq!(
            t3.to_string(),
            "+---++---+\n\
             | A || B |\n\
             +---++---+\n\
             split (layout: horizontal, ratio: 0.50)\n\
             \x20 leaf: A\n\
             \x20 leaf: B\n"
        );

        // Split right at B
        let tc = TestTree::new(&v("C"));
        let t4 = t3
            .split(find(&t3, "B"), Direction::Right, h(0.5), &tc)
            .unwrap();
        assert_eq!(
            t4.to_string(),
            "+--------++---++---+\n\
             |    A   || B || C |\n\
             +--------++---++---+\n\
             split (layout: horizontal, ratio: 0.50)\n\
             \x20 leaf: A\n\
             \x20 split (layout: horizontal, ratio: 0.50)\n\
             \x20   leaf: B\n\
             \x20   leaf: C\n"
        );

        // Split right at C
        let td = TestTree::new(&v("D"));
        let t5 = t4
            .split(find(&t4, "C"), Direction::Right, h(0.5), &td)
            .unwrap();
        assert_eq!(
            t5.to_string(),
            "+------------------++--------++---++---+\n\
             |         A        ||    B   || C || D |\n\
             +------------------++--------++---++---+\n\
             split (layout: horizontal, ratio: 0.50)\n\
             \x20 leaf: A\n\
             \x20 split (layout: horizontal, ratio: 0.50)\n\
             \x20   leaf: B\n\
             \x20   split (layout: horizontal, ratio: 0.50)\n\
             \x20     leaf: C\n\
             \x20     leaf: D\n"
        );

        // Find "previous" from D back.
        let mut current = b'D';
        while current != b'A' {
            let label = (current as char).to_string();
            let handle = t5.previous(find(&t5, &label)).unwrap();
            assert_eq!(leaf_label(&t5, handle).as_bytes(), &[current - 1]);
            current -= 1;
        }
        assert!(t5.previous(find(&t5, "A")).is_none());

        // Find "next" from A forward.
        let mut current = b'A';
        while current != b'D' {
            let label = (current as char).to_string();
            let handle = t5.next(find(&t5, &label)).unwrap();
            assert_eq!(leaf_label(&t5, handle).as_bytes(), &[current + 1]);
            current += 1;
        }
        assert!(t5.next(find(&t5, "D")).is_none());
    }

    #[test]
    fn split_tree_split_vertical() {
        let t1 = TestTree::new(&v("A"));
        let t2 = TestTree::new(&v("B"));
        let t3 = t1
            .split(Handle::ROOT, Direction::Down, h(0.5), &t2)
            .unwrap();
        assert_eq!(
            t3.diagram_string(),
            "+---+\n| A |\n+---+\n+---+\n| B |\n+---+\n"
        );
    }

    fn ab_split(dir: Direction, ratio: f32) -> TestTree {
        let t1 = TestTree::new(&v("A"));
        let t2 = TestTree::new(&v("B"));
        t1.split(Handle::ROOT, dir, h(ratio), &t2).unwrap()
    }

    #[test]
    fn split_tree_split_horizontal_with_zero_ratio() {
        assert_eq!(
            ab_split(Direction::Right, 0.0).diagram_string(),
            "+---+\n| B |\n+---+\n"
        );
    }

    #[test]
    fn split_tree_split_vertical_with_zero_ratio() {
        assert_eq!(
            ab_split(Direction::Down, 0.0).diagram_string(),
            "+---+\n| B |\n+---+\n"
        );
    }

    #[test]
    fn split_tree_split_horizontal_with_full_width() {
        assert_eq!(
            ab_split(Direction::Right, 1.0).diagram_string(),
            "+---+\n| A |\n+---+\n"
        );
    }

    #[test]
    fn split_tree_split_vertical_with_full_width() {
        assert_eq!(
            ab_split(Direction::Down, 1.0).diagram_string(),
            "+---+\n| A |\n+---+\n"
        );
    }

    #[test]
    fn split_tree_remove_leaf() {
        let t3 = ab_split(Direction::Right, 0.5);
        let t4 = t3.remove(find(&t3, "A"));
        assert_eq!(t4.diagram_string(), "+---+\n| B |\n+---+\n");
    }

    #[test]
    fn split_tree_split_twice_remove_intermediary() {
        let t3 = TestTree::new(&v("C"));

        // A | B horizontal.
        let split1 = ab_split(Direction::Right, 0.5);

        // Insert C below that.
        let split2 = split1
            .split(Handle::ROOT, Direction::Down, h(0.5), &t3)
            .unwrap();
        assert_eq!(
            split2.diagram_string(),
            "+---++---+\n\
             | A || B |\n\
             +---++---+\n\
             +--------+\n\
             |    C   |\n\
             +--------+\n"
        );

        // Remove "B"
        let split3 = split2.remove(find(&split2, "B"));
        assert_eq!(
            split3.diagram_string(),
            "+---+\n| A |\n+---+\n+---+\n| C |\n+---+\n"
        );

        // Remove every node from split2 (our most complex one), which should
        // never crash. We don't test the result is correct, this just verifies
        // we don't hit any assertion failures.
        for i in 0..split2.nodes().len() {
            let _ = split2.remove(Handle::from_idx(i));
        }
    }

    #[test]
    fn split_tree_spatial_goto() {
        let t1 = TestTree::new(&v("A"));
        let t2 = TestTree::new(&v("B"));
        let t3 = TestTree::new(&v("C"));
        let t4 = TestTree::new(&v("D"));

        // A | B horizontal
        let split_ab = t1
            .split(Handle::ROOT, Direction::Right, h(0.5), &t2)
            .unwrap();

        // A | C vertical
        let split_ac = split_ab
            .split(find(&split_ab, "A"), Direction::Down, h(0.8), &t3)
            .unwrap();

        // B | D vertical
        let split = split_ac
            .split(find(&split_ab, "B"), Direction::Down, h(0.3), &t4)
            .unwrap();

        assert_eq!(
            split.diagram_string(),
            "+---++---+\n\
             |   || B |\n\
             |   |+---+\n\
             |   |+---+\n\
             | A ||   |\n\
             |   ||   |\n\
             |   ||   |\n\
             |   || D |\n\
             +---+|   |\n\
             +---+|   |\n\
             | C ||   |\n\
             +---++---+\n"
        );

        let go = |from: &str, dir: Direction| {
            let target = split
                .goto(find(&split, from), Goto::Spatial(dir))
                .expect("target");
            leaf_label(&split, target)
        };

        // Spatial C => right
        assert_eq!(go("C", Direction::Right), "D");
        // Spatial D => left
        assert_eq!(go("D", Direction::Left), "A");
        // Spatial A => left (wrapped)
        assert_eq!(go("A", Direction::Left), "B");
        // Spatial B => right (wrapped)
        assert_eq!(go("B", Direction::Right), "A");
        // Spatial C => down (wrapped)
        assert_eq!(go("C", Direction::Down), "A");

        // Equalize
        let equal = split.equalize();
        assert_eq!(
            equal.diagram_string(),
            "+---++---+\n\
             | A || B |\n\
             +---++---+\n\
             +---++---+\n\
             | C || D |\n\
             +---++---+\n"
        );
    }

    #[test]
    fn split_tree_resize() {
        let split = ab_split(Direction::Right, 0.5);
        assert_eq!(
            split.diagram_string(),
            "+---++---+\n| A || B |\n+---++---+\n"
        );

        // Resize
        {
            let resized = split.resize(find(&split, "B"), Layout::Horizontal, h(0.25));
            assert_eq!(
                resized.diagram_string(),
                "+-------------++---+\n\
                 |      A      || B |\n\
                 +-------------++---+\n"
            );
        }

        // Resize the other direction (negative ratio)
        {
            let resized = split.resize(find(&split, "B"), Layout::Horizontal, h(-0.25));
            assert_eq!(
                resized.diagram_string(),
                "+---++-------------+\n\
                 | A ||      B      |\n\
                 +---++-------------+\n"
            );
        }
    }

    #[test]
    fn split_tree_clone_empty_tree() {
        let t = TestTree::empty();
        let t2 = t.clone();
        assert_eq!(t2.to_string(), "empty");
    }

    #[test]
    fn split_tree_zoom() {
        let mut split = ab_split(Direction::Right, 0.5);
        let b = find(&split, "B");
        split.zoom(Some(b));

        let expected = "split (layout: horizontal, ratio: 0.50)\n\
                        \x20 leaf: A\n\
                        \x20 (zoomed) leaf: B\n";
        assert_eq!(split.text_string(), expected);

        // Clone preserves zoom
        let clone = split.clone();
        assert_eq!(clone.text_string(), expected);
    }

    #[test]
    fn split_tree_split_resets_zoom() {
        let mut t1 = TestTree::new(&v("A"));
        let t2 = TestTree::new(&v("B"));

        // Zoom A
        let a = find(&t1, "A");
        t1.zoom(Some(a));

        let split = t1
            .split(Handle::ROOT, Direction::Right, h(0.5), &t2)
            .unwrap();
        assert_eq!(
            split.text_string(),
            "split (layout: horizontal, ratio: 0.50)\n\
             \x20 leaf: A\n\
             \x20 leaf: B\n"
        );
    }

    #[test]
    fn split_tree_remove_and_zoom() {
        let mut split = ab_split(Direction::Right, 0.5);
        let a = find(&split, "A");
        split.zoom(Some(a));

        // Remove A, should unzoom
        {
            let removed = split.remove(find(&split, "A"));
            assert!(removed.zoomed().is_none());
            assert_eq!(removed.text_string(), "leaf: B\n");
        }

        // Remove B, should keep zoom
        {
            let removed = split.remove(find(&split, "B"));
            assert_eq!(removed.text_string(), "(zoomed) leaf: A\n");
        }
    }

    // Extra tests beyond the Zig suite.

    #[test]
    fn split_tree_goto_linear() {
        let mut t = TestTree::new(&v("A"));
        for l in ["B", "C"] {
            let n = TestTree::new(&v(l));
            let at = find(&t, if l == "B" { "A" } else { "B" });
            t = t.split(at, Direction::Right, h(0.5), &n).unwrap();
        }
        let a = find(&t, "A");
        let c = find(&t, "C");
        assert_eq!(t.goto(a, Goto::Previous), None);
        assert_eq!(t.goto(c, Goto::Next), None);
        assert_eq!(t.goto(a, Goto::PreviousWrapped), Some(c));
        assert_eq!(t.goto(c, Goto::NextWrapped), Some(a));
        assert_eq!(leaf_label(&t, t.goto(a, Goto::Next).unwrap()), "B");
    }

    #[test]
    fn split_tree_split_too_many_nodes() {
        // 2^15 each would exceed u16 handles.
        let big = SplitTree {
            nodes: vec![Node::Leaf(v("X")); 40_000],
            zoomed: None,
        };
        let err = big
            .split(Handle::ROOT, Direction::Right, h(0.5), &big)
            .unwrap_err();
        assert_eq!(err, SplitError::OutOfMemory);
    }

    #[test]
    fn split_tree_default_label_uses_handle() {
        #[derive(Clone)]
        struct Plain;
        impl SplitTreeView for Plain {}
        let a = SplitTree::new(&Plain);
        let b = SplitTree::new(&Plain);
        let s = a.split(Handle::ROOT, Direction::Right, h(0.5), &b).unwrap();
        assert_eq!(
            s.to_string(),
            "+---++---+\n\
             | 2 || 1 |\n\
             +---++---+\n\
             split (layout: horizontal, ratio: 0.50)\n\
             \x20 leaf: 2\n\
             \x20 leaf: 1\n"
        );
    }

    #[test]
    fn split_tree_ref_counts_follow_clones() {
        use std::rc::Rc;
        #[derive(Clone)]
        #[allow(dead_code)]
        struct Rcv(Rc<()>);
        impl SplitTreeView for Rcv {}
        let rc = Rc::new(());
        let a = SplitTree::new(&Rcv(rc.clone()));
        assert_eq!(Rc::strong_count(&rc), 2);
        let b = SplitTree::new(&Rcv(rc.clone()));
        let s = a.split(Handle::ROOT, Direction::Right, h(0.5), &b).unwrap();
        assert_eq!(Rc::strong_count(&rc), 5);
        drop(a);
        drop(b);
        assert_eq!(Rc::strong_count(&rc), 3);
        drop(s);
        assert_eq!(Rc::strong_count(&rc), 1);
    }
}
