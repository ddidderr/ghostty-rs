# Differential fuzzing of the terminal core (agent DF)

Zig (Ghostty 1.3.1, the reference) vs Rust (`ghostty-terminal` + `ghostty-vt`
read-only stream), on random escape-heavy byte streams.

## Layout

| Path | What |
|---|---|
| `tools/difftest/setup.sh` | Idempotent setup: Zig 0.15.2 -> `target/zig-0.15.2/`, a COPY of the Zig source -> `target/zig-src/`, builds the Zig driver (ReleaseSafe -> `target/difftest-zig/`, ReleaseFast -> `target/difftest-zig-fast/`). Nothing in `/tmp`, reference tree untouched. |
| `tools/difftest/zig/` | Zig driver (`build.zig`, `src/main.zig`), depends on the source copy via a path dependency and the `ghostty-vt` module (SIMD enabled, like a default Ghostty build). |
| `tools/difftest/src/lib.rs`, `bin/difftest_rs.rs` | Rust twin: `difftest_rs <cols> <rows> <max_scrollback> <file>`; `lib.rs::dump` is the canonical dump (must stay byte-identical to the Zig one). |
| `tools/difftest/src/inputgen.rs` | Deterministic (splitmix64) escape-heavy generator: ASCII/UTF-8/wide/combining/emoji/ZWJ/invalid UTF-8/C1 bytes, C0 controls, CSI with edge-value params and intermediates, SGR (incl. colon forms), DECSET/DECRST/XTSAVE/XTRESTORE/DECRQM, ESC sequences, charsets, OSC (colors, hyperlinks, pwd, 133...), DCS, APC, tabs, scroll regions + left/right margins, IL/DL/ICH/DCH/ECH/REP, alt screen, resets, byte mutations. Geometry (1x1 .. 300x3) and scrollback (0..100000) vary per case. |
| `tools/difftest/src/bin/difftest_fuzz.rs` | Harness: `run` (N cases, threads, seed, `--size CxR`, `--sb`, `--tokens`, `--zig PATH`), `check`, `min` (ddmin minimiser on divergence), `minz`/`minr` (minimise an abort). |
| `tools/difftest/triage.sh` | Minimise every saved failure and print the first differing lines. |
| `just difftest [CASES] [SEEDS]` | Setup + run in both assertion modes. |

Failures are saved in `target/difftest-work/fails-<seed>/` (`.bin`, `.zig.txt`, `.rs.txt`).

## Dump format

Per screen (primary + alternate if it exists): cursor (x, y, pending wrap,
protected, style, hyperlink flag), saved cursor, charset state, protected mode,
kitty keyboard flags, every row of scrollback+active (row flags wrap /
wrap_continuation / semantic_prompt / kitty placeholder; every non-blank cell:
content tag, codepoint(s) + grapheme tail, wide, protected/hyperlink flags,
semantic content, full style, hyperlink URI + id). Terminal level: size, active
screen, scrolling region, all mode values + saved modes, tabstops, mouse/shell
flags, status display, mouse shape, pwd, previous char, dynamic colors,
modified palette entries, plus the formatter's plain text (`plainString`).

## Comparison rules

* Identical dump = pass.
* If the Zig driver aborts (failed assertion / segfault in Zig code, e.g.
  Devanagari conjunct at the right edge with left/right margins, REP with a
  huge count while a page grows under a hyperlink, grapheme-break assertion in
  `Terminal.print`) there is no defined behaviour to compare: counted under
  "zig aborted" and not a divergence.
* A Rust panic where Zig succeeded is a divergence.
* Two passes: assertions on (Rust debug vs Zig ReleaseSafe) and assertions off
  (Rust `production` profile vs Zig ReleaseFast).

## Divergences found and fixed

