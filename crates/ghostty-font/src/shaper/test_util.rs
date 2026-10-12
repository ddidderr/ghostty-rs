//! Test helpers shared by the shaper tests (the Zig `testShaper*` helpers
//! plus a tiny terminal emulation on top of `ghostty_terminal::screen::Screen`).
//!
//! TODO(integration): the Zig tests drive a full `Terminal` through a VT
//! stream and read a `RenderState`. `Terminal` is not available in
//! ghostty-terminal yet, so these tests print through `Screen` instead
//! (`Term`), which covers everything the tests need (text, grapheme
//! clusters, SGR, cursor movement).

#![allow(clippy::collapsible_if)]

use ghostty_terminal::{
    page::Cell,
    point::{Coordinate, Point},
    screen::{Options, Screen},
};
use ghostty_vt::{color::Rgb, sgr::Attribute};

use crate::{
    Face,
    Library,
    SharedGrid,
    Style,
    codepoint_resolver::CodepointResolver,
    collection::{AddOptions, Collection, LoadOptions, SizeAdjustment},
    discovery::{Descriptor, Fontconfig},
    embedded,
    face::{DesiredSize, FontData, Options as FaceOptions},
    shape,
    shaper::{
        harfbuzz::Shaper,
        run::{RunCell, RunIterator, RunOptions},
    },
};

#[derive(Clone, Copy)]
pub(crate) enum TestFont {
    Inconsolata,
    MonaspaceNeon,
    Arabic,
}

pub(crate) struct TestShaper {
    pub shaper: Shaper,
    pub grid: SharedGrid,
}

fn face_opts() -> FaceOptions {
    FaceOptions::new(DesiredSize::new(12.0))
}

fn add_regular(c: &mut Collection, face: Face) {
    c.add(
        face,
        AddOptions {
            style: Style::Regular,
            fallback: false,
            size_adjustment: SizeAdjustment::None,
        },
    )
    .unwrap();
}

/// Helper to return a fully initialized shaper.
pub(crate) fn test_shaper() -> TestShaper {
    test_shaper_with_font(TestFont::Inconsolata)
}

pub(crate) fn test_shaper_with_font(font_req: TestFont) -> TestShaper {
    let test_font = match font_req {
        TestFont::Inconsolata => embedded::INCONSOLATA,
        TestFont::MonaspaceNeon => embedded::MONASPACE_NEON,
        TestFont::Arabic => embedded::ARABIC,
    };

    let lib = Library::init().unwrap();
    let mut c = Collection::new();
    c.load_options = Some(LoadOptions::new(lib.clone()));

    add_regular(
        &mut c,
        Face::init(&lib, FontData::Static(test_font), face_opts()).unwrap(),
    );
    add_regular(
        &mut c,
        Face::init(&lib, FontData::Static(embedded::EMOJI), face_opts()).unwrap(),
    );
    add_regular(
        &mut c,
        Face::init(&lib, FontData::Static(embedded::EMOJI_TEXT), face_opts()).unwrap(),
    );

    let grid = SharedGrid::new(CodepointResolver::new(c)).unwrap();
    let shaper = Shaper::new(&shape::Options {
        // Some of our tests rely on dlig being enabled by default
        features: vec!["dlig".into()],
    })
    .unwrap();

    TestShaper { shaper, grid }
}

/// Returns None (the test is skipped, like `error.SkipZigTest`) if the font
/// can't be discovered.
pub(crate) fn test_shaper_with_discovered_font(font_req: &str) -> Option<TestShaper> {
    let lib = Library::init().unwrap();
    let mut c = Collection::new();
    c.load_options = Some(LoadOptions::new(lib.clone()));

    // Discover and add our font to the collection.
    {
        let disco = Fontconfig::init();
        let mut it = disco
            .discover(&Descriptor {
                family: Some(font_req.into()),
                size: 12.0,
                monospace: false,
                ..Descriptor::default()
            })
            .ok()?;
        let mut face = it.next().ok()??;

        // Check which font was discovered - skip if it doesn't match the request
        let name = face.name().unwrap_or_else(|_| "(unknown)".to_string());
        if !name.contains(font_req) {
            return None;
        }

        let loaded = face.load(&lib, face_opts()).ok()?;
        add_regular(&mut c, loaded);
    }

    let grid = SharedGrid::new(CodepointResolver::new(c)).unwrap();
    let shaper = Shaper::new(&shape::Options::default()).unwrap();
    Some(TestShaper { shaper, grid })
}

