//! Compact cells and rows, with bounded owned page storage.
use std::collections::{HashMap, HashSet};

use ghostty_vt::color::Rgb;

use crate::{
    hyperlink::{self, Hyperlink, HyperlinkId},
    ref_counted_set::{AddError, Layout as SetLayout},
    style::{self, Style},
};
/// Default grapheme allocation request, in bytes.
pub const GRAPHEME_BYTES_DEFAULT: u32 = 1024;
/// Default string allocation request, in bytes.
pub const STRING_BYTES_DEFAULT: u32 = 2048;
/// Reference ABI size of a hyperlink set item.
pub const HYPERLINK_ITEM_SIZE: usize = 48;
/// Reference ABI size of a style set item.
pub const STYLE_ITEM_SIZE: usize = 36;
/// The standard page capacity (smaller grapheme region in tests, as in Zig).
pub const STD_CAPACITY: Capacity = Capacity {
    cols: 215,
    rows: 215,
    styles: 128,
    hyperlink_bytes: 192,
    grapheme_bytes: if cfg!(test) { 512 } else { 8192 },
    string_bytes: STRING_BYTES_DEFAULT,
};
/// ContentTag field of the packed grid representation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ContentTag {
    /// Single codepoint.
    #[default]
    Codepoint = 0,
    /// First codepoint of a grapheme cluster.
    CodepointGrapheme = 1,
    /// Background-only palette color.
    BgColorPalette = 2,
    /// Background-only RGB color.
    BgColorRgb = 3,
}
/// Wide field of the packed grid representation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Wide {
    /// Single grid cell.
    #[default]
    Narrow = 0,
    /// Two-cell character.
    Wide = 1,
    /// Trailing half of a wide character.
    SpacerTail = 2,
    /// End-of-line spacer before a wrapped wide character.
    SpacerHead = 3,
}
/// SemanticContent field of the packed grid representation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SemanticContent {
    /// Ordinary command output.
    #[default]
    Output = 0,
    /// User input.
    Input = 1,
    /// Application prompt.
    Prompt = 2,
}
/// SemanticPrompt field of the packed grid representation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SemanticPrompt {
    /// No prompt cells.
    #[default]
    None = 0,
    /// Primary prompt row.
    Prompt = 1,
    /// Continuation prompt row.
    PromptContinuation = 2,
}
/// Eight-byte terminal cell; all-zero bits describe an empty cell.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, bytemuck::Zeroable)]
#[repr(transparent)]
pub struct Cell(u64);
impl Cell {
    /// Construct an unstyled single-codepoint cell.
    pub fn init(cp: u32) -> Self {
        let mut c = Self::default();
        c.set_codepoint(cp);
        c
    }
    fn bits(self, shift: u32, mask: u64) -> u64 {
        (self.0 >> shift) & mask
    }
    fn set_bits(&mut self, shift: u32, mask: u64, value: u64) {
        self.0 = (self.0 & !(mask << shift)) | ((value & mask) << shift);
    }
    /// Return the bit-packed representation.
    pub const fn raw(self) -> u64 {
        self.0
    }
    /// True when every bit is zero.
    pub fn is_zero(self) -> bool {
        self.0 == 0
    }
    /// Return the codepoint, or zero for background-only cells.
    pub fn codepoint(self) -> u32 {
        if (self.content_tag() as u8) < 2 {
            self.bits(2, 0x1fffff) as u32
        } else {
            0
        }
    }
    /// Set the codepoint payload without changing the content tag.
    pub fn set_codepoint(&mut self, cp: u32) {
        self.set_bits(2, 0xffffff, u64::from(cp & 0x1fffff));
    }
    /// Return the palette payload.
    pub fn color_palette(self) -> u8 {
        self.bits(2, 255) as u8
    }
    /// Set the palette payload without changing the content tag.
    pub fn set_color_palette(&mut self, v: u8) {
        self.set_bits(2, 0xffffff, u64::from(v));
    }
    /// Return the RGB payload.
    pub fn color_rgb(self) -> Rgb {
        Rgb {
            r: self.bits(2, 255) as u8,
            g: self.bits(10, 255) as u8,
            b: self.bits(18, 255) as u8,
        }
    }
    /// Set the RGB payload without changing the content tag.
    pub fn set_color_rgb(&mut self, v: Rgb) {
        self.set_bits(
            2,
            0xffffff,
            u64::from(v.r) | (u64::from(v.g) << 8) | (u64::from(v.b) << 16),
        );
    }
    /// True when the cell contains a nonzero text codepoint.
    pub fn has_text(self) -> bool {
        self.codepoint() != 0
    }
    /// True when the cell refers to extra grapheme codepoints.
    pub fn has_grapheme(self) -> bool {
        self.content_tag() == ContentTag::CodepointGrapheme
    }
    /// Width in grid cells, including one-cell spacers.
    pub fn grid_width(self) -> u8 {
        if self.wide() == Wide::Wide { 2 } else { 1 }
    }
    /// True when the cell has a nondefault style ID.
    pub fn has_styling(self) -> bool {
        self.style_id() != 0
    }
    /// True if the cell is a plain single-codepoint cell with the default
    /// style (content tag `Codepoint` and style id 0): a single mask test.
    #[inline(always)]
    pub fn is_plain(self) -> bool {
        self.0 & (3 | (0xFFFF << 26)) == 0
    }
    /// True for a blank narrow textual cell; style does not affect this test.
    pub fn is_empty(self) -> bool {
        (self.content_tag() as u8) < 2 && !self.has_text() && self.wide() == Wide::Narrow
    }
    /// True if any cell has text.
    pub fn has_text_any(cells: &[Self]) -> bool {
        cells.iter().any(|c| c.has_text())
    }
    /// Read the content_tag field.
    pub fn content_tag(self) -> ContentTag {
        match self.bits(0, 3) {
            0 => ContentTag::Codepoint,
            1 => ContentTag::CodepointGrapheme,
            2 => ContentTag::BgColorPalette,
            _ => ContentTag::BgColorRgb,
        }
    }
    /// Update the content_tag field.
    pub fn set_content_tag(&mut self, v: ContentTag) {
        self.set_bits(0, 3, v as u64);
    }
    /// Read the style_id field.
    pub fn style_id(self) -> u16 {
        self.bits(26, 65535) as u16
    }
    /// Update the style_id field.
    pub fn set_style_id(&mut self, v: u16) {
        self.set_bits(26, 65535, v as u64);
    }
    /// Read the wide field.
    pub fn wide(self) -> Wide {
        match self.bits(42, 3) {
            0 => Wide::Narrow,
            1 => Wide::Wide,
            2 => Wide::SpacerTail,
            _ => Wide::SpacerHead,
        }
    }
    /// Update the wide field.
    pub fn set_wide(&mut self, v: Wide) {
        self.set_bits(42, 3, v as u64);
    }
    /// Read the protected field.
    pub fn protected(self) -> bool {
        self.bits(44, 1) != 0
    }
    /// Update the protected field.
    pub fn set_protected(&mut self, v: bool) {
        self.set_bits(44, 1, v as u64);
    }
    /// Read the hyperlink field.
    pub fn hyperlink(self) -> bool {
        self.bits(45, 1) != 0
    }
    /// Update the hyperlink field.
    pub fn set_hyperlink(&mut self, v: bool) {
        self.set_bits(45, 1, v as u64);
    }
    /// Read the semantic_content field.
    pub fn semantic_content(self) -> SemanticContent {
        match self.bits(46, 3) {
            0 => SemanticContent::Output,
            1 => SemanticContent::Input,
            _ => SemanticContent::Prompt,
        }
    }
    /// Update the semantic_content field.
    pub fn set_semantic_content(&mut self, v: SemanticContent) {
        self.set_bits(46, 3, v as u64);
    }
}
/// Eight-byte row metadata. The low 32 bits hold its first cell index.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Row(u64);
impl Row {
    /// First cell index in the page's row-major storage.
    pub fn cells(self) -> usize {
        (self.0 & 0xffffffff) as usize
    }
    /// Set the row's first cell index.
    pub fn set_cells(&mut self, v: usize) {
        self.0 = (self.0 & !0xffffffff) | (v as u64 & 0xffffffff);
    }
    /// True if cells can have managed style, hyperlink, or grapheme data.
    pub fn managed_memory(self) -> bool {
        self.styled() || self.hyperlink() || self.grapheme()
    }
    /// Prompt-row classification.
    pub fn semantic_prompt(self) -> SemanticPrompt {
        match (self.0 >> 37) & 3 {
            1 => SemanticPrompt::Prompt,
            2 => SemanticPrompt::PromptContinuation,
            _ => SemanticPrompt::None,
        }
    }
    /// Set prompt-row classification.
    pub fn set_semantic_prompt(&mut self, v: SemanticPrompt) {
        self.0 = (self.0 & !(3 << 37)) | ((v as u64) << 37);
    }
    /// Read the wrap flag.
    pub fn wrap(self) -> bool {
        self.0 & (1 << 32) != 0
    }
    /// Update the wrap flag.
    pub fn set_wrap(&mut self, v: bool) {
        self.0 = (self.0 & !(1 << 32)) | (u64::from(v) << 32);
    }
    /// Read the wrap_continuation flag.
    pub fn wrap_continuation(self) -> bool {
        self.0 & (1 << 33) != 0
    }
    /// Update the wrap_continuation flag.
    pub fn set_wrap_continuation(&mut self, v: bool) {
        self.0 = (self.0 & !(1 << 33)) | (u64::from(v) << 33);
    }
    /// Read the grapheme flag.
    pub fn grapheme(self) -> bool {
        self.0 & (1 << 34) != 0
    }
    /// Update the grapheme flag.
    pub fn set_grapheme(&mut self, v: bool) {
        self.0 = (self.0 & !(1 << 34)) | (u64::from(v) << 34);
    }
    /// Read the styled flag.
    pub fn styled(self) -> bool {
        self.0 & (1 << 35) != 0
    }
    /// Update the styled flag.
    pub fn set_styled(&mut self, v: bool) {
        self.0 = (self.0 & !(1 << 35)) | (u64::from(v) << 35);
    }
    /// Read the hyperlink flag.
    pub fn hyperlink(self) -> bool {
        self.0 & (1 << 36) != 0
    }
    /// Update the hyperlink flag.
    pub fn set_hyperlink(&mut self, v: bool) {
        self.0 = (self.0 & !(1 << 36)) | (u64::from(v) << 36);
    }
    /// Read the kitty_virtual_placeholder flag.
    pub fn kitty_virtual_placeholder(self) -> bool {
        self.0 & (1 << 39) != 0
    }
    /// Update the kitty_virtual_placeholder flag.
    pub fn set_kitty_virtual_placeholder(&mut self, v: bool) {
        self.0 = (self.0 & !(1 << 39)) | (u64::from(v) << 39);
    }
    /// Read the dirty flag.
    pub fn dirty(self) -> bool {
        self.0 & (1 << 40) != 0
    }
    /// Update the dirty flag.
    pub fn set_dirty(&mut self, v: bool) {
        self.0 = (self.0 & !(1 << 40)) | (u64::from(v) << 40);
    }
}
/// Current logical page dimensions.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    /// Columns currently addressable.
    pub cols: u16,
    /// Rows currently addressable.
    pub rows: u16,
}
/// Fixed logical allocation limits for a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    /// Allocated columns.
    pub cols: u16,
    /// Allocated rows.
    pub rows: u16,
    /// Requested style hash-table slots.
    pub styles: u16,
    /// Requested hyperlink-set bytes, excluding strings and map.
    pub hyperlink_bytes: u16,
    /// Requested grapheme bytes.
    pub grapheme_bytes: u32,
    /// Requested string bytes.
    pub string_bytes: u32,
}
impl Default for Capacity {
    fn default() -> Self {
        Self {
            cols: 0,
            rows: 0,
            styles: 16,
            hyperlink_bytes: 192,
            grapheme_bytes: GRAPHEME_BYTES_DEFAULT,
            string_bytes: STRING_BYTES_DEFAULT,
        }
    }
}
/// Capacity adjustment that retains the original total byte allocation.
#[derive(Debug, Default, Clone, Copy)]
pub struct Adjustment {
    /// New column count; rows absorb the size change.
    pub cols: Option<u16>,
}
impl Capacity {
    fn available_bytes_for_grid(self) -> usize {
        let l = Page::layout(self);
        let mut end = l.total_size;
        for (bytes, align) in [
            (l.hyperlink_map_layout.total_size, 4),
            (l.hyperlink_set_layout.total_size, 8),
            (l.string_alloc_layout.total_size, 8),
            (l.grapheme_map_layout.total_size, 8),
            (l.grapheme_alloc_layout.total_size, 8),
            (l.styles_layout.total_size, 8),
        ] {
            end = (end - bytes) & !(align - 1);
        }
        end
    }
    /// Maximum columns that still allow one row without enlarging the page.
    pub fn max_cols(self) -> Option<u16> {
        let bytes = self.available_bytes_for_grid();
        (bytes > 8).then(|| ((bytes - 8) / 8).min(65535) as u16)
    }
    /// Adjust columns, retaining the same page size by changing row capacity.
    pub fn adjust(self, req: Adjustment) -> Result<Self, Error> {
        let Some(cols) = req.cols else {
            return Ok(self);
        };
        let rows = self.available_bytes_for_grid() / (8 + usize::from(cols) * 8);
        if rows == 0 {
            return Err(Error::OutOfMemory);
        }
        Ok(Self {
            cols,
            rows: rows.min(65535) as u16,
            ..self
        })
    }
}
/// Logical bitmap allocator layout, preserving the reference size formula.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllocLayout {
    /// Total bytes including the reference allocator's chunk multiplier.
    pub total_size: usize,
    /// Number of 64-bit chunk bitmaps.
    pub bitmap_count: usize,
    /// Start of allocation data.
    pub chunks_start: usize,
}
impl AllocLayout {
    fn new(cap: usize, chunk: usize) -> Self {
        let aligned = cap.next_multiple_of(chunk);
        let bitmap_count = (aligned / chunk).div_ceil(64);
        Self {
            total_size: bitmap_count * 8 + aligned * chunk,
            bitmap_count,
            chunks_start: bitmap_count * 8,
        }
    }
}
/// Logical offset hash-map layout.
#[derive(Debug, Default, Clone, Copy)]
pub struct MapLayout {
    /// Total bytes including the 16-byte header.
    pub total_size: usize,
    /// Maximum entries (no extra load-factor reduction).
    pub capacity: usize,
}
impl MapLayout {
    fn new(capacity: usize, value_size: usize, value_align: usize) -> Self {
        let keys_start = (16 + capacity).next_multiple_of(4);
        let vals_start = (keys_start + capacity * 4).next_multiple_of(value_align);
        Self {
            total_size: (vals_start + capacity * value_size).next_multiple_of(value_align.max(4)),
            capacity,
        }
    }
}
/// Complete logical page layout for the 64-bit Linux reference ABI.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// Page-aligned byte size.
    pub total_size: usize,
    /// Row array offset.
    pub rows_start: usize,
    /// Row array bytes.
    pub rows_size: usize,
    /// Cell array offset.
    pub cells_start: usize,
    /// Cell array bytes.
    pub cells_size: usize,
    /// Style set offset.
    pub styles_start: usize,
    /// Style set layout.
    pub styles_layout: SetLayout,
    /// Grapheme allocator offset.
    pub grapheme_alloc_start: usize,
    /// Grapheme allocator layout.
    pub grapheme_alloc_layout: AllocLayout,
    /// Grapheme map offset.
    pub grapheme_map_start: usize,
    /// Grapheme map layout.
    pub grapheme_map_layout: MapLayout,
    /// String allocator offset.
    pub string_alloc_start: usize,
    /// String allocator layout.
    pub string_alloc_layout: AllocLayout,
    /// Hyperlink set offset.
    pub hyperlink_set_start: usize,
    /// Hyperlink set layout.
    pub hyperlink_set_layout: SetLayout,
    /// Hyperlink map offset.
    pub hyperlink_map_start: usize,
    /// Hyperlink map layout.
    pub hyperlink_map_layout: MapLayout,
    /// Requested capacity.
    pub capacity: Capacity,
}
/// Recoverable page operation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Allocation or grid capacity exhausted.
    OutOfMemory,
    /// Cell or row is outside the logical page.
    InvalidCoordinate,
    /// A grid dimension is zero.
    InvalidDimensions,
    /// Style set needs more space.
    StyleSetOutOfMemory,
    /// Style IDs need compaction.
    StyleSetNeedsRehash,
    /// No contiguous grapheme allocation fits.
    GraphemeAllocOutOfMemory,
    /// Grapheme map capacity exhausted.
    GraphemeMapOutOfMemory,
    /// Hyperlink map capacity exhausted.
    HyperlinkMapOutOfMemory,
    /// Hyperlink set capacity exhausted.
    SetOutOfMemory,
    /// Hyperlink IDs need compaction.
    SetNeedsRehash,
    /// No contiguous string allocation fits.
    StringsOutOfMemory,
    /// Missing style or hyperlink ID.
    InvalidId,
    /// Reference count overflow or underflow.
    InvalidRefCount,
    /// Capacity cannot grow further.
    OutOfSpace,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<AddError> for Error {
    fn from(e: AddError) -> Self {
        match e {
            AddError::OutOfMemory => Self::StyleSetOutOfMemory,
            AddError::NeedsRehash => Self::StyleSetNeedsRehash,
            AddError::InvalidId => Self::InvalidId,
            AddError::InvalidRefCount => Self::InvalidRefCount,
        }
    }
}
/// Hasher for cell-index keys: indices are already well distributed, so a
/// single multiply replaces SipHash on the print path.
#[derive(Debug, Default, Clone, Copy)]
struct IndexHasher(u64);
impl std::hash::Hasher for IndexHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ u64::from(b)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }
    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.0 = (n as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}
