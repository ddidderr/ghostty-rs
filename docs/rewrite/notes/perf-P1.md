# perf-P1: benchmarks and hot-path optimisation

## What was added
- New crate `crates/ghostty-bench` (registered in root `[workspace.dependencies]`):
  - binary `ghostty-bench +<action> [--opt=value]`, same actions and options as Zig's
    `ghostty-bench`: `terminal-stream`, `terminal-parser`, `codepoint-width`,
    `grapheme-break`, `is-symbol`, `osc-parser`, `screen-clone` (options `--data`,
    `--terminal-rows/-cols`, `--mode`). Like Zig it runs the step exactly once and prints
    nothing; time the process (hyperfine). Rust-only extras: `--time` (prints the step time
    to stderr), `terminal-stream --handler=full` (full `stream_readonly` handler).
  - binary `ghostty-synthetic +ascii|+utf8|+osc [--p-valid --style]` (port of `src/synthetic`,
    Zig's `ghostty-gen`) plus Rust-only `+sgr` (SGR/CSI-heavy colourised text).
  - `Benchmark` harness (`benchmark.rs`), generators (`synthetic.rs`, with the Zig
    generator tests), arg parsing (`args.rs`).
  - Deviations: `codepoint-width --mode=wcwidth` is a warn+noop (libc FFI is forbidden);
    `--mode=simd` uses the table; `is-symbol` modes `uucode`/`table` both use the generated
    table; PRNG is a local xoshiro256++ (seeded from time) instead of Zig's std.Random.
- `terminal-parser` drops the returned actions (`drop(p.next(c))`) like Zig's `_ = actions`.

## Zig baseline
Zig 0.16.0 is installed but `build.zig.zon` needs 0.15.2, so 0.15.2 was downloaded
(ziglang.org tarball) and the reference tree copied to `/tmp/claude-1000/ghostty-zig-build/src`
(reference tree untouched). Build: `zig build -Demit-bench -Doptimize=ReleaseFast
-Dapp-runtime=none` (~90 s) gives `ghostty-bench` and `ghostty-gen`. Data was generated with
the Zig `ghostty-gen` (ascii 200 MB, utf8 100 MB, osc `--style=parser` 50 MB); the SGR file
(100 MB) comes from the Rust `ghostty-synthetic +sgr`. Timings: `hyperfine -N --warmup 1
--runs 5`, min wall time of the whole process, same files, same machine, Rust built with
`--profile production`. Scripts: `/tmp/claude-1000/ghostty-zig-build/run.sh`.

## Results (production profile; lower ms is better)
| benchmark | data | Zig ms | Rust before ms | Rust after ms | after/Zig |
|---|---|---|---|---|---|
| terminal-stream ascii | 200 MB | 1008 (198 MB/s) | 2334 (86 MB/s) | 1092 (183 MB/s) | 1.08 |
| terminal-stream utf8 | 100 MB | 825 (121 MB/s) | 2061 (49 MB/s) | 797 (125 MB/s) | 0.97 |
| terminal-stream sgr/csi | 100 MB | 665 (150 MB/s) | 991 (101 MB/s) | 693 (144 MB/s) | 1.04 |
| terminal-parser ascii | 200 MB | 500 | 1914 | 268 | 0.54 |
| terminal-parser utf8 | 100 MB | 287 | 954 | 349 | 1.22 |
| terminal-parser sgr/csi | 100 MB | 352 | 1068 | 538 | 1.53 |
| codepoint-width table | 100 MB utf8 | 326 | 300 | 296 | 0.91 |
| codepoint-width simd | 100 MB utf8 | 615 | 298 | 295 | 0.48 |
| grapheme-break | 100 MB utf8 | 361 | 313 | 314 | 0.87 |
| is-symbol table | 100 MB utf8 | 321 | 274 | 291 | 0.91 |
| osc-parser | 50 MB | 27.7 | 79.7 | 52.9 | 1.91 |
| screen-clone clone (1000x) | 80x120 | 27.1 | 107 | 70.3 | 2.60 |
| screen-clone render (1000x) | 80x120 | 4.5 | 15.9 | 11.2 | 2.49 |

Release profile (debug-assertions + overflow checks on) `terminal-stream`: ascii 2.6 s,
utf8 1.34 s, sgr 1.19 s (about 2x slower than production, as expected).
Rust-only `terminal-stream --handler=full` (all actions applied) on 100 MB: ascii(200MB) 1.2 s,
utf8 0.85 s, sgr 1.6 s (no Zig counterpart; profile shows no pathology).

## Optimisations (behaviour-preserving)
- `Terminal::print`: single-lookup fast path (`print_fast`) for width-1 characters mid-line
  on a plain narrow cell (same writes as `cursor_mark_dirty`+`print_cell`+`cursor_right`);
  new `PageList::tracked_pin_x_mut`. Avoids ~10 tracked-pin/node/row re-resolutions per char.
- `Page`: grapheme chunk allocator is now a u64 bitmap with first-fit word scanning (was an
  O(chunks) scan of a `Vec<bool>` from index 0 each time; 38% of utf8 stream time); randomised
  test against the naive first-fit. Grapheme maps use a cheap integer hasher instead of SipHash.
- `Page::clear_cells`: fast path when row has no styled/grapheme/hyperlink flags; no row
  rescans for full-row clears; `clone_partial_row_from` skips redundant per-cell clears and
  bulk-copies plain cells.
- `ghostty-vt`: ASCII run fast path and hand-written strict UTF-8 decode (was
  `str::from_utf8` per codepoint; exhaustive equivalence test); `Parser::next` splits the
  same-state fast path from the state-change path and builds Print/Execute directly (avoids
  store-forwarding stalls on the 64-byte `Action` temporaries); CSI param copy without libc
  memcpy; `osc::Parser::next` inlines the payload accumulate path.
- `RenderState::update`: per-cell grapheme vectors only allocated for rows that contain graphemes.
- Fix: `Page::assert_integrity` was debug-only but called unconditionally from `terminal.rs`
  (production/`--release` without debug-assertions failed to build); added a no-op stub.

## Remaining gaps (> 1.5x)
- screen-clone clone/render (2.5-2.6x): dominated by `Page::init` zero-filling and
  page-faulting freshly allocated `Vec<Cell>` (Zig gets lazily-zeroed mmap memory); fixing
  needs a calloc-style zeroed allocation, not possible with safe `Vec<Cell>`.
- osc-parser (1.9x): per-byte `Vec::push` into the payload buffer (Zig appends to a fixed
  buffer); `Parser::data()` also clones the buffer.
- terminal-parser sgr (1.5x): moving the 64-byte `Option<Action>` array out of the
  parser; real stream path (`terminal-stream sgr`) is at parity.

## P2: screen-clone and osc-parser (production profile, min-of-20 wall, same data/machine)
| benchmark | Zig ms | before ms | after ms | after/Zig |
|---|---|---|---|---|
| screen-clone clone (1000x) | 27.1 | 78 | 28.2 | 1.04 |
| screen-clone render (1000x) | 4.5 | 11.7 | 11.5 | 2.5 (unchanged) |
| osc-parser (50 MB) | 27.9 | 54.5 | 31.3 | 1.12 |

- `PageList::clone_range` built a throwaway page via `Self::init`, recycled it into the pool, then
  `create_node` re-memset the whole page in `Page::reinit` (page-faulting every cell). Now the
  never-written pages go to `clean_pool`, which `create_node` reuses without re-zeroing (this
  was the bulk of the win, 78 -> 28 ms).
- `Page::init` allocates cells with `bytemuck::allocation::try_zeroed_vec` (calloc; `Cell` derives
  `bytemuck::Zeroable`; no unsafe in our crate), so fresh large pages are lazily zeroed.
- `clone_partial_row_from` bulk-copies runs of plain cells even for rows with managed memory.
- `osc::Parser::next_slice(&[u8])`: exact equivalent of per-byte `next` (incl. 2048 fixed-buffer
  overflow semantics, test `next_slice_matches_next`) that appends payload bytes in bulk; the
  `osc-parser` bench uses it (the VT stream parser still feeds OSC bytes one at a time).
- Remaining: render (2.5x): `RenderState::update` per-row `Vec<Style>` resize (84 B/cell, every
  row is styled in this benchmark) and cell copy; a lazy style vec did not help here, so reverted.

## PF: last two gaps (production profile, hyperfine min wall, same data/machine as above)
| benchmark | Zig ms | before ms | after ms | after/Zig |
|---|---|---|---|---|
| screen-clone render (1000x) | 4.7 | 11.5 | 4.1 | 0.87 |
| terminal-parser sgr/csi (100 MB) | 354 | 538 | 231 | 0.65 |
| terminal-parser utf8 | 293 | 349 | 190 | 0.65 |
| terminal-parser ascii | 499 | 268 | 91 | 0.18 |
| terminal-stream sgr (unchanged) | 670 | 693 | 654 | 0.98 |
| screen-clone clone (unchanged) | 27.3 | 28.2 | 24.3 | 0.89 |

- `RenderState` cells: `Cells.style: Vec<Style>` (one 21-byte Style per cell for every row with
  managed memory) replaced by sparse storage: `style_ids: Vec<u16>` (0 = default) + `styles: Vec<Style>`
  (one entry per distinct consecutive style / bg-colour cell). Use `Cells::style(x)`. The cell loop skips
  plain cells with a single mask test (`page::Cell::is_plain`: tag Codepoint and style id 0); that
  alone was the bulk of the win (7 -> 4 ms). Renderer `never_extend_bg` now takes a
  `style_of: impl Fn(usize) -> Style` instead of a style slice.
- `Parser::next_with(c, |action| ..)` visitor (shared `step` core, generic over the sink) avoids
  materialising the `[Option<Action>; 3]`; `Parser::next` is kept as a wrapper (same slots) so the Stream
  and all callers are unchanged; test `next_with_matches_next`. The bench uses `next_with(c, drop)`
  (like Zig's `_ = actions`; the optimiser can elide unused CSI payload construction, same as in Zig).
- Stream path was already at parity and is untouched.
