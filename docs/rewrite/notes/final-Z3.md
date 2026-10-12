# Z3 final fixes (vt-capi, shadertoy, tripwire, crash, kitty debug dump, bench signpost)

Supersedes the stale statements in libghostty-vt.md ("interior collections use the global allocator"),
crash-CR.md ("no minidump") and macos-M1-renderer.md ("iChannelTime stride limitation").

## 1. C allocator governs parser storage (lib/allocator.zig, terminal/c/osc.zig)
* `ghostty-vt::osc` gets a small allocator abstraction: trait `ByteStorage` (fallible `try_reserve`/`try_extend`,
  `reset` releases growth) behind `Parser::with_storage`; default stays a `Vec<u8>` (also implements the trait).
  A failed grow sets the parser state invalid, exactly like Zig's `WriteFailed`; a failed 2048-byte reserve when
  switching to the allocating writer falls back to the fixed buffer (Zig `initCapacity catch`).
* `ghostty-vt-capi/alloc.rs`: `CByteStorage` (payload buffer, 2048 bytes allocated at `ghostty_osc_new`, growth by
  alloc-copy-free through the vtable) and `CBlock` (NUL-terminated title copy). `ghostty_osc_new` returns
  `GHOSTTY_OUT_OF_MEMORY` when the parser object or its buffer cannot be allocated; if the title copy fails,
  `ghostty_osc_end` reports an invalid command. `OscCommand` now holds the type key + C-allocated title only.
* Remaining global-allocator use: transient `Command` payload copies made inside `ghostty_osc_end` (Zig borrows
  the parser buffer); they never outlive the call.
* Tests: `tests/c/alloc_test.c` + `tests/c_allocator.rs` (counting/failing/limiting C allocator: leak-free
  accounting, OOM at each creation allocation, live-bytes limit invalidating a large OSC 52, title-copy failure,
  mid-payload failure then reuse); unit test `storage_failure_invalidates_osc` in ghostty-vt.

## 2. Shadertoy
* `shadertoy_sampler.rs`: functions keyed by full signature (per-parameter sampler layout, all overloads kept);
  a call picks the overload by arity and which arguments are samplers (`iChannelN` or a sampler param of the enclosing
  function). Tests `overloads_are_keyed_by_signature`, `overloads_through_sampler_parameters`.
* `shadertoy.rs`: `msl_from_compiled` post-processes naga's MSL: uniform-block scalar arrays with std140 stride
  (`iChannelTime[4]`, found by reflection over the naga module, not by name) become `metal::vec<T,N> inner[K]` wrappers
  and element reads get `.x`. Limitation: copying the whole array into a local variable is not rewritten.
  Test `msl_uniform_block_matches_std140_upload_layout` parses the emitted `struct Globals`, computes MSL
  offsets/size and compares every member to `offset_of!(Uniforms, ..)` and `size_of::<Uniforms>()` (4496).

## 3. Tripwire (tripwire.zig)
* `ghostty-terminal/src/tripwire.rs`: `tripwire_module!(name, PointEnum, Error)` -> `name::{check, error_always,
  error_after, end(ResetMode), reset}`; `end` returns `UntrippedError` (warn-logs untripped points); state is
  thread-local so parallel tests are isolated; `check` is a no-op outside `cfg(test)`. 5 tests ported/added
  (the 3 Zig tests + retain/reset + thread isolation).
* Ported OOM tests: `Tabstops: resize alloc failure preserves state` (tabstops.rs, `resize_tw`);
  `PageList init error` (page_list.rs, `init_tw` + `init_pages_tw`; Zig `init_memory_pool` has no Rust analogue
  (no pool), `viewport_pin`/`viewport_pin_track` are both checked before the single `track_pin` reserve);
  both `reloadActive partial history cleanup` tests (search/screen.rs, `reload_active_tw`; assert OOM is returned
  and `history_results` is unchanged).
* Not portable (infallible Rust code, documented): `selectionString map allocation failure cleanup`
  (`selection_string*` return `String`/`Vec`, no fallible copy; nothing to unwind). Font: Atlas `init/reserve/grow`
  errors (Atlas storage is infallible `Vec`) and SharedGrid `init error` (no allocation points; the
  render-rollback test already exists as `render_glyph_error_after_cache_insert_rolls_back_cache_entry`).
  ghostty-font can use `ghostty_terminal::tripwire` if fallible paths are added later.
* Fixed the `enum_variant_names` clippy lint reported on `InitPagesPoint` (variants renamed `Node/BufStd/BufNonStd`).

## 4. Crash minidump (crash/sentry.zig)
* Linux fatal-signal path: handler keeps building head+event in static memory, then captures `siginfo` +
  machine context (+FP state) into a static `crash_context::CrashContext` and calls `minidump::capture`: raw
  `clone` (fork semantics, own 4 MiB mmap'd stack because the handler runs on a tiny altstack, no atfork handlers),
  `PR_SET_PTRACER_ANY`, parent polls `waitpid` with a 20 s timeout. The child uses `minidump-writer 0.13`
  (`MinidumpWriterConfig` + `set_crash_context`, the crate's documented clone-and-ptrace pattern for the local
  process) and writes the full envelope: event + `minidump.dmp` attachment (`attachment_type: event.minidump`).
  Any failure falls back to the previous event-only envelope. Stack overflow: if the SP page is unreadable
  (guard page) the writer is retried with SP moved to the first readable page above it (reported RSP is then
  approximate, stack contents captured).
* Tests (`crates/ghostty-crash/tests/minidump.rs`): SIGSEGV, SIGABRT and stack overflow on a spawned thread;
  parse with the `minidump` crate; assert thread list contains the crashing tid with stack memory and the main thread,
  exception stream blames that tid with the signal number and registers, modules/memory lists present.
  Existing root `tests/crash.rs` still pass.
* Not done: macOS/Windows native minidumps (sentry-native uses its own backend there); no symbol upload.

## 5. kitty debugDump, bench signpost
* `LoadingImage::{debug_dump_filename, debug_dump_to, debug_dump}` (debug builds only), same file name format as Zig; test.
* `ghostty-bench/src/signpost.rs` (apple only): `os_log_create` + `os_signpost_id_make_with_pointer` +
  `_os_signpost_emit_with_name_impl` interval begin/end around the benchmark loop, subsystem `com.mitchellh.ghostty`,
  category PointsOfInterest, name `ghostty`. Compile/clippy-checked for aarch64-apple-darwin only (not run).

## Gate
workspace `cargo clippy --all-targets -D warnings`, `cargo +nightly fmt --check`, `cargo test --workspace` pass;
aarch64-apple-darwin clippy passes for bench, renderer, crash, terminal, vt-capi.