type IndexMap<V> = HashMap<usize, V, std::hash::BuildHasherDefault<IndexHasher>>;
#[derive(Debug, Clone, Copy)]
struct Allocation {
    start: usize,
    len: usize,
}
#[derive(Debug, Clone)]
struct ChunkAlloc {
    /// Bitmap of used chunks (bit set = used), 64 chunks per word.
    used: Vec<u64>,
    /// Total number of chunks.
    chunks: usize,
    /// Every word below this index is completely used.
    first_free_word: usize,
    chunk: usize,
}
impl ChunkAlloc {
    fn new(layout: AllocLayout, chunk: usize) -> Result<Self, Error> {
        let mut used = Vec::new();
        used.try_reserve_exact(layout.bitmap_count)
            .map_err(|_| Error::OutOfMemory)?;
        used.resize(layout.bitmap_count, 0);
        Ok(Self {
            used,
            chunks: layout.bitmap_count * 64,
            first_free_word: 0,
            chunk,
        })
    }
    /// First-fit allocation of `bytes` (rounded up to whole chunks): the
    /// lowest-addressed run of free chunks is used.
    fn alloc(&mut self, bytes: usize) -> Option<Allocation> {
        let n = bytes.div_ceil(self.chunk);
        if n == 0 {
            return Some(Allocation { start: 0, len: 0 });
        }
        while self.first_free_word < self.used.len() && self.used[self.first_free_word] == u64::MAX
        {
            self.first_free_word += 1;
        }
        let mut run = 0usize;
        let mut run_start = 0usize;
        for w in self.first_free_word..self.used.len() {
            let word = self.used[w];
            if word == u64::MAX {
                run = 0;
                continue;
            }
            if word == 0 {
                if run == 0 {
                    run_start = w * 64;
                }
                run += 64;
                if run >= n {
                    return Some(self.take(run_start, n));
                }
                continue;
            }
            if n == 1 {
                let b = word.trailing_ones() as usize;
                return Some(self.take(w * 64 + b, 1));
            }
            for b in 0..64 {
                if word >> b & 1 == 1 {
                    run = 0;
                } else {
                    if run == 0 {
                        run_start = w * 64 + b;
                    }
                    run += 1;
                    if run == n {
                        return Some(self.take(run_start, n));
                    }
                }
            }
        }
        None
    }
    fn take(&mut self, start: usize, len: usize) -> Allocation {
        for i in start..start + len {
            self.used[i / 64] |= 1 << (i % 64);
        }
        Allocation { start, len }
    }
    fn free(&mut self, a: Allocation) {
        for i in a.start..a.start + a.len {
            self.used[i / 64] &= !(1 << (i % 64));
        }
        self.first_free_word = self.first_free_word.min(a.start / 64);
    }
    fn reset(&mut self) {
        self.used.fill(0);
        self.first_free_word = 0;
    }
}
/// Page-owned rows, cells, and bounded auxiliary storage.
#[derive(Debug, Clone)]
pub struct Page {
    /// Logical dimensions; no greater than capacity.
    pub size: Size,
    /// Fixed logical allocation limits.
    pub capacity: Capacity,
    /// A full-page operation dirtied every row.
    pub dirty: bool,
    /// Row metadata in screen order.
    pub rows: Vec<Row>,
    /// Cells in row-major capacity order.
    pub cells: Vec<Cell>,
    /// Shared nondefault cell styles.
    pub styles: style::Set,
    /// Shared hyperlink values.
    pub hyperlink_set: hyperlink::Set,
    // Graphemes share a page arena; copying a cell never allocates a Vec.
    grapheme_map: IndexMap<std::ops::Range<usize>>,
    grapheme_data: Vec<u32>,
    grapheme_allocations: IndexMap<Allocation>,
    grapheme_alloc: ChunkAlloc,
    hyperlink_map: hyperlink::Map,
    string_alloc: ChunkAlloc,
    link_allocations: HashMap<u16, [Option<Allocation>; 2]>,
    pause_integrity_checks: usize,
}
/// A cell with its page-independent managed values, used when copying pages.
#[derive(Debug, Clone)]
pub(crate) struct OwnedCell {
    pub cell: Cell,
    pub style: Option<Style>,
    pub grapheme: Option<Vec<u32>>,
    pub hyperlink: Option<Hyperlink>,
}
impl Page {
    /// Compute the reference Zig layout without allocating memory.
    pub fn layout(cap: Capacity) -> Layout {
        let rows_start = 0;
        let rows_size = usize::from(cap.rows) * 8;
        let cells_start = rows_size;
        let cells_size = usize::from(cap.rows) * usize::from(cap.cols) * 8;
        let styles_start = (cells_start + cells_size).next_multiple_of(8);
        let styles_layout = SetLayout::init(usize::from(cap.styles), STYLE_ITEM_SIZE, 4);
        let grapheme_alloc_start = (styles_start + styles_layout.total_size).next_multiple_of(8);
        let grapheme_alloc_layout = AllocLayout::new(cap.grapheme_bytes as usize, 16);
        let grapheme_map_start =
            (grapheme_alloc_start + grapheme_alloc_layout.total_size).next_multiple_of(8);
        let gc = if cap.grapheme_bytes == 0 {
            0
        } else {
            (cap.grapheme_bytes as usize)
                .div_ceil(16)
                .next_power_of_two()
        };
        let grapheme_map_layout = MapLayout::new(gc, 16, 8);
        let string_alloc_start =
            (grapheme_map_start + grapheme_map_layout.total_size).next_multiple_of(8);
        let string_alloc_layout = AllocLayout::new(cap.string_bytes as usize, 32);
        let hyperlink_set_start =
            (string_alloc_start + string_alloc_layout.total_size).next_multiple_of(8);
        let hc = usize::from(cap.hyperlink_bytes) / HYPERLINK_ITEM_SIZE;
        let hyperlink_set_layout = SetLayout::init(hc, HYPERLINK_ITEM_SIZE, 8);
        let hyperlink_map_start =
            (hyperlink_set_start + hyperlink_set_layout.total_size).next_multiple_of(4);
        let hyperlink_map_layout = MapLayout::new(
            if hc == 0 {
                0
            } else {
                (hc * 16).next_power_of_two()
            },
            2,
            2,
        );
        let total_size =
            (hyperlink_map_start + hyperlink_map_layout.total_size).next_multiple_of(4096);
        Layout {
            total_size,
            rows_start,
            rows_size,
            cells_start,
            cells_size,
            styles_start,
            styles_layout,
            grapheme_alloc_start,
            grapheme_alloc_layout,
            grapheme_map_start,
            grapheme_map_layout,
            string_alloc_start,
            string_alloc_layout,
            hyperlink_set_start,
            hyperlink_set_layout,
            hyperlink_map_start,
            hyperlink_map_layout,
            capacity: cap,
        }
    }
    /// Allocate zeroed rows and cells. Initial logical size is the full capacity.
    pub fn init(cap: Capacity) -> Result<Self, Error> {
        if cap.cols == 0 || cap.rows == 0 {
            return Err(Error::InvalidDimensions);
        }
        let l = Self::layout(cap);
        if l.total_size > u32::MAX as usize {
            return Err(Error::OutOfSpace);
        }
        let mut rows = Vec::new();
        rows.try_reserve_exact(usize::from(cap.rows))
            .map_err(|_| Error::OutOfMemory)?;
        for y in 0..cap.rows {
            let mut row = Row::default();
            row.set_cells(usize::from(y) * usize::from(cap.cols));
            rows.push(row);
        }
        let count = usize::from(cap.cols) * usize::from(cap.rows);
        // Zeroed allocation: large buffers come lazily zeroed from the OS.
        let cells: Vec<Cell> =
            bytemuck::allocation::try_zeroed_vec(count).map_err(|()| Error::OutOfMemory)?;
        let grapheme_alloc = ChunkAlloc::new(l.grapheme_alloc_layout, 16)?;
        let mut grapheme_data = Vec::new();
        let codepoints = grapheme_alloc.chunks * 4;
        grapheme_data
            .try_reserve_exact(codepoints)
            .map_err(|_| Error::OutOfMemory)?;
        grapheme_data.resize(codepoints, 0);
        let mut grapheme_map = IndexMap::default();
        grapheme_map
            .try_reserve(l.grapheme_map_layout.capacity)
            .map_err(|_| Error::OutOfMemory)?;
        let mut grapheme_allocations = IndexMap::default();
        grapheme_allocations
            .try_reserve(l.grapheme_map_layout.capacity)
            .map_err(|_| Error::OutOfMemory)?;
        let mut hyperlink_map = HashMap::new();
        hyperlink_map
            .try_reserve(l.hyperlink_map_layout.capacity)
            .map_err(|_| Error::OutOfMemory)?;
        let mut link_allocations = HashMap::new();
        link_allocations
            .try_reserve(l.hyperlink_set_layout.cap)
            .map_err(|_| Error::OutOfMemory)?;
        Ok(Self {
            size: Size {
                rows: cap.rows,
                cols: cap.cols,
            },
            capacity: cap,
            dirty: false,
            rows,
            cells,
            styles: style::Set::init_with_hasher(usize::from(cap.styles), Style::zig_hash)?,
            hyperlink_set: hyperlink::Set::init_with_hasher(
                usize::from(cap.hyperlink_bytes) / 48,
                Hyperlink::zig_hash,
            )?,
            grapheme_map,
            grapheme_data,
            grapheme_allocations,
            grapheme_alloc,
            hyperlink_map,
            string_alloc: ChunkAlloc::new(l.string_alloc_layout, 32)?,
            link_allocations,
            pause_integrity_checks: 0,
        })
    }
    /// Reset contents, retaining grid and allocator backing storage.
    pub fn reinit(&mut self) -> Result<(), Error> {
        self.cells.fill(Cell::default());
        for (y, r) in self.rows.iter_mut().enumerate() {
            *r = Row::default();
            r.set_cells(y * usize::from(self.capacity.cols));
        }
        self.size = Size {
            cols: self.capacity.cols,
            rows: self.capacity.rows,
        };
        self.dirty = false;
        self.grapheme_map.clear();
        self.grapheme_allocations.clear();
        self.grapheme_alloc.reset();
        self.hyperlink_map.clear();
        self.string_alloc.reset();
        self.link_allocations.clear();
        self.styles.clear();
        self.hyperlink_set.clear();
        Ok(())
    }
    /// Return row metadata by logical row number.
    pub fn get_row(&self, y: usize) -> Option<&Row> {
        self.rows.get(y).filter(|_| y < usize::from(self.size.rows))
    }
    /// Return mutable row metadata by logical row number.
    pub fn get_row_mut(&mut self, y: usize) -> Option<&mut Row> {
        self.rows
            .get_mut(y)
            .filter(|_| y < usize::from(self.size.rows))
    }
    /// Return the visible cells of a logical row in O(1).
    pub fn get_cells(&self, y: usize) -> Option<&[Cell]> {
        let start = self.get_row(y)?.cells();
        self.cells.get(start..start + usize::from(self.size.cols))
    }
    /// Return mutable visible cells of a logical row in O(1).
    pub fn get_cells_mut(&mut self, y: usize) -> Option<&mut [Cell]> {
        let start = self.get_row(y)?.cells();
        self.cells
            .get_mut(start..start + usize::from(self.size.cols))
    }
    /// Return a cell index for a logical coordinate.
    pub fn cell_index(&self, x: usize, y: usize) -> Result<usize, Error> {
        if x >= usize::from(self.size.cols) {
            return Err(Error::InvalidCoordinate);
        }
        Ok(self.get_row(y).ok_or(Error::InvalidCoordinate)?.cells() + x)
    }
    /// Return both metadata and cell at a logical coordinate.
    pub fn get_row_and_cell(&self, x: usize, y: usize) -> Option<(&Row, &Cell)> {
        Some((self.get_row(y)?, &self.cells[self.cell_index(x, y).ok()?]))
    }
    /// Resolve style without a map lookup for the default style.
    pub fn style(&self, cell: Cell) -> Style {
        if cell.style_id() == 0 {
            Style::default()
        } else {
            self.styles
                .get(cell.style_id())
                .copied()
                .unwrap_or_default()
        }
    }
    /// Assign and retain styling, releasing the cell's previous style.
    pub fn set_style(&mut self, x: usize, y: usize, value: Style) -> Result<(), Error> {
        let i = self.cell_index(x, y)?;
        let id = if value.is_default() {
            0
        } else {
            self.styles.add(value)?
        };
        let old = self.cells[i].style_id();
        if old != 0 {
            self.styles.release(old)?;
        }
        self.cells[i].set_style_id(id);
        if id != 0 {
            self.rows[y].set_styled(true);
        }
        self.rows[y].set_dirty(true);
        Ok(())
    }
    /// Return additional codepoints, excluding the first stored on the cell.
    pub fn lookup_grapheme(&self, index: usize) -> Option<&[u32]> {
        self.grapheme_map
            .get(&index)
            .map(|range| &self.grapheme_data[range.clone()])
    }
    /// Number of cells with grapheme data.
    pub fn grapheme_count(&self) -> usize {
        self.grapheme_map.len()
    }
    /// Maximum number of grapheme map entries.
    pub fn grapheme_capacity(&self) -> usize {
        Self::layout(self.capacity).grapheme_map_layout.capacity
    }
    /// Attach additional codepoints to a textual cell.
    pub fn set_graphemes(&mut self, x: usize, y: usize, cps: &[u32]) -> Result<(), Error> {
        let i = self.cell_index(x, y)?;
        if self.cells[i].codepoint() == 0 || cps.is_empty() || self.cells[i].has_grapheme() {
            return Err(Error::InvalidCoordinate);
        }
        if self.grapheme_count() >= self.grapheme_capacity() {
            return Err(Error::GraphemeMapOutOfMemory);
        }
        let a = self
            .grapheme_alloc
            .alloc(cps.len() * 4)
            .ok_or(Error::GraphemeAllocOutOfMemory)?;
        let range = a.start * 4..a.start * 4 + cps.len();
        self.grapheme_data[range.clone()].copy_from_slice(cps);
        self.grapheme_map.insert(i, range);
        self.grapheme_allocations.insert(i, a);
        self.cells[i].set_content_tag(ContentTag::CodepointGrapheme);
        self.rows[y].set_grapheme(true);
        Ok(())
    }
    /// Append one additional codepoint, retaining allocation chunk semantics.
    pub fn append_grapheme(&mut self, x: usize, y: usize, cp: u32) -> Result<(), Error> {
        let i = self.cell_index(x, y)?;
        if !self.cells[i].has_grapheme() {
            return self.set_graphemes(x, y, &[cp]);
        }
        let range = self
            .grapheme_map
            .get_mut(&i)
            .ok_or(Error::InvalidCoordinate)?;
        if range.len().is_multiple_of(4) {
            // Allocate before releasing the old block, as in the page allocator.
            let a = self
                .grapheme_alloc
                .alloc((range.len() + 1) * 4)
                .ok_or(Error::GraphemeAllocOutOfMemory)?;
            let start = a.start * 4;
            self.grapheme_data.copy_within(range.clone(), start);
            *range = start..start + range.len();
            if let Some(old) = self.grapheme_allocations.insert(i, a) {
                self.grapheme_alloc.free(old);
            }
        }
        self.grapheme_data[range.end] = cp;
        range.end += 1;
        Ok(())
    }
    /// Release a cell's additional codepoints.
    pub fn clear_grapheme(&mut self, index: usize) {
        if self.grapheme_map.remove(&index).is_some()
            && let Some(a) = self.grapheme_allocations.remove(&index)
        {
            self.grapheme_alloc.free(a);
        }
        if let Some(c) = self.cells.get_mut(index)
            && c.has_grapheme()
        {
            c.set_content_tag(ContentTag::Codepoint);
        }
    }
    /// Move grapheme ownership between cell indices.
    pub fn move_grapheme(&mut self, src: usize, dst: usize) -> Result<(), Error> {
        if src >= self.cells.len() || dst >= self.cells.len() {
            return Err(Error::InvalidCoordinate);
        }
        if src == dst {
            return Ok(());
        }
        self.clear_grapheme(dst);
        if let Some(data) = self.grapheme_map.remove(&src) {
            self.grapheme_map.insert(dst, data);
            if let Some(a) = self.grapheme_allocations.remove(&src) {
                self.grapheme_allocations.insert(dst, a);
            }
            self.cells[src].set_content_tag(ContentTag::Codepoint);
            self.cells[dst].set_content_tag(ContentTag::CodepointGrapheme);
        }
        Ok(())
    }
    /// Look up a cell's page-local hyperlink ID.
    pub fn lookup_hyperlink(&self, index: usize) -> Option<u16> {
        self.hyperlink_map.get(&index).copied()
    }
    /// Bytes of string storage currently in use (Zig:
    /// `string_alloc.usedBytes`); used by the inspector.
    pub fn string_used_bytes(&self) -> usize {
        let used: usize = self
            .string_alloc
            .used
            .iter()
            .map(|w| w.count_ones() as usize)
            .sum();
        used * self.string_alloc.chunk
    }
    /// Total bytes of string storage (Zig: `string_alloc.capacityBytes`).
    pub fn string_capacity_bytes(&self) -> usize {
        self.string_alloc.chunks * self.string_alloc.chunk
    }
    /// Number of hyperlink cells.
    pub fn hyperlink_count(&self) -> usize {
        self.hyperlink_map.len()
    }
    /// Maximum hyperlink-map entries.
    pub fn hyperlink_capacity(&self) -> usize {
        Self::layout(self.capacity).hyperlink_map_layout.capacity
    }
    /// Allocate strings and retain a page-local hyperlink.
    pub fn insert_hyperlink(&mut self, link: Hyperlink) -> Result<u16, Error> {
        self.insert_hyperlink_with_id(link, None)
    }
    /// Retain a hyperlink, preserving a source page ID when possible.
    pub(crate) fn insert_hyperlink_with_id(
        &mut self,
        link: Hyperlink,
        preferred: Option<u16>,
    ) -> Result<u16, Error> {
        // Existing live links already own their strings. Retain them before
        // attempting another allocation on a potentially full page.
        if let Some(id) = self.hyperlink_set.lookup(&link) {
            self.hyperlink_set.r#use(id).map_err(|_| Error::InvalidId)?;
            return Ok(id);
        }
        let allocations = self.allocate_hyperlink_strings(&link)?;
        let result = if let Some(id) = preferred {
            self.hyperlink_set
                .add_with_id(link, id)
                .map(|actual| actual.unwrap_or(id))
        } else {
            self.hyperlink_set.add(link)
        };
        self.finish_hyperlink_insert(result, allocations)
    }
    fn allocate_hyperlink_strings(
        &mut self,
        link: &Hyperlink,
    ) -> Result<[Option<Allocation>; 2], Error> {
        let mut allocations = [None; 2];
        let uri = self
            .string_alloc
            .alloc(link.uri.len())
            .ok_or(Error::StringsOutOfMemory)?;
        allocations[0] = Some(uri);
        if let HyperlinkId::Explicit(v) = &link.id {
            if let Some(a) = self.string_alloc.alloc(v.len()) {
                allocations[1] = Some(a);
            } else {
                self.string_alloc.free(uri);
                return Err(Error::StringsOutOfMemory);
            }
        }
        Ok(allocations)
    }
    fn finish_hyperlink_insert(
        &mut self,
        result: Result<u16, AddError>,
        allocations: [Option<Allocation>; 2],
    ) -> Result<u16, Error> {
        // A failed insertion may still trim dead IDs. Reclaim their strings
        // before returning the error, matching the set's deletion callback.
        self.link_allocations.retain(|id, allocations| {
            if self.hyperlink_set.contains_slot(*id) {
                return true;
            }
            for a in allocations.iter().flatten() {
                self.string_alloc.free(*a);
            }
            false
        });
        let id = match result {
            Ok(id) => id,
            Err(e) => {
                for a in allocations.into_iter().flatten() {
                    self.string_alloc.free(a);
                }
                return Err(match e {
                    AddError::NeedsRehash => Error::SetNeedsRehash,
                    _ => Error::SetOutOfMemory,
                });
            }
        };
        if self.hyperlink_set.ref_count(id) > 1 {
            for a in allocations.into_iter().flatten() {
                self.string_alloc.free(a);
            }
        } else if let Some(v) = self.link_allocations.insert(id, allocations) {
            for a in v.into_iter().flatten() {
                self.string_alloc.free(a);
            }
        }
        Ok(id)
    }
    /// Set a hyperlink, consuming an existing retained reference (no increment).
    pub fn set_hyperlink(&mut self, x: usize, y: usize, id: u16) -> Result<(), Error> {
        let i = self.cell_index(x, y)?;
        if self.hyperlink_set.get(id).is_none() {
            return Err(Error::InvalidId);
        }
        if !self.hyperlink_map.contains_key(&i)
            && self.hyperlink_count() >= self.hyperlink_capacity()
        {
            return Err(Error::HyperlinkMapOutOfMemory);
        }
        self.hyperlink_map
            .try_reserve(1)
            .map_err(|_| Error::OutOfMemory)?;
        self.clear_hyperlink(i)?;
        self.hyperlink_map.insert(i, id);
        self.cells[i].set_hyperlink(true);
        self.rows[y].set_hyperlink(true);
        Ok(())
    }
    /// Release a cell's hyperlink reference.
    pub fn clear_hyperlink(&mut self, i: usize) -> Result<(), Error> {
        if let Some(id) = self.hyperlink_map.remove(&i) {
            self.hyperlink_set.release(id)?;
        }
        if let Some(c) = self.cells.get_mut(i) {
            c.set_hyperlink(false);
        }
        Ok(())
    }
    /// Recompute the conservative row grapheme flag.
    pub fn update_row_grapheme_flag(&mut self, y: usize) {
        if let Some(c) = self.get_cells(y) {
            let v = c.iter().any(|c| c.has_grapheme());
            self.rows[y].set_grapheme(v);
        }
    }
    /// Recompute the conservative row style flag.
    pub fn update_row_styled_flag(&mut self, y: usize) {
        if let Some(c) = self.get_cells(y) {
            let v = c.iter().any(|c| c.has_styling());
            self.rows[y].set_styled(v);
        }
    }
    /// Recompute the conservative row hyperlink flag.
    pub fn update_row_hyperlink_flag(&mut self, y: usize) {
        if let Some(c) = self.get_cells(y) {
            let v = c.iter().any(|c| c.hyperlink());
            self.rows[y].set_hyperlink(v);
        }
    }
    /// True if the page or any visible row is dirty.
    pub fn is_dirty(&self) -> bool {
        self.dirty
            || self.rows[..usize::from(self.size.rows)]
                .iter()
                .any(|r| r.dirty())
    }
    /// Clear a half-open column range, optionally preserving protected cells.
    pub fn clear_cells(
        &mut self,
        y: usize,
        start: usize,
        end: usize,
        protect: bool,
    ) -> Result<(), Error> {
        if start > end || end > usize::from(self.size.cols) || y >= usize::from(self.size.rows) {
            return Err(Error::InvalidCoordinate);
        }
        let old = self.rows[y];
        let full = start == 0 && end == usize::from(self.size.cols) && !protect;
        // Only needed when the row stays (possibly) styled, i.e. a partial clear.
        let styled = old.styled()
            && !full
            && self
                .get_cells(y)
                .is_some_and(|cells| cells.iter().any(|c| c.has_styling()));
        if old.kitty_virtual_placeholder()
            && full
            && self
                .get_cells(y)
                .is_some_and(|cells| cells.iter().all(|c| c.codepoint() != 0x10eeee))
        {
            self.rows[y].set_kitty_virtual_placeholder(false);
        }
        let base = self.rows[y].cells();

        // Fast path: the row flags are a superset of what its cells carry, so
        // with none of them set there is nothing to release per cell.
        if !protect && !old.styled() && !old.grapheme() && !old.hyperlink() {
            self.cells[base + start..base + end].fill(Cell::default());
            self.rows[y].set_dirty(true);
            return Ok(());
        }

        for i in base + start..base + end {
            let c = self.cells[i];
            if protect && c.protected() {
                continue;
            }
            if c.has_styling() {
                self.styles.release(c.style_id())?;
            }
            if c.has_grapheme() {
                self.clear_grapheme(i);
            }
            if c.hyperlink() {
                self.clear_hyperlink(i)?;
            }
            self.cells[i] = Cell::default();
        }
        if old.grapheme() {
            if full {
                self.rows[y].set_grapheme(false);
            } else {
                self.update_row_grapheme_flag(y);
            }
        }
        if old.hyperlink() {
            if full {
                self.rows[y].set_hyperlink(false);
            } else {
                self.update_row_hyperlink_flag(y);
            }
        }
        if old.styled() {
            self.rows[y].set_styled(!full && styled);
        }
        self.rows[y].set_dirty(true);
        Ok(())
    }
    /// Move a half-open cell range, clearing overwritten values and the source.
    pub fn move_cells(
        &mut self,
        src_y: usize,
        src_x: usize,
        dst_y: usize,
        dst_x: usize,
        len: usize,
    ) -> Result<(), Error> {
        if src_x + len > usize::from(self.size.cols) || dst_x + len > usize::from(self.size.cols) {
            return Err(Error::InvalidCoordinate);
        }
        let src = self.cell_index(src_x, src_y)?;
        let dst = self.cell_index(dst_x, dst_y)?;
        if src == dst {
            return Ok(());
        }
        // Move in memmove order; auxiliary maps follow the cell indices.
        for n in 0..len {
            let n = if dst > src { len - 1 - n } else { n };
            let si = src + n;
            let di = dst + n;
            self.clear_cells(dst_y, dst_x + n, dst_x + n + 1, false)?;
            let c = self.cells[si];
            self.cells[di] = c;
            self.cells[si] = Cell::default();
            if c.has_grapheme() {
                if let Some(v) = self.grapheme_map.remove(&si) {
                    self.grapheme_map.insert(di, v);
                }
                if let Some(a) = self.grapheme_allocations.remove(&si) {
                    self.grapheme_allocations.insert(di, a);
                }
            }
            if c.hyperlink()
                && let Some(v) = self.hyperlink_map.remove(&si)
            {
                self.hyperlink_map.insert(di, v);
            }
        }
        for y in [src_y, dst_y] {
            self.update_row_grapheme_flag(y);
            self.update_row_styled_flag(y);
            self.update_row_hyperlink_flag(y);
            self.rows[y].set_dirty(true);
        }
        Ok(())
    }
    /// Swap two cells and their managed values without changing reference counts.
    pub fn swap_cells(&mut self, ax: usize, ay: usize, bx: usize, by: usize) -> Result<(), Error> {
        let a = self.cell_index(ax, ay)?;
        let b = self.cell_index(bx, by)?;
        if a == b {
            return Ok(());
        }
        self.cells.swap(a, b);

        swap_map(&mut self.grapheme_map, a, b);
        swap_map(&mut self.grapheme_allocations, a, b);
        swap_map(&mut self.hyperlink_map, a, b);
        for y in [ay, by] {
            self.update_row_grapheme_flag(y);
            self.update_row_styled_flag(y);
            self.update_row_hyperlink_flag(y);
            self.rows[y].set_dirty(true);
        }
        Ok(())
    }
    pub(crate) fn export_cell(&self, i: usize) -> OwnedCell {
        let cell = self.cells[i];
        OwnedCell {
            cell,
            style: if cell.has_styling() {
                self.styles.get(cell.style_id()).copied()
            } else {
                None
            },
            grapheme: if cell.has_grapheme() {
                self.lookup_grapheme(i).map(<[u32]>::to_vec)
            } else {
                None
            },
            hyperlink: if cell.hyperlink() {
                self.lookup_hyperlink(i)
                    .and_then(|id| self.hyperlink_set.get(id))
                    .cloned()
            } else {
                None
            },
        }
    }
    pub(crate) fn import_cell(&mut self, x: usize, y: usize, v: &OwnedCell) -> Result<(), Error> {
        let i = self.cell_index(x, y)?;
        self.clear_cells(y, x, x + 1, false)?;
        let mut c = v.cell;
        c.set_style_id(0);
        c.set_hyperlink(false);
        if c.has_grapheme() {
            c.set_content_tag(ContentTag::Codepoint);
        }
        self.cells[i] = c;
        if let Some(s) = v.style {
            self.set_style(x, y, s)?;
        }
        if let Some(g) = &v.grapheme {
            self.set_graphemes(x, y, g)?;
        }
        if let Some(h) = &v.hyperlink {
            let id = if let Some(id) = self.hyperlink_set.lookup(h) {
                self.hyperlink_set.r#use(id)?;
                id
            } else {
                self.insert_hyperlink(h.clone())?
            };
            if let Err(e) = self.set_hyperlink(x, y, id) {
                self.hyperlink_set.release(id)?;
                return Err(e);
            }
        }
        Ok(())
    }
    /// Copy a cell directly from a borrowed source page. Only the destination's
    /// persistent managed storage is allocated; no owned cell snapshot is made.
    pub(crate) fn clone_cell_from(
        &mut self,
        src: &Self,
        i: usize,
        x: usize,
        y: usize,
    ) -> Result<(), Error> {
        self.clear_cells(y, x, x + 1, false)?;
        self.clone_cell_from_cleared(src, i, x, y)
    }
    /// Like [`Page::clone_cell_from`] but the destination cell must already
    /// have been cleared (e.g. its whole row range was cleared up front).
    fn clone_cell_from_cleared(
        &mut self,
        src: &Self,
        i: usize,
        x: usize,
        y: usize,
    ) -> Result<(), Error> {
        let cell = src.cells[i];
        let dst = self.cell_index(x, y)?;
        // Plain cells (the vast majority) carry no managed memory.
        if !cell.has_grapheme() && !cell.hyperlink() && !cell.has_styling() {
            self.cells[dst] = cell;
            return Ok(());
        }
        self.cells[dst] = cell;
        self.cells[dst].set_style_id(0);
        self.cells[dst].set_hyperlink(false);
        if cell.has_grapheme() {
            self.cells[dst].set_content_tag(ContentTag::Codepoint);
        }
        if cell.has_grapheme() {
            self.set_graphemes(x, y, src.lookup_grapheme(i).ok_or(Error::InvalidId)?)?;
        }
        if cell.hyperlink() {
            let source_id = src.lookup_hyperlink(i).ok_or(Error::InvalidId)?;
            let link = src.hyperlink_set.get(source_id).ok_or(Error::InvalidId)?;
            let id = if let Some(id) = self.hyperlink_set.lookup(link) {
                self.hyperlink_set.r#use(id)?;
                id
            } else {
                self.insert_hyperlink_with_id(link.clone(), Some(source_id))?
            };
            if let Err(error) = self.set_hyperlink(x, y, id) {
                self.hyperlink_set.release(id)?;
                return Err(error);
            }
        }
        if cell.has_styling() {
            let style = *src.styles.get(cell.style_id()).ok_or(Error::InvalidId)?;
            let id = self
                .styles
                .add_with_id(style, cell.style_id())?
                .unwrap_or(cell.style_id());
            self.cells[dst].set_style_id(id);
            self.rows[y].set_styled(true);
        }
        Ok(())
    }
    pub(crate) fn can_allocate_grapheme(&mut self, bytes: usize) -> bool {
        if let Some(allocation) = self.grapheme_alloc.alloc(bytes) {
            self.grapheme_alloc.free(allocation);
            true
        } else {
            false
        }
    }
    /// Test the contiguous allocation used by reflow before duplicating a link.
    pub(crate) fn can_allocate_string(&mut self, bytes: usize) -> bool {
        if let Some(allocation) = self.string_alloc.alloc(bytes) {
            self.string_alloc.free(allocation);
            true
        } else {
            false
        }
    }
    pub(crate) fn clone_hyperlink_from(
        &mut self,
        src: &Self,
        i: usize,
        x: usize,
        y: usize,
    ) -> Result<(), Error> {
        if self.hyperlink_count() >= self.hyperlink_capacity() {
            return Err(Error::HyperlinkMapOutOfMemory);
        }
        let source_id = src.lookup_hyperlink(i).ok_or(Error::InvalidId)?;
        let link = src.hyperlink_set.get(source_id).ok_or(Error::InvalidId)?;
        let required = link.uri.len()
            + match &link.id {
                HyperlinkId::Explicit(id) => id.len(),
                HyperlinkId::Implicit(_) => 0,
            };
        if !self.can_allocate_string(required) {
            return Err(Error::StringsOutOfMemory);
        }
        let allocations = self.allocate_hyperlink_strings(link)?;
        let result = self
            .hyperlink_set
            .add_ref_with_id(link, source_id)
            .map(|id| id.unwrap_or(source_id));
        let id = self.finish_hyperlink_insert(result, allocations)?;
        if let Err(e) = self.set_hyperlink(x, y, id) {
            self.hyperlink_set.release(id)?;
            return Err(e);
        }
        Ok(())
    }
    /// Clone rows from another page, truncating or padding columns as needed.
    pub fn clone_from(&mut self, src: &Self, y_start: usize, y_end: usize) -> Result<(), Error> {
        if y_start > y_end
            || y_end > usize::from(src.size.rows)
            || y_end - y_start > usize::from(self.capacity.rows)
        {
            return Err(Error::InvalidCoordinate);
        }
        for y in 0..y_end - y_start {
            self.clone_row_from(src, y_start + y, y)?;
        }
        Ok(())
    }
    /// Clone one complete row to a destination row.
    pub fn clone_row_from(&mut self, src: &Self, src_y: usize, dst_y: usize) -> Result<(), Error> {
        let cols = usize::from(self.size.cols.min(src.size.cols));
        self.clone_partial_row_from(src, src_y, dst_y, 0, 0, cols)?;
        if self.size.cols > src.size.cols {
            let i = self.cell_index(usize::from(src.size.cols - 1), dst_y)?;
            if self.cells[i].wide() == Wide::SpacerHead {
                self.cells[i].set_wide(Wide::Narrow);
            }
        }
        Ok(())
    }
    /// Clone a column range, preserving untouched destination cells.
    pub fn clone_partial_row_from(
        &mut self,
        src: &Self,
        src_y: usize,
        dst_y: usize,
        src_x: usize,
        dst_x: usize,
        len: usize,
    ) -> Result<(), Error> {
        if src_x
            .checked_add(len)
            .is_none_or(|end| end > usize::from(src.size.cols))
            || dst_x
                .checked_add(len)
                .is_none_or(|end| end > usize::from(self.size.cols))
        {
            return Err(Error::InvalidCoordinate);
        }
        let source_row = *src.get_row(src_y).ok_or(Error::InvalidCoordinate)?;
        let original_row = *self.get_row(dst_y).ok_or(Error::InvalidCoordinate)?;
        self.clear_cells(dst_y, dst_x, dst_x + len, false)?;
        if source_row.managed_memory() {
            // Row metadata/cell indices were validated above (`get_row`).
            let from = source_row.cells() + src_x;
            let to = original_row.cells() + dst_x;
            let src_cells = &src.cells[from..from + len];
            let mut run = 0;
            for (n, cell) in src_cells.iter().enumerate() {
                if !cell.has_grapheme() && !cell.hyperlink() && !cell.has_styling() {
                    continue;
                }
                // Bulk-copy the preceding run of plain cells (no managed memory).
                self.cells[to + run..to + n].copy_from_slice(&src_cells[run..n]);
                self.clone_cell_from_cleared(src, from + n, dst_x + n, dst_y)?;
                run = n + 1;
            }
            self.cells[to + run..to + len].copy_from_slice(&src_cells[run..len]);
        } else {
            let from = source_row.cells() + src_x;
            let to = original_row.cells() + dst_x;
            self.cells[to..to + len].copy_from_slice(&src.cells[from..from + len]);
        }
        let managed = self.rows[dst_y];
        let mut metadata = source_row;
        metadata.set_cells(original_row.cells());
        if len < usize::from(self.size.cols) {
            metadata.set_wrap(original_row.wrap());
            metadata.set_wrap_continuation(original_row.wrap_continuation());
            metadata.set_grapheme(managed.grapheme());
            metadata.set_styled(managed.styled());
            metadata.set_hyperlink(managed.hyperlink());
            metadata.set_dirty(source_row.dirty() || original_row.dirty());
        }
        self.rows[dst_y] = metadata;
        Ok(())
    }
    /// Clone columns within a page with the same row metadata rules as a cross-page copy.
    pub(crate) fn clone_partial_row_within(
        &mut self,
        src_y: usize,
        dst_y: usize,
        left: usize,
        len: usize,
    ) -> Result<(), Error> {
        let source_row = *self.get_row(src_y).ok_or(Error::InvalidCoordinate)?;
        let original_row = *self.get_row(dst_y).ok_or(Error::InvalidCoordinate)?;
        self.clone_cells_within(src_y, dst_y, left, left, len)?;
        let managed = self.rows[dst_y];
        let mut metadata = source_row;
        metadata.set_cells(original_row.cells());
        if len < usize::from(self.size.cols) {
            metadata.set_wrap(original_row.wrap());
            metadata.set_wrap_continuation(original_row.wrap_continuation());
            metadata.set_grapheme(managed.grapheme());
            metadata.set_styled(managed.styled());
            metadata.set_hyperlink(managed.hyperlink());
            metadata.set_dirty(source_row.dirty() || original_row.dirty());
        }
        self.rows[dst_y] = metadata;
        Ok(())
    }
    /// Clone a range within the same page, including overlapping ranges.
    pub fn clone_cells_within(
        &mut self,
        src_y: usize,
        dst_y: usize,
        src_x: usize,
        dst_x: usize,
        len: usize,
    ) -> Result<(), Error> {
        if src_x + len > usize::from(self.size.cols) || dst_x + len > usize::from(self.size.cols) {
            return Err(Error::InvalidCoordinate);
        }
        let src = self.cell_index(src_x, src_y)?;
        self.cell_index(dst_x, dst_y)?;
        let dst = self.cell_index(dst_x, dst_y)?;
        // Copy in memmove order. Only one managed cell is borrowed through an
        // owned value at a time, so overlap never requires a row snapshot.
        for n in 0..len {
            let n = if dst > src { len - 1 - n } else { n };
            let value = self.export_cell(src + n);
            self.import_cell(dst_x + n, dst_y, &value)?;
        }
        Ok(())
    }
    /// Minimum capacity that can clone the half-open row range.
    pub fn exact_row_capacity(&self, start: usize, end: usize) -> Result<Capacity, Error> {
        if start >= end || end > usize::from(self.size.rows) {
            return Err(Error::InvalidCoordinate);
        }
        let mut styles = HashSet::new();
        let mut links = HashSet::new();
        let mut gb = 0;
        let mut sb = 0;
        let mut hc: usize = 0;
        for y in start..end {
            let base = self.rows[y].cells();
            for x in 0..usize::from(self.size.cols) {
                let i = base + x;
                let c = self.cells[i];
                if c.has_styling() {
                    styles.insert(c.style_id());
                }
                if let Some(g) = self.lookup_grapheme(i) {
                    gb += (g.len() * 4).next_multiple_of(16);
                }
                if let Some(id) = self.lookup_hyperlink(i) {
                    hc += 1;
                    if links.insert(id)
                        && let Some(h) = self.hyperlink_set.get(id)
                    {
                        sb += h.string_bytes();
                    }
                }
            }
        }
        let sc = style::Set::capacity_for_count(styles.len());
        let lc = hyperlink::Set::capacity_for_count(links.len()).max(hc.div_ceil(16));
        Ok(Capacity {
            cols: self.size.cols,
            rows: (end - start) as u16,
            styles: u16::try_from(sc).map_err(|_| Error::OutOfSpace)?,
            hyperlink_bytes: u16::try_from(lc * 48).map_err(|_| Error::OutOfSpace)?,
            grapheme_bytes: u32::try_from(gb).map_err(|_| Error::OutOfSpace)?,
            string_bytes: u32::try_from(sb).map_err(|_| Error::OutOfSpace)?,
        })
    }
    /// Nestable suspension of debug integrity checks.
    pub fn pause_integrity_checks(&mut self, pause: bool) {
        if pause {
            self.pause_integrity_checks += 1;
        } else {
            self.pause_integrity_checks = self.pause_integrity_checks.saturating_sub(1);
        }
    }
}
/// Structural invariant failures reported by debug/test verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityError {
    /// ZeroRowCount invariant violation.
    ZeroRowCount,
    /// ZeroColCount invariant violation.
    ZeroColCount,
    /// UnmarkedGraphemeRow invariant violation.
    UnmarkedGraphemeRow,
    /// MissingGraphemeData invariant violation.
    MissingGraphemeData,
    /// InvalidGraphemeCount invariant violation.
    InvalidGraphemeCount,
    /// UnmarkedGraphemeCell invariant violation.
    UnmarkedGraphemeCell,
    /// MissingStyle invariant violation.
    MissingStyle,
    /// UnmarkedStyleRow invariant violation.
    UnmarkedStyleRow,
    /// MismatchedStyleRef invariant violation.
    MismatchedStyleRef,
    /// InvalidStyleCount invariant violation.
    InvalidStyleCount,
    /// MissingHyperlinkData invariant violation.
    MissingHyperlinkData,
    /// MismatchedHyperlinkRef invariant violation.
    MismatchedHyperlinkRef,
    /// UnmarkedHyperlinkCell invariant violation.
    UnmarkedHyperlinkCell,
    /// UnmarkedHyperlinkRow invariant violation.
    UnmarkedHyperlinkRow,
    /// InvalidSpacerTailLocation invariant violation.
    InvalidSpacerTailLocation,
    /// InvalidSpacerHeadLocation invariant violation.
    InvalidSpacerHeadLocation,
    /// UnwrappedSpacerHead invariant violation.
    UnwrappedSpacerHead,
    /// InvalidDimensions invariant violation.
    InvalidDimensions,
}
#[cfg(any(debug_assertions, test))]
impl Page {
    /// Verify grid, metadata, auxiliary maps, and reference counts.
    pub fn verify_integrity(&self) -> Result<(), IntegrityError> {
        use IntegrityError as E;
        if self.pause_integrity_checks > 0 {
            return Ok(());
        }
        if self.size.rows == 0 {
            return Err(E::ZeroRowCount);
        }
        if self.size.cols == 0 {
            return Err(E::ZeroColCount);
        }
        if self.size.rows > self.capacity.rows || self.size.cols > self.capacity.cols {
            return Err(E::InvalidDimensions);
        }
        let mut styles = HashMap::<u16, usize>::new();
        let mut links = HashMap::<u16, usize>::new();
        let mut graphemes = 0;
        for y in 0..usize::from(self.size.rows) {
            let r = self.rows[y];
            let base = r.cells();
            for x in 0..usize::from(self.size.cols) {
                let i = base + x;
                let c = self.cells[i];
                if c.has_grapheme() {
                    graphemes += 1;
                    if !r.grapheme() {
                        return Err(E::UnmarkedGraphemeRow);
                    }
                    let g = self.lookup_grapheme(i).ok_or(E::MissingGraphemeData)?;
                    if g.is_empty() {
                        return Err(E::InvalidGraphemeCount);
                    }
                } else if self.grapheme_map.contains_key(&i) {
                    return Err(E::UnmarkedGraphemeCell);
                }
                if c.has_styling() {
                    if !r.styled() {
                        return Err(E::UnmarkedStyleRow);
                    }
                    if self.styles.get(c.style_id()).is_none() {
                        return Err(E::MissingStyle);
                    }
                    *styles.entry(c.style_id()).or_default() += 1;
                }
                if c.hyperlink() {
                    if !r.hyperlink() {
                        return Err(E::UnmarkedHyperlinkRow);
                    }
                    let id = self.lookup_hyperlink(i).ok_or(E::MissingHyperlinkData)?;
                    if self.hyperlink_set.get(id).is_none() {
                        return Err(E::MissingHyperlinkData);
                    }
                    *links.entry(id).or_default() += 1;
                } else if self.hyperlink_map.contains_key(&i) {
                    return Err(E::UnmarkedHyperlinkCell);
                }
                match c.wide() {
                    Wide::SpacerTail => {
                        if x == 0 || self.cells[i - 1].wide() != Wide::Wide {
                            return Err(E::InvalidSpacerTailLocation);
                        }
                    }
                    Wide::SpacerHead => {
                        if x + 1 != usize::from(self.size.cols) {
                            return Err(E::InvalidSpacerHeadLocation);
                        }
                        if !r.wrap() {
                            return Err(E::UnwrappedSpacerHead);
                        }
                    }
                    _ => {}
                }
            }
        }
        if graphemes > self.grapheme_count() {
            return Err(E::InvalidGraphemeCount);
        }
        for (id, _, refs) in self.styles.entries() {
            if styles.get(&id).copied().unwrap_or(0) > usize::from(refs) {
                return Err(E::MismatchedStyleRef);
            }
        }
        for (id, _, refs) in self.hyperlink_set.entries() {
            if links.get(&id).copied().unwrap_or(0) > usize::from(refs) {
                return Err(E::MismatchedHyperlinkRef);
            }
        }
        Ok(())
    }
    /// Assert structural integrity in debug/test builds.
    pub fn assert_integrity(&self) {
        assert_eq!(self.verify_integrity(), Ok(()));
    }
}

