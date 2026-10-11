//! Compatibility wrappers for the native [`Pin`] iterators.
pub use crate::page_list::{CellIterator as CellIter, RowIterator as RowIter};
use crate::page_list::{Direction, PageList, Pin, PromptIterator};

/// Iterate rows through an inclusive limit row.
pub fn row_iterator(
    list: &PageList,
    start: Pin,
    direction: Direction,
    limit: Option<Pin>,
) -> RowIter<'_> {
    start.row_iterator(list, direction, limit)
}
/// Iterate cells through an inclusive limit row.
pub fn cell_iterator(
    list: &PageList,
    start: Pin,
    direction: Direction,
    limit: Option<Pin>,
) -> CellIter<'_> {
    start.cell_iterator(list, direction, limit)
}
/// Iterate prompt starts through an inclusive limit row.
pub fn prompt_iterator(
    list: &PageList,
    start: Pin,
    direction: Direction,
    limit: Option<Pin>,
) -> PromptIterator<'_> {
    start.prompt_iterator(list, direction, limit)
}
