# ghostty-font — F1 (faces, discovery, collection, resolver, shaper, shared grid)

Linux-only backend (`fontconfig_freetype`): fontconfig discovery, FreeType
rasterization, HarfBuzz shaping. FFI via `freetype-sys`, `harfbuzz-sys`,
`yeslogic-fontconfig-sys` (the crates.io `fontconfig-sys` 2.11 does not parse with
current cargo). Crate lint is `unsafe_code = "deny"`; unsafe is confined to
`face/freetype/ft.rs`, `fontconfig.rs`, `shaper/hb.rs` (module-level allow, every
block has a `// SAFETY:` comment). Embedded fonts + licenses live in `res/`
(JetBrains Mono 2.304 variable/static [OFL], Symbols Nerd Font 3.4.0 [MIT], Noto
emoji and the test fonts from `ZIG/src/font/res`).

## Module map (Zig -> Rust, `crates/ghostty-font/src/`)
| Zig | Rust |
|---|---|
| main.zig (Style, Presentation, sprite_index, options) | lib.rs, backend.rs (`backend.zig`) |
| library.zig | library.rs (`Library`, Clone/Arc, face-creation mutex) |
| face.zig | face.rs (`Options`, `DesiredSize`, `Variation(Id)`, `GlyphSize`, `RenderOptions`, `Constraint`, `FontData`) |
| face/freetype.zig | face/freetype.rs (+ `freetype/ft.rs` FFI, `freetype/resize.rs` = stbir_resize_uint8 equivalent) |
| DeferredFace.zig / discovery.zig | deferred_face.rs / discovery.rs (+ `fontconfig.rs` safe wrapper) |
| Collection.zig / CodepointResolver.zig / CodepointMap.zig | collection.rs / codepoint_resolver.rs / codepoint_map.rs |
| SharedGrid.zig | shared_grid.rs |
| embedded.zig | embedded.rs (`include_bytes!` from `res/`) |
| opentype.zig + opentype/* | opentype/{mod,head,hhea,os2,post,sfnt,svg}.rs |
| nerd_font_attributes.zig | nerd_font_attributes.rs (413 sorted ranges -> 276 constraints, binary search; generated mechanically) |
| shape.zig, shaper/{feature,Cache,noop,harfbuzz}.zig | shape.rs, shaper/{feature,cache,noop,harfbuzz,hb}.rs |
| (config types used by the font code) | config_types.rs (`FreetypeLoadFlags`, `FontSyntheticStyle`, `FontShapingBreak`) |

W7's local `Presentation`/`RenderOptions` stand-ins in `sprite/face.rs` are replaced by
`crate::Presentation` / `crate::face::RenderOptions` (`RenderOptions::new(grid_metrics)`).

## API overview for the renderer
```rust
use ghostty_font::{Library, Face, Collection, CodepointResolver, SharedGrid, Style, Presentation};
use ghostty_font::face::{self, DesiredSize, FontData, RenderOptions};
use ghostty_font::collection::{AddOptions, LoadOptions, Index, SizeAdjustment};

let lib = Library::init()?;                        // Clone; share it
let opts = face::Options::new(DesiredSize { points: 12.0, xdpi: 96, ydpi: 96 });
let mut c = Collection::new();
c.load_options = Some(LoadOptions::new(lib.clone())); // needed for deferred faces + SharedGrid
c.add(Face::init(&lib, FontData::Static(embedded::VARIABLE), opts)?,
      AddOptions { style: Style::Regular, fallback: false, size_adjustment: SizeAdjustment::None })?;
// fonts found by name: Fontconfig::init().discover(&Descriptor{family:Some("..".into()), ..})
//   -> iterator of DeferredFace -> c.add_deferred(df, AddOptions{..})
c.complete_styles(synthetic_style)?;               // synthesize bold/italic if missing
let mut r = CodepointResolver::new(c);             // + r.codepoint_map = ..., r.sprite = ...
let grid = SharedGrid::new(r)?;                    // computes Metrics, sprite face, atlases
```
* Metrics: `grid.metrics()` / `grid.cell_size()` (cell metrics -> terminal grid size).
* Lookup: `grid.get_index(cp, Style, Option<Presentation>) -> Result<Option<Index>>` (cached;
  `Index` is (style, idx), `ghostty_font::SPRITE_INDEX` selects the sprite font).
* Render: `grid.render_glyph(index, glyph_index, RenderOptions{ constraint, constraint_width, cell_width, ..RenderOptions::new(metrics)})
  -> Render{glyph, presentation}` (cached, grows atlas on AtlasFull). `render_codepoint` = lookup+shape-less render.
  `grid.read()/write()` give the `State` guard with `atlas_grayscale`, `atlas_color` (check `atlas.modified/resized`
  atomics to upload to GL), `resolver`, `metrics`.
* Nerd-font icon constraints: the renderer must call `nerd_font_attributes::get_constraint(cp)` and pass it in
  `RenderOptions.constraint` (as in Zig `renderer/generic.zig`; SharedGrid itself only adds the emoji constraint).
* Shaping: `shaper::harfbuzz::Shaper::new(&shape::Options{features})`; per run:
  `prepare(); add_codepoint(cp, cluster)...; finalize(); shape(Some(&face))` (None for the sprite font)
  -> `&[shape::Cell{x, x_offset, y_offset, glyph_index}]`, or `shape_text(face, &[(cp, cluster)])`.
  `shaper::cache::Cache` caches cells per `shape::TextRun`. Shaping needs the face from the collection
  (`collection.get_face(index)` under the grid write lock, as Zig does).
* Face: `get_metrics()` -> `metrics::FaceMetrics` -> `Metrics::calc`; `set_size`, `set_variations`,
  `synthetic_bold/italic`, `glyph_index`, `is_color_glyph`, `render_glyph(&mut Atlas, glyph_index, RenderOptions)`.

## Tests
139 tests pass in the crate (W7's 53 + this work). Zig tests ported: face.zig (Variation x2, Constraints), freetype.zig (9/9),
Collection (11/12; "face metrics" is web_canvas-only upstream), CodepointResolver 3/3, CodepointMap 1/1,
SharedGrid getIndex + render rollback (emulating the tripwire) + error test, discovery/DeferredFace fontconfig tests (CoreText/Wasm skipped),
opentype (head/hhea/os2/post/sfnt/svg), feature.zig, Cache.zig, shaper tests that need no terminal (shape with
embedded fonts: ligatures with `dlig` as in the Zig test shaper, arabic LTR, combining marks, special passthrough, -liga).
Skipped as Zig-only: allocator failure injection, Atlas.Wasm/web_canvas/coretext tests, refAllDecls.

## Deferred / TODO(integration)
* shaper/run.zig (RunIterator/TextRun over terminal rows) and SharedGridSet.zig: not ported (need ghostty-terminal/config). The
  23 terminal-dependent harfbuzz shaper tests are deferred to F2: run iterator, run iterator: empty cells with background set,
  shape, shape inconsolata ligs, shape monaspace ligs, shape arabic forced LTR, shape emoji width, shape emoji width long,
  shape variation selector VS15, VS16, shape with empty cells in between, shape Combining characters, shape Devanagari string,
  shape Tai Tham vowels, shape Tibetan characters, shape Tai Tham letters, shape Javanese ligatures, shape Chakma vowel sign,
  shape Bengali ligatures, shape box glyphs, shape selection boundary, shape cursor boundary, shape cursor boundary and colored emoji,
  shape cell attribute change. `shape::TextRun` is a local version without the `grid` field.
* `config_types.rs` mirrors config's `FreetypeLoadFlags`, `FontSyntheticStyle`, `FontShapingBreak`; `shared_grid::CellSize`
  stands in for `renderer::CellSize`.
* ghostty-config should map `adjust-*` options through `Modifier::parse_cli` (W7 note) and font-codepoint-map into `CodepointMap`.

## Deviations
* `Face` is `Send + Sync` (no unsafe impl); synthetic bold/italic copies share the original's `ft_mutex` (Zig allocated one per Face
  but they shared one FT_Face). Unsupported pixel mode returns `RenderError::UnsupportedPixelMode` instead of panicking; bitmap
  path checks the atlas format before resizing. `set_variations` caps at 32 axes without overrun.
* Collection aliases store the `Index` of the target entry. SharedGrid wraps `RwLock<State>`; `Collection::get_loaded_face(&self)` is
  an added read-lock path.
* freetype-sys lacks FT_Bitmap_*/TT_Header/TT_HoriHeader/FT_LOAD_NO_SVG: declared locally in `ft.rs`.
* stb resize replaced with an in-crate Catmull-Rom/Mitchell separable resampler (not bit-identical to stb).
* `face.rs` carries `#![allow(clippy::manual_midpoint)]` to keep `(a+b)/2` verbatim.