#[cfg(not(any(debug_assertions, test)))]
impl Page {
    /// No-op in release builds (the integrity checks are debug-only).
    #[inline(always)]
    pub fn assert_integrity(&self) {}
}

fn swap_map<T, S: std::hash::BuildHasher>(map: &mut HashMap<usize, T, S>, a: usize, b: usize) {
    let av = map.remove(&a);
    let bv = map.remove(&b);
    if let Some(v) = av {
        map.insert(b, v);
    }
    if let Some(v) = bv {
        map.insert(a, v);
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn cell_is_plain() {
        let mut c = Cell::init(u32::from('a'));
        assert!(c.is_plain());
        c.set_style_id(1);
        assert!(!c.is_plain());
        c.set_style_id(0);
        c.set_content_tag(ContentTag::BgColorRgb);
        assert!(!c.is_plain());
        c.set_content_tag(ContentTag::CodepointGrapheme);
        assert!(!c.is_plain());
    }

    use super::*;
    #[test]
    fn chunk_alloc_matches_naive_first_fit() {
        let mut fast = ChunkAlloc::new(
            AllocLayout {
                bitmap_count: 5,
                ..AllocLayout::default()
            },
            16,
        )
        .unwrap();
        let mut naive = vec![false; 5 * 64];
        let mut live: Vec<Allocation> = Vec::new();
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            if rnd() % 3 != 0 || live.is_empty() {
                let n = (rnd() % 70 + 1) as usize;
                let expect = (|| {
                    let mut run = 0;
                    for (i, &used) in naive.iter().enumerate() {
                        run = if used { 0 } else { run + 1 };
                        if run == n {
                            return Some(i + 1 - n);
                        }
                    }
                    None
                })();
                let got = fast.alloc(n * 16);
                assert_eq!(got.map(|a| a.start), expect);
                if let Some(a) = got {
                    naive[a.start..a.start + a.len].fill(true);
                    live.push(a);
                }
            } else {
                let i = (rnd() as usize) % live.len();
                let a = live.swap_remove(i);
                naive[a.start..a.start + a.len].fill(false);
                fast.free(a);
            }
        }
    }

    use crate::style::{Color, Flags};
    fn page() -> Page {
        Page::init(Capacity {
            cols: 10,
            rows: 10,
            styles: 8,
            ..Capacity::default()
        })
        .unwrap()
    }
    fn write(p: &mut Page, x: usize, y: usize, cp: u32) {
        let i = p.cell_index(x, y).unwrap();
        p.cells[i] = Cell::init(cp);
    }
    fn cp(p: &Page, x: usize, y: usize) -> u32 {
        p.get_row_and_cell(x, y).unwrap().1.codepoint()
    }
    fn bold() -> Style {
        Style {
            flags: Flags {
                bold: true,
                ..Flags::default()
            },
            ..Style::default()
        }
    }
    fn link() -> Hyperlink {
        Hyperlink {
            id: HyperlinkId::Implicit(1),
            uri: b"https://example.com".to_vec(),
        }
    }
    fn clone_exact(p: &Page, end: usize) -> Page {
        let cap = p.exact_row_capacity(0, end).unwrap();
        let mut q = Page::init(cap).unwrap();
        q.clone_from(p, 0, end).unwrap();
        assert_eq!(q.exact_row_capacity(0, end).unwrap(), cap);
        q.assert_integrity();
        q
    }
    #[test]
    fn page_layout_can_take_a_maxed_capacity() {
        let l = Page::layout(Capacity {
            cols: u16::MAX,
            rows: u16::MAX,
            styles: u16::MAX,
            hyperlink_bytes: u16::MAX,
            grapheme_bytes: u32::MAX,
            string_bytes: u32::MAX,
        });
        assert!(l.total_size > u32::MAX as usize);
    }
    #[test]
    fn cell_is_zero_by_default() {
        let c = Cell::init(0);
        assert_eq!(c.raw(), 0);
        assert_eq!(c.semantic_content(), SemanticContent::Output);
        assert_eq!(std::mem::size_of::<Cell>(), 8);
        assert_eq!(std::mem::size_of::<Row>(), 8);
    }
    #[test]
    fn page_capacity_adjust_cols_down() {
        let original = Page::layout(STD_CAPACITY).total_size;
        let cap = STD_CAPACITY
            .adjust(Adjustment {
                cols: Some(STD_CAPACITY.cols / 2),
            })
            .unwrap();
        assert_eq!(Page::layout(cap).total_size, original);
        assert!(
            Page::layout(Capacity {
                rows: cap.rows + 1,
                ..cap
            })
            .total_size
                > original
        );
    }
    #[test]
    fn page_capacity_adjust_cols_down_to_1() {
        let original = Page::layout(STD_CAPACITY).total_size;
        let cap = STD_CAPACITY.adjust(Adjustment { cols: Some(1) }).unwrap();
        assert_eq!(Page::layout(cap).total_size, original);
        assert!(
            Page::layout(Capacity {
                rows: cap.rows + 1,
                ..cap
            })
            .total_size
                > original
        );
    }
    #[test]
    fn page_capacity_adjust_cols_up() {
        let original = Page::layout(STD_CAPACITY).total_size;
        let cap = STD_CAPACITY
            .adjust(Adjustment {
                cols: Some(STD_CAPACITY.cols * 2),
            })
            .unwrap();
        assert_eq!(Page::layout(cap).total_size, original);
        assert!(
            Page::layout(Capacity {
                rows: cap.rows + 1,
                ..cap
            })
            .total_size
                > original
        );
    }
    #[test]
    fn page_capacity_adjust_cols_sweep() {
        let original = Page::layout(STD_CAPACITY).total_size;
        let mut cap = STD_CAPACITY;
        for cols in 1..STD_CAPACITY.cols * 2 {
            cap = cap.adjust(Adjustment { cols: Some(cols) }).unwrap();
            assert_eq!(Page::layout(cap).total_size, original);
            assert!(
                Page::layout(Capacity {
                    rows: cap.rows + 1,
                    ..cap
                })
                .total_size
                    > original
            );
        }
    }
    #[test]
    fn page_capacity_adjust_cols_too_high() {
        assert_eq!(
            STD_CAPACITY.adjust(Adjustment {
                cols: Some(u16::MAX)
            }),
            Err(Error::OutOfMemory)
        );
    }
    #[test]
    fn capacity_max_cols_basic() {
        let max = STD_CAPACITY.max_cols().unwrap();
        assert!(max >= STD_CAPACITY.cols);
        assert!(
            STD_CAPACITY
                .adjust(Adjustment { cols: Some(max) })
                .unwrap()
                .rows
                >= 1
        );
        assert_eq!(
            STD_CAPACITY.adjust(Adjustment {
                cols: Some(max + 1)
            }),
            Err(Error::OutOfMemory)
        );
    }
    #[test]
    fn capacity_max_cols_preserves_total_size() {
        let cap = STD_CAPACITY
            .adjust(Adjustment {
                cols: STD_CAPACITY.max_cols(),
            })
            .unwrap();
        assert_eq!(
            Page::layout(cap).total_size,
            Page::layout(STD_CAPACITY).total_size
        );
    }
    #[test]
    fn capacity_max_cols_with_1_row_exactly() {
        assert_eq!(
            STD_CAPACITY
                .adjust(Adjustment {
                    cols: STD_CAPACITY.max_cols()
                })
                .unwrap()
                .rows,
            1
        );
    }
    #[test]
    fn page_init() {
        let p = Page::init(Capacity {
            cols: 120,
            rows: 80,
            styles: 32,
            ..Capacity::default()
        })
        .unwrap();
        p.assert_integrity();
    }
    #[test]
    fn page_read_and_write_cells() {
        let mut p = page();
        for y in 0..10 {
            write(&mut p, 1, y, y as u32);
        }
        for y in 0..10 {
            assert_eq!(cp(&p, 1, y), y as u32);
        }
    }
    #[test]
    fn page_append_grapheme_small() {
        let mut p = page();
        write(&mut p, 0, 0, 9);
        p.append_grapheme(0, 0, 10).unwrap();
        assert!(p.rows[0].grapheme());
        assert_eq!(p.lookup_grapheme(0), Some([10].as_slice()));
        p.append_grapheme(0, 0, 11).unwrap();
        assert_eq!(p.lookup_grapheme(0), Some([10, 11].as_slice()));
        p.clear_grapheme(0);
        p.update_row_grapheme_flag(0);
        assert!(!p.rows[0].grapheme());
        assert!(!p.cells[0].has_grapheme());
    }
    #[test]
    fn page_append_grapheme_larger_than_chunk() {
        let mut p = page();
        write(&mut p, 0, 0, 9);
        for i in 0..40 {
            p.append_grapheme(0, 0, 10 + i).unwrap();
        }
        assert_eq!(p.lookup_grapheme(0).unwrap(), (10..50).collect::<Vec<_>>());
    }
    #[test]
    fn page_clear_grapheme_not_all_cells() {
        let mut p = page();
        for x in 0..2 {
            write(&mut p, x, 0, 9);
            p.append_grapheme(x, 0, 10).unwrap();
        }
        p.clear_grapheme(0);
        p.update_row_grapheme_flag(0);
        assert!(p.rows[0].grapheme());
        assert!(!p.cells[0].has_grapheme());
        assert!(p.cells[1].has_grapheme());
    }
    #[test]
    fn page_clone() {
        let mut p = page();
        for y in 0..10 {
            write(&mut p, 1, y, y as u32);
        }
        let q = p.clone();
        assert_eq!(p.capacity, q.capacity);
        for y in 0..10 {
            write(&mut p, 1, y, 0);
            assert_eq!(cp(&q, 1, y), y as u32);
            assert_eq!(cp(&p, 1, y), 0);
        }
    }
    #[test]
    fn page_clone_from() {
        let mut p = page();
        for y in 0..10 {
            write(&mut p, 1, y, y as u32);
        }
        let q = {
            let mut q = page();
            q.clone_from(&p, 0, 10).unwrap();
            q
        };
        assert_eq!(p.capacity, q.capacity);
        for y in 0..10 {
            write(&mut p, 1, y, 0);
            assert_eq!(cp(&q, 1, y), y as u32);
            assert_eq!(cp(&p, 1, y), 0);
        }
    }
    #[test]
    fn page_clone_graphemes() {
        let mut p = page();
        write(&mut p, 0, 0, 9);
        p.append_grapheme(0, 0, 10).unwrap();
        p.append_grapheme(0, 0, 11).unwrap();
        let q = p.clone();
        assert!(q.rows[0].grapheme());
        assert_eq!(q.lookup_grapheme(0).unwrap(), [10, 11]);
    }
    #[test]
    fn page_clone_styles() {
        let mut p = page();
        for x in 0..10 {
            write(&mut p, x, 0, x as u32 + 1);
            p.set_style(x, 0, bold()).unwrap();
        }
        let q = p.clone();
        for x in 0..10 {
            assert_eq!(p.cells[x].style_id(), q.cells[x].style_id());
            assert_eq!(q.style(q.cells[x]), bold());
        }
        q.assert_integrity();
    }
    #[test]
    fn page_clone_from_shrink_columns() {
        let mut p = page();
        for y in 0..10 {
            write(&mut p, 1, y, y as u32);
        }
        let mut q = Page::init(Capacity {
            cols: 5,
            ..p.capacity
        })
        .unwrap();
        q.clone_from(&p, 0, 10).unwrap();
        assert_eq!(q.size.cols, 5);
        for y in 0..10 {
            assert_eq!(cp(&q, 1, y), y as u32);
        }
    }
    #[test]
    fn page_clone_from_partial() {
        let mut p = page();
        for y in 0..10 {
            write(&mut p, 1, y, y as u32);
        }
        let mut q = page();
        q.clone_from(&p, 0, 5).unwrap();
        for y in 0..10 {
            assert_eq!(cp(&q, 1, y), if y < 5 { y as u32 } else { 0 });
        }
    }
    #[test]
    fn page_clone_from_hyperlinks_exact_capacity() {
        let mut p = Page::init(Capacity {
            cols: 50,
            rows: 50,
            ..Capacity::default()
        })
        .unwrap();
        let id = p.insert_hyperlink(link()).unwrap();
        for n in 0..p.hyperlink_capacity() {
            let x = n / 50;
            let y = n % 50;
            write(&mut p, x, y, 42);
            p.hyperlink_set.r#use(id).unwrap();
            p.set_hyperlink(x, y, id).unwrap();
        }
        assert_eq!(p.hyperlink_count(), p.hyperlink_capacity());
        let mut q = Page::init(p.capacity).unwrap();
        q.clone_from(&p, 0, 50).unwrap();
        assert_eq!(q.hyperlink_count(), p.hyperlink_count());
    }
    #[test]
    fn page_clone_from_graphemes() {
        let mut p = page();
        for y in 0..10 {
            write(&mut p, 1, y, y as u32 + 1);
            p.append_grapheme(1, y, 10).unwrap();
        }
        let mut q = page();
        q.clone_from(&p, 0, 10).unwrap();
        for y in 0..10 {
            p.clear_cells(y, 0, 10, false).unwrap();
            assert_eq!(cp(&q, 1, y), y as u32 + 1);
            assert_eq!(
                q.lookup_grapheme(q.cell_index(1, y).unwrap()).unwrap(),
                [10]
            );
        }
        q.assert_integrity();
    }
    #[test]
    fn page_clone_from_frees_dst_graphemes() {
        let p = page();
        let mut q = page();
        for y in 0..10 {
            write(&mut q, 1, y, 9);
            q.append_grapheme(1, y, 10).unwrap();
        }
        q.clone_from(&p, 0, 10).unwrap();
        assert_eq!(q.grapheme_count(), 0);
        for y in 0..10 {
            assert!(!q.rows[y].grapheme());
        }
    }
    #[test]
    fn page_clone_row_from_partial() {
        let mut p = page();
        let mut q = page();
        for x in 0..10 {
            write(&mut p, x, 0, x as u32 + 1);
        }
        q.clone_partial_row_from(&p, 0, 0, 2, 2, 6).unwrap();
        for x in 0..10 {
            assert_eq!(
                cp(&q, x, 0),
                if (2..8).contains(&x) { x as u32 + 1 } else { 0 }
            );
        }
        assert_eq!(q.grapheme_count(), 0);
        assert!(!q.rows[0].grapheme());
        q.assert_integrity();
    }
    #[test]
    fn page_clone_row_from_partial_grapheme_in_non_copied_source_region() {
        let mut p = page();
        let mut q = page();
        for x in 0..10 {
            write(&mut p, x, 0, x as u32 + 1);
        }
        for x in [0, 9] {
            p.append_grapheme(x, 0, 10).unwrap();
        }
        q.clone_partial_row_from(&p, 0, 0, 2, 2, 6).unwrap();
        for x in 0..10 {
            assert_eq!(
                cp(&q, x, 0),
                if (2..8).contains(&x) { x as u32 + 1 } else { 0 }
            );
        }
        assert_eq!(q.grapheme_count(), 0);
        assert!(!q.rows[0].grapheme());
        q.assert_integrity();
    }
    #[test]
    fn page_clone_row_from_partial_grapheme_in_non_copied_dest_region() {
        let mut p = page();
        let mut q = page();
        for x in 0..10 {
            write(&mut p, x, 0, x as u32 + 1);
        }
        for x in 0..10 {
            write(&mut q, x, 0, 0xbb);
        }
        for x in [0, 9] {
            q.append_grapheme(x, 0, 10).unwrap();
        }
        q.clone_partial_row_from(&p, 0, 0, 2, 2, 6).unwrap();
        for x in 0..10 {
            assert_eq!(
                cp(&q, x, 0),
                if (2..8).contains(&x) {
                    x as u32 + 1
                } else {
                    0xbb
                }
            );
        }
        assert_eq!(q.grapheme_count(), 2);
        assert!(q.rows[0].grapheme());
        q.assert_integrity();
    }
    #[test]
    fn page_clone_row_from_partial_hyperlink_in_same_page_copy() {
        let mut p = page();
        for x in 0..10 {
            write(&mut p, x, 0, x as u32 + 1);
        }
        let id = p.insert_hyperlink(link()).unwrap();
        p.set_hyperlink(7, 0, id).unwrap();
        p.clone_cells_within(0, 1, 2, 2, 8 - 2).unwrap();
        for x in 0..10 {
            assert_eq!(
                cp(&p, x, 1),
                if (2..8).contains(&x) { x as u32 + 1 } else { 0 }
            );
        }
        assert_eq!(p.hyperlink_count(), 2);
        assert!(p.rows[1].hyperlink());
        p.assert_integrity();
    }
    #[test]
    fn page_clone_row_from_partial_hyperlink_in_same_page_omit() {
        let mut p = page();
        for x in 0..10 {
            write(&mut p, x, 0, x as u32 + 1);
        }
        let id = p.insert_hyperlink(link()).unwrap();
        p.set_hyperlink(7, 0, id).unwrap();
        p.clone_cells_within(0, 1, 2, 2, 6 - 2).unwrap();
        for x in 0..10 {
            assert_eq!(
                cp(&p, x, 1),
                if (2..6).contains(&x) { x as u32 + 1 } else { 0 }
            );
        }
        assert_eq!(p.hyperlink_count(), 1);
        assert!(!p.rows[1].hyperlink());
        p.assert_integrity();
    }
    #[test]
    fn page_move_cells_text_only() {
        let mut p = page();
        for x in 0..10 {
            write(&mut p, x, 0, x as u32 + 1);
        }
        p.move_cells(0, 0, 1, 0, 10).unwrap();
        for x in 0..10 {
            assert_eq!(cp(&p, x, 0), 0);
            assert_eq!(cp(&p, x, 1), x as u32 + 1);
        }
        p.assert_integrity();
    }
    #[test]
    fn page_move_cells_graphemes() {
        let mut p = page();
        for x in 0..10 {
            write(&mut p, x, 0, x as u32 + 1);
            p.append_grapheme(x, 0, 10).unwrap();
        }
        p.move_cells(0, 0, 1, 0, 10).unwrap();
        for x in 0..10 {
            assert_eq!(cp(&p, x, 0), 0);
            assert_eq!(cp(&p, x, 1), x as u32 + 1);
            assert_eq!(
                p.lookup_grapheme(p.cell_index(x, 1).unwrap()).unwrap(),
                [10]
            );
        }
        p.assert_integrity();
    }
    #[test]
    fn page_verify_integrity_graphemes_good() {
        let mut p = page();
        write(&mut p, 0, 0, 1);
        p.append_grapheme(0, 0, 10).unwrap();
        p.assert_integrity();
    }
    #[test]
    fn page_verify_integrity_grapheme_row_not_marked() {
        let mut p = page();
        write(&mut p, 0, 0, 1);
        p.append_grapheme(0, 0, 10).unwrap();
        p.rows[0].set_grapheme(false);
        assert_eq!(
            p.verify_integrity(),
            Err(IntegrityError::UnmarkedGraphemeRow)
        );
    }
    #[test]
    fn page_verify_integrity_styles_good() {
        let mut p = page();
        for x in 0..10 {
            write(&mut p, x, 0, x as u32 + 1);
            p.set_style(x, 0, bold()).unwrap();
        }
        p.assert_integrity();
    }
    #[test]
    fn page_verify_integrity_styles_ref_count_mismatch() {
        let mut p = page();
        for x in 0..10 {
            write(&mut p, x, 0, x as u32 + 1);
            p.set_style(x, 0, bold()).unwrap();
        }
        p.styles.release(p.cells[0].style_id()).unwrap();
        assert_eq!(
            p.verify_integrity(),
            Err(IntegrityError::MismatchedStyleRef)
        );
    }
    #[test]
    fn page_verify_integrity_zero_rows() {
        let mut p = page();
        p.size.rows = 0;
        assert_eq!(p.verify_integrity(), Err(IntegrityError::ZeroRowCount));
    }
    #[test]
    fn page_verify_integrity_zero_cols() {
        let mut p = page();
        p.size.cols = 0;
        assert_eq!(p.verify_integrity(), Err(IntegrityError::ZeroColCount));
    }
    #[test]
    fn page_exact_row_capacity_empty_rows() {
        let p = page();
        assert_eq!(
            p.exact_row_capacity(0, 5).unwrap(),
            Capacity {
                cols: 10,
                rows: 5,
                styles: 0,
                hyperlink_bytes: 0,
                grapheme_bytes: 0,
                string_bytes: 0
            }
        );
    }
    #[test]
    fn page_exact_row_capacity_styles() {
        let mut p = page();
        assert_eq!(p.exact_row_capacity(0, 5).unwrap().styles, 0);
        p.set_style(0, 0, bold()).unwrap();
        assert_eq!(
            usize::from(p.exact_row_capacity(0, 5).unwrap().styles),
            style::Set::capacity_for_count(1)
        );
        p.set_style(1, 0, bold()).unwrap();
        assert_eq!(
            usize::from(p.exact_row_capacity(0, 5).unwrap().styles),
            style::Set::capacity_for_count(1)
        );
        p.set_style(
            2,
            0,
            Style {
                flags: Flags {
                    italic: true,
                    ..Flags::default()
                },
                ..Style::default()
            },
        )
        .unwrap();
        p.set_style(
            0,
            7,
            Style {
                flags: Flags {
                    underline: ghostty_vt::sgr::Underline::Single,
                    ..Flags::default()
                },
                ..Style::default()
            },
        )
        .unwrap();
        assert_eq!(
            usize::from(p.exact_row_capacity(0, 5).unwrap().styles),
            style::Set::capacity_for_count(2)
        );
        assert_eq!(
            usize::from(p.exact_row_capacity(0, 10).unwrap().styles),
            style::Set::capacity_for_count(3)
        );
        clone_exact(&p, 5);
    }
    #[test]
    fn page_exact_row_capacity_single_style_clone() {
        let mut p = page();
        p.set_style(0, 0, bold()).unwrap();
        let q = clone_exact(&p, 1);
        assert_eq!(q.style(q.cells[0]), bold());
    }
    #[test]
    fn page_exact_row_capacity_styles_max_single_row() {
        let mut p = Page::init(Capacity {
            cols: u16::MAX,
            rows: 1,
            styles: u16::MAX,
            ..Capacity::default()
        })
        .unwrap();
        for x in 0..1000 {
            p.set_style(
                x,
                0,
                Style {
                    fg_color: Color::Rgb(Rgb {
                        r: x as u8,
                        g: (x >> 8) as u8,
                        b: 0,
                    }),
                    ..Style::default()
                },
            )
            .unwrap();
        }
        assert_eq!(
            usize::from(p.exact_row_capacity(0, 1).unwrap().styles),
            style::Set::capacity_for_count(1000)
        );
    }
    #[test]
    fn page_exact_row_capacity_grapheme_bytes() {
        let mut p = page();
        assert_eq!(p.exact_row_capacity(0, 5).unwrap().grapheme_bytes, 0);
        for (x, cp0) in [(0, 97), (1, 101), (2, 111)] {
            write(&mut p, x, 0, cp0);
            p.append_grapheme(x, 0, 0x301).unwrap();
            assert_eq!(
                p.exact_row_capacity(0, 5).unwrap().grapheme_bytes,
                (x as u32 + 1) * 16
            );
        }
        p.append_grapheme(2, 0, 0x302).unwrap();
        p.append_grapheme(2, 0, 0x303).unwrap();
        write(&mut p, 0, 7, 120);
        p.append_grapheme(0, 7, 0x304).unwrap();
        assert_eq!(p.exact_row_capacity(0, 5).unwrap().grapheme_bytes, 48);
        assert_eq!(p.exact_row_capacity(0, 10).unwrap().grapheme_bytes, 64);
        clone_exact(&p, 5);
    }
    #[test]
    fn page_exact_row_capacity_grapheme_bytes_larger_than_chunk() {
        let mut p = page();
        write(&mut p, 0, 0, 97);
        for cp0 in 0x300..0x306 {
            p.append_grapheme(0, 0, cp0).unwrap();
        }
        assert_eq!(p.exact_row_capacity(0, 1).unwrap().grapheme_bytes, 32);
        clone_exact(&p, 1);
    }
    #[test]
    fn page_exact_row_capacity_hyperlinks() {
        let mut p = Page::init(Capacity {
            hyperlink_bytes: 32 * 48,
            string_bytes: 512,
            ..page().capacity
        })
        .unwrap();
        assert_eq!(p.exact_row_capacity(0, 5).unwrap().hyperlink_bytes, 0);
        let id = p.insert_hyperlink(link()).unwrap();
        p.set_hyperlink(0, 0, id).unwrap();
        let one = p.exact_row_capacity(0, 5).unwrap();
        assert_eq!(one.string_bytes, 32);
        assert_eq!(
            usize::from(one.hyperlink_bytes),
            hyperlink::Set::capacity_for_count(1) * 48
        );
        p.hyperlink_set.r#use(id).unwrap();
        p.set_hyperlink(1, 0, id).unwrap();
        assert_eq!(p.exact_row_capacity(0, 5).unwrap(), one);
        let id = p
            .insert_hyperlink(Hyperlink {
                id: HyperlinkId::Explicit(b"my-link-id".to_vec()),
                uri: b"https://other.example.org/path".to_vec(),
            })
            .unwrap();
        p.set_hyperlink(2, 0, id).unwrap();
        assert_eq!(p.exact_row_capacity(0, 5).unwrap().string_bytes, 96);
        let id = p
            .insert_hyperlink(Hyperlink {
                id: HyperlinkId::Implicit(99),
                uri: b"https://outside.example.com".to_vec(),
            })
            .unwrap();
        p.set_hyperlink(0, 7, id).unwrap();
        assert_eq!(p.exact_row_capacity(0, 5).unwrap().string_bytes, 96);
        assert_eq!(p.exact_row_capacity(0, 10).unwrap().string_bytes, 128);
        clone_exact(&p, 5);
    }
    #[test]
    fn page_exact_row_capacity_single_hyperlink_clone() {
        let mut p = page();
        let id = p.insert_hyperlink(link()).unwrap();
        p.set_hyperlink(0, 0, id).unwrap();
        let q = clone_exact(&p, 1);
        assert!(q.cells[0].hyperlink());
    }
    #[test]
    fn page_exact_row_capacity_hyperlink_map_capacity_for_many_cells() {
        let mut p = Page::init(Capacity {
            cols: 50,
            rows: 2,
            hyperlink_bytes: 32 * 48,
            string_bytes: 512,
            ..Capacity::default()
        })
        .unwrap();
        let id = p.insert_hyperlink(link()).unwrap();
        for x in 0..50 {
            if x > 0 {
                p.hyperlink_set.r#use(id).unwrap();
            }
            p.set_hyperlink(x, 0, id).unwrap();
        }
        assert!(p.exact_row_capacity(0, 1).unwrap().hyperlink_bytes >= 4 * 48);
        let q = clone_exact(&p, 1);
        assert_eq!(q.hyperlink_count(), 50);
    }
    #[test]
    fn grapheme_append_preserves_row_dirty_state() {
        let mut p = page();
        p.cells[0] = Cell::init('a' as u32);
        p.rows[0].set_dirty(false);
        for _ in 0..6 {
            p.append_grapheme(0, 0, 0x301).unwrap();
            assert!(!p.rows[0].dirty());
        }
        p.rows[0].set_dirty(true);
        p.append_grapheme(0, 0, 0x302).unwrap();
        assert!(p.rows[0].dirty());
        p.assert_integrity();
    }

    #[test]
    fn hyperlink_dedup_succeeds_with_full_string_allocator() {
        let mut p = Page::init(Capacity {
            cols: 2,
            rows: 1,
            string_bytes: 32,
            ..Capacity::default()
        })
        .unwrap();
        let link = Hyperlink {
            uri: vec![b'a'; Page::layout(p.capacity).string_alloc_layout.bitmap_count * 64 * 32],
            id: HyperlinkId::Implicit(1),
        };
        let id = p.insert_hyperlink(link.clone()).unwrap();
        assert!(!p.can_allocate_string(32));
        assert_eq!(p.insert_hyperlink(link).unwrap(), id);
        assert_eq!(p.hyperlink_set.ref_count(id), 2);
        assert!(!p.can_allocate_string(32));
    }
}
