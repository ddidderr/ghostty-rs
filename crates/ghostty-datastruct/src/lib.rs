//! The datastruct package contains data structures or anything closely
//! related to data structures. Port of `src/datastruct/*`.
//!
//! `intrusive_linked_list.zig` is intentionally not ported: its only user
//! (`lru.zig`) is implemented with an index-based list here. See
//! `docs/rewrite/notes/ghostty-datastruct.md`.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::float_cmp,
        clippy::assert_is_empty,
        clippy::redundant_closure_for_method_calls
    )
)]

pub mod array_list_collection;
pub mod blocking_queue;
pub mod cache_table;
pub mod circ_buf;
pub mod comparison;
pub mod lru;
pub mod message_data;
pub mod segmented_pool;
pub mod split_tree;

pub use array_list_collection::ArrayListCollection;
pub use blocking_queue::BlockingQueue;
pub use cache_table::CacheTable;
pub use circ_buf::CircBuf;
pub use message_data::MessageData;
pub use segmented_pool::SegmentedPool;
pub use split_tree::SplitTree;

/// Stand-in for Zig's `Allocator.Error.OutOfMemory`, returned where a fixed
/// capacity structure cannot accept more data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("out of memory")]
pub struct OutOfMemory;