1. **UTF-8 decoding of ill-formed input** (`ghostty-vt/src/simd.rs`,
   `stream.rs`). The Rust port modelled Zig's *scalar fallback*
   (`utf8DecodeUntilControlSeqScalar`), but a default Ghostty build runs the
   C++ SIMD path (`vt.cpp`: simdutf + utfcpp `replace_invalid` +
   `trim_partial_utf8`). Differences: ONE U+FFFD per invalid sequence with all
   following continuation bytes swallowed (`c0 a9`, `e4 a9 f0`, ...); decoding of
   everything before an ESC without deferring truncated tails; byte-value based
   tail trimming when no ESC; and the remaining tail (not just one byte) goes to
   the scalar decoder. Rewrote `utf8_decode_until_control_seq` and
   `next_slice_capped` to mirror C++/Zig. Tests: `simd::tests::*`,
   `stream_readonly::tests::difftest_invalid_utf8_*`.
2. **`PageList::erase_rows` stale row flags** (`ghostty-terminal/src/page_list.rs`).
   Zig swaps surviving rows into place and only clears the leftover rows' cells,
   so e.g. `wrap` stays set on rows that later become visible again when the
   page regrows (CSI 3 J then more output). Rust rotated and cleared. Now swaps
   exactly like Zig for erases starting at the top of a page. Test:
   `difftest_erase_scrollback_keeps_stale_row_flags`.
3. **Grapheme move on wrap with left/right margins** (`terminal.rs`
   `print_grapheme`). Zig moves the grapheme from the original `prev.cell`
   pointer; Rust used `old_pin = new_pin.up(1)` with `x = right_limit - 1`,
   which is a different cell when margins stop the wrap from scrolling the
   previous cell (stream returned `InvalidCoordinate`). Now remembers the
   original cell index. Test: `difftest_grapheme_wrap_with_margins_moves_prev_cell`.

## Upstream bugs found (Zig memory-safety / UB; NOT replicated in Rust)

1. **Stale cell pointer after page growth in the grapheme-attach path**
   (`src/terminal/Terminal.zig:482` sets `prev.cell = cursor.page_cell`, then
   `printCell` of the spacer tail can call `increaseCapacity` (hyperlink/string
   bytes) which reallocates the page, and `:539`
   `appendGrapheme(prev.cell, c)` writes through the dangling pointer; the
   grapheme is lost). Repro (5x30, scrollback 1): the 107-byte minimized stream
   in `stream_readonly::tests::difftest_upstream_uaf_grapheme_survives_page_growth`
   (a hyperlinked cell then `U+2764 U+FE0F` wrapping at the edge). Rust keeps the
   grapheme. Harness: mismatches where Zig logged `adjusting page capacity` are
   saved as `suspect*` and not counted as divergences (see `zig_run`).
2. Zig aborts / segfaults (not compared): the `assert(!graphemeBreak(...))`
   in `Terminal.print` (`Terminal.zig:375`) after mode 2027 with some
   U+2028/U+2060/BOM sequences; `assert(y < size.rows)` (`page.zig:1012`) and
   segfaults for Devanagari conjuncts at the right edge with left/right margins;
   `assert(!cell.hyperlink)` (`Terminal.zig:834`) / segfault on REP with a huge
   count while a hyperlinked page grows. Repro e.g. 20x8:
   `ESC[?2027h ESC[?69h ESC[;10s ESC[50;41H` + U+0915 U+094D U+0937
   (Zig faults; Rust now handles it, test
   `difftest_grapheme_wrap_with_margins_moves_prev_cell`).

## Known Zig-side aborts (not Rust bugs)

See "Comparison rules". Roughly 0.1-0.4 % of generated cases (mostly the
Devanagari/margins one, which the generator hits on purpose) abort in Zig.

## Final run

Final campaign (after all fixes): 10 geometries x 20k (assertions on), 5 x 20k
(assertions off, production Rust vs ReleaseFast Zig), 3 x 30k mixed geometries
= 390k cases, 0 divergences, 2 suspects (both the upstream UAF above, 0.1-0.5 %
of cases abort in Zig and are skipped). Not done: replaying Zig test-suite
inputs (those tests are already ported as Rust unit tests).

`cargo test --workspace --no-fail-fast`: everything passes except
`ghostty-rs --test gtk_replay` (needs a GPU/Vulkan: `ZINK: vkEnumeratePhysicalDevices
failed`), an environment limitation unrelated to this work.