/// A minimal terminal on top of a [`Screen`].
pub(crate) struct Term {
    pub screen: Screen,
}

impl Term {
    pub fn new(cols: u16, rows: u16) -> Term {
        Term {
            screen: Screen::new(Options::new(cols, rows, 0)).unwrap(),
        }
    }

    /// Print a string with grapheme clustering disabled (the default of
    /// `Terminal`: `grapheme_cluster` mode off). Zero-width codepoints
    /// still combine with the previous cell.
    pub fn print(&mut self, s: &str) {
        self.print_mode(s, false);
    }

    /// Print a string with grapheme clustering enabled
    /// (`t.modes.set(.grapheme_cluster, true)`): codepoints that do not
    /// break the grapheme cluster are attached to the cell of the cluster's
    /// first codepoint.
    pub fn print_clustered(&mut self, s: &str) {
        self.print_mode(s, true);
    }

    fn print_mode(&mut self, s: &str, clustering: bool) {
        let mut state = ghostty_unicode::BreakState::default();
        let mut prev: Option<u32> = None;
        let mut base: Option<ghostty_terminal::page_list::Pin> = None;
        for ch in s.chars() {
            let cp = ch as u32;
            if clustering {
                if let (Some(p), Some(pin)) = (prev, base) {
                    if !ghostty_unicode::grapheme_break(p, cp, &mut state) {
                        let page = self.screen.page_mut(pin.node);
                        page.append_grapheme(usize::from(pin.x), usize::from(pin.y), cp)
                            .unwrap();
                        prev = Some(cp);
                        continue;
                    }
                }
            }
            // A new cluster starts at the cursor (wide chars never wrap in
            // these tests).
            base = Some(self.screen.cursor_pin());
            state = ghostty_unicode::BreakState::default();
            self.screen
                .test_write_string(ch.encode_utf8(&mut [0; 4]))
                .unwrap();
            prev = Some(cp);
        }
    }

    pub fn sgr(&mut self, attr: Attribute<'_>) {
        self.screen.set_attribute(attr).unwrap();
    }

    pub fn fg_rgb(&mut self, r: u8, g: u8, b: u8) {
        self.sgr(Attribute::DirectColorFg(Rgb { r, g, b }));
    }

    pub fn bg_rgb(&mut self, r: u8, g: u8, b: u8) {
        self.sgr(Attribute::DirectColorBg(Rgb { r, g, b }));
    }

    pub fn cursor_right(&mut self, n: u16) {
        self.screen.cursor_right(n);
    }

    /// Mutable access to the cell at (x, y) of the active area.
    pub fn cell_mut(&mut self, x: u16, y: u32) -> &mut Cell {
        self.screen
            .pages
            .get_cell_mut(Point::Active(Coordinate { x, y }))
            .unwrap()
    }

    /// The run cells of row `y` of the active area (`state.row_data.get(y).cells`).
    pub fn row(&self, y: u32) -> Vec<RunCell> {
        let pin = self
            .screen
            .pages
            .pin(Point::Active(Coordinate { x: 0, y }))
            .unwrap();
        RunCell::from_page_row(self.screen.page(pin.node), usize::from(pin.y))
    }
}

/// One iterated run together with its shaped cells and the number of
/// codepoints that were in the HarfBuzz buffer before shaping.
pub(crate) struct Shaped {
    pub run: crate::shape::TextRun,
    pub hb_len: usize,
    pub cells: Vec<shape::Cell>,
}

/// Iterate all runs of `opts`, shaping each one.
pub(crate) fn shape_all(shaper: &mut Shaper, opts: RunOptions<'_>) -> Vec<Shaped> {
    let mut it = RunIterator::new(opts);
    let mut out = Vec::new();
    while let Some(run) = it.next(shaper).unwrap() {
        let hb_len = shaper.hb_buf_len();
        let cells = shaper.shape_run(it.opts.grid, &run).unwrap().to_vec();
        out.push(Shaped { run, hb_len, cells });
    }
    out
}
