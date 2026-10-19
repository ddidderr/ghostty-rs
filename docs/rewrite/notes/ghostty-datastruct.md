# ghostty-datastruct

Port of `ZIG/src/datastruct/*`.

| Zig | Rust (`crates/ghostty-datastruct/src`) | Notes |
|---|---|---|
| `split_tree.zig` | `split_tree.rs` | Full port, all 17 tests (+4 extra). |
| `circ_buf.zig` | `circ_buf.rs` | `CircBuf<T>` takes the default value in `new`. All 21 tests (+2). |
| `lru.zig` | `lru.rs` | Slab + index linked list. All 4 tests (+1). |
| `cache_table.zig` | `cache_table.rs` | `CacheContext` trait (hash/eql/evicted). 1 test (+2). |
| `blocking_queue.zig` | `blocking_queue.rs` | `Mutex` + `Condvar`; explicit close wakes every blocked producer. Shutdown regression covers indefinite/timed waits and rejection after draining. |
| `segmented_pool.zig` | `segmented_pool.rs` | Index handles instead of pointers. 1 test. |
| `message_data.zig` | `message_data.rs` | enum `Small/Stable/Alloc`. 3 tests (+1). |
| `comparison.zig` | `comparison.rs` | `DeepEqual` / `ApproxEq` traits. All 24 tests. |
| `array_list_collection.zig` | `array_list_collection.rs` | 0 Zig tests (1 added). |
| `intrusive_linked_list.zig` | **not ported** | Only user is `lru.zig`; replaced by an index-based list. Its 1 test is skipped. |
| `main.zig` | `lib.rs` | `refAllDecls` test skipped (Zig only). |

Zig tests: 73 ported (all), 2 skipped (`intrusive_linked_list` basic test,
`main.zig` refAllDecls). Total Rust tests: 88.

## SplitTree design / deviations

* View type: Zig stores `*View` with `ref`/`unref`. Rust: `V: SplitTreeView`
  (`Clone` + optional `split_tree_label() -> Option<String>`). `Clone` is the
  ref, `Drop` is the unref (use `Rc`/`Arc`/GObject handles inside the view).
  Allocation is infallible, so no allocator parameters; the arena is a `Vec`.
  `SplitTree::new(&view)` replaces `init`, `empty()` replaces `.empty`,
  `Clone` replaces `clone`, `Drop` replaces `deinit`.
* Only `split` can fail (`SplitError::OutOfMemory` when > 65535 nodes, as in Zig).
* `Handle` is a `u16` newtype (`Handle::ROOT`, `idx()`, `offset()`,
  `from_idx()`); `nodes()`/`zoomed()` are read-only accessors.
* Ratios and spatial slots use `half::f16` exactly like Zig's `f16` (so
  rounding in `{d:.2}` output, `resize`, spatial navigation match).
* `goto` no longer takes an allocator; spatial navigation allocates a `Vec`.
* Formatting: `Display` = diagram + text (like `{f}`); `format_diagram`,
  `format_text` (writer based), `diagram_string()` / `text_string()` helpers
  replace `std.fmt.alt(t, .formatDiagram/.formatText)`. If a view has no label,
  the node index is used (digit width = `log10(nodes)+1`).
* `getGObjectType` (GTK boxed type glue) is not ported: the GTK crate should
  wrap `SplitTree<V>` itself (e.g. a `glib::Boxed` newtype using `Clone`).

## Other deviations

* `lru`: `V: Default` is required (the new slot is default-initialised where
  Zig leaves it undefined). `GetOrPutResult.evicted` contains the evicted
  entry's **old key** and old value (Zig set the new key before copying the
  entry out, which looks like a bug; only `.value` is used in tests). `get`
  returns `Option<&V>` and does not affect LRU order (as in Zig).
  `resize` returns the evicted values (oldest first) as `Option<Vec<V>>`.
* `cache_table`: `get` returns `Option<&V>` (moves the hit to the bucket end);
  buckets are `Vec`s instead of fixed arrays; `K`/`V: Clone` for `put` because
  the eviction hook and the return value both receive the evicted pair.
* `blocking_queue`: `create`/`destroy` -> `new()`/`Arc`; `drain()` returns a
  guard iterator that releases the lock (and signals waiting producers) on drop.
  Blocking pushes use `wait_while`, so spurious wakeups no longer fail a push.
* `segmented_pool`: `get()` returns a slot index; use `at`/`at_mut`.
* `message_data`: `Stable` holds `&'static [T]`; `Alloc` holds a `Vec`.
* `comparison`: Zig reflection (`deepEqual` over any type) is replaced by traits;
  implement `DeepEqual` for custom types (rarely needed — prefer `PartialEq`).
  `expect_approx_equal` panics instead of returning an error; `@Vector` and
  anonymous structs map to arrays/tuples.

## TODO(integration)

None.
