# Visual parity vs original Ghostty 1.3.1 (agent VP)

## Method (`scripts/visual-parity.sh`, `just visual-parity [scenario...]`)

- A private headless `weston` (`--backend=headless --renderer=gl`, 1200x800, `desktop-shell`, no
  panel) per run, on its own socket in `$XDG_RUNTIME_DIR=/tmp/vp-runtime-$UID` (socket paths must be
  <108 bytes). The user's desktop is never touched. Mesa is forced for weston (`__EGL_VENDOR_LIBRARY_FILENAMES`
  -> 50_mesa.json); the NVIDIA EGL crashes Xwayland's glamor. `XWAYLAND_NO_GLAMOR=1`.
- `weston --debug` is what authorizes `weston-screenshooter` (without it: "Output capture error: unauthorized").
- Both apps run with `env -i`, `GSETTINGS_BACKEND=memory`, no D-Bus, isolated `XDG_CONFIG_HOME` with the
  same explicit config (JetBrains Mono 13, bg/fg, padding 8, `maximize = true` so the window geometry
  is deterministic, `cursor-style-blink = false`, `title = Ghostty`, no shell integration) and
  `LANG=C.UTF-8` (otherwise `printf '\U..'` in the test page prints literally).
- Scenarios: `page` (block cursor), `page-bar` (bar cursor), `tabs` (new tab + split right + split down,
  driven with `xdotool` through Xwayland since the original Ghostty has no input replay; Rust is driven
  the same way so both apps are treated identically).
  Test page: `scripts/visual-parity/testpage.sh` (ASCII, ligatures, bold/italic/faint/inverse/strike/
  overline, all underline styles + colored, 16/256/truecolor, box drawing, blocks, braille, powerline,
  nerd font icons, CJK, emoji, wide chars, math, cursor). Not covered: mouse selection (no input
  injection into the original on Wayland; could be added through the Xwayland path).
- `scripts/visual-parity/compare.py`: per-pixel max-channel delta, % differing, % > 8 / > 32 levels, max,
  mean, red heat-map PNG; run on the full frame, the terminal area and the headerbar strip.
  `scripts/visual-parity/zoom.py`: ref | rust | diff crops, upscaled.
- Output (not committed): `target/visual-parity/<scenario>/{ref,rust,diff,diff-term,diff-chrome}.png`,
  `target/visual-parity/stats.txt`. `RUST_BIN` defaults to `target/production/ghostty`.
  Iteration build: `CARGO_TARGET_DIR=target/vp CARGO_PROFILE_PRODUCTION_LTO=off
  CARGO_PROFILE_PRODUCTION_CODEGEN_UNITS=16 CARGO_PROFILE_PRODUCTION_INCREMENTAL=true cargo build --profile production`.

## Results (% of pixels differing at all; max channel delta)

| scenario | before | after |
|---|---|---|
| page (full frame) | 0.130% (1250 px, max 93) | 0.000% (0 px; was 0.020% / 196 px before Z15) |
| page-bar | same as page | 0.000% |
| tabs + splits | 44.6% (a whole pane blank, bug) | 0.000% (bit-identical) |
| chrome (headerbar, tab bar) | 0.000% | 0.000% |

Sprite sheets (`ghostty-font` tests, Zig reference PNGs): differing pixels 5.76% -> 1.20% -> 0 (bit-exact after Z15;
test tolerance is now zero).

Font metrics, cell size, baseline, glyph rasterization, colors, cursor, underline positions, padding,
GTK CSS/headerbar/tab bar were already pixel-identical; no change was needed there.

## Fixes

1. **Split rebuild blanked an unchanged pane** (`ghostty-core/src/surface/mod.rs`, `draw_frame`): the GTK
   `render` handler called `renderer.draw_frame(false)`; Zig's `glRender` uses `drawFrame(true)`. With
   `sync = false` and nothing dirty the renderer only re-presents the last target, which is cleared when
   the GLArea is re-realized (nested split = tree rebuild = unrealize/realize of every surface whose size
   did not change), so that pane stayed empty until something changed. Now `true` like Zig.
2. **Sprite antialiasing** (`ghostty-font/src/sprite/canvas.rs`): replaced tiny-skia's coverage with a port of
   z2d 0.10's default `multisample_4x` rasterizer (4x4 samples, scanline spans with edge x rounded at
   sample row centers, non-zero filter, alpha = `coverage * 16 - 1`, full = 255, z2d's Bezier
   flattening with tolerance 0.1). (Stroke outlines were still tiny-skia's until Z15, which ported z2d's stroker.)

## Remaining differences

None (Z15). z2d's stroke plotter (pen, joins, caps, spline flattening), Cairo-style arc segmentation and
z2d's truncating integer compositing were ported (`ghostty-font/src/sprite/stroke.rs`, `canvas.rs`;
see `final-Z15.md`). Final `just visual-parity`: page, page-bar and tabs are 0 differing pixels on the
full frame, the terminal area and the chrome (max delta 0). All sprite sheets in the `ghostty-font`
reference test are bit-exact (tolerance now zero).
