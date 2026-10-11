//! The generic renderer (`renderer/generic.zig`), instantiated for the
//! OpenGL graphics API (`renderer.zig`: `pub const Renderer = GenericRenderer(OpenGL)`).
//!
//! The generic renderer builds the GPU cell buffers from a
//! [`ghostty_terminal::render::RenderState`] and draws frames.
//!
//! # Threading model
//!
//! Like Ghostty on GTK (`must_draw_from_app_thread = true`), the work is split
//! between two threads:
//!
//! * The **renderer thread** ([`crate::thread`]) owns the "update" half:
//!   [`Renderer::update_frame`] snapshots the terminal under the renderer
//!   mutex ([`crate::state::SharedState`]), shapes text, renders glyphs into the
//!   font atlases and rebuilds the CPU copies of the cell buffers. It never
//!   issues a GL call.
//! * The **apprt main thread** (the one the `GtkGLArea` context is current on)
//!   owns the "draw" half: [`Renderer::draw_frame`] uploads buffers, atlases,
//!   images and the background image, then issues all GL calls. The renderer
//!   thread merely *requests* a draw through [`Host::redraw_surface`]; the
//!   apprt answers by calling `queue_render` on the GLArea whose `render`
//!   signal handler calls `draw_frame`.
//!
//! The renderer state is split to make that safe without `unsafe`:
//!
//! * `draw` ([`DrawState`], Zig's `draw_mutex`): everything `draw_frame`
//!   reads. Held by the renderer thread only for the short "publish" part of
//!   `update_frame` (`rebuild_cells`, uniforms) and by `set_*`/`change_config`.
//! * `thread` ([`ThreadState`]): data only the renderer thread touches
//!   (`RenderState`, font shaper + cache, overlay, search highlights). It is a
//!   `Mutex` only so that `Renderer` is `Sync`; it is uncontended.
//!
//! Lock order is always: renderer mutex (`SharedState`) is *not* held while
//! taking `draw`; `thread` before `draw` before the font grid lock.
//!
//! # GL context requirements
//!
//! Everything that creates, uses or destroys GL objects ([`Renderer::new`],
//! [`Renderer::draw_frame`], [`Renderer::display_realized`],
//! [`Renderer::display_unrealized`], dropping the `Renderer`) must run on the
//! thread with the GL context current. The renderer thread never does.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use ghostty_config::config::Config;
use ghostty_config::path::Path as ConfigPath;
use ghostty_config::types_a::{BoldColor, FontShapingBreak, TerminalColor, WindowPaddingColor};
use ghostty_config::types_b::{
    AlphaBlending, BackgroundBlur, BackgroundImageFit, BackgroundImagePosition, WindowColorspace,
};
use ghostty_font::face::{Constraint, ConstraintSize, RenderOptions};
use ghostty_font::nerd_font_attributes::get_constraint;
use ghostty_font::shape::{self, Cache as ShaperCache, RunIterator, RunOptions, TextRun};
use ghostty_font::shaper::run::RunCell;
use ghostty_font::{Atlas, Presentation, SPRITE_INDEX, SharedGrid, Sprite, Style as FontStyle};
use ghostty_input::Mods;
use ghostty_terminal::highlight::Flattened;
use ghostty_terminal::page::{self, Cell as PageCell, Wide};
use ghostty_terminal::page_list::Scrollbar;
use ghostty_terminal::point::{Coordinate, Tag};
use ghostty_terminal::render::{Cursor as RenderCursor, Dirty, RenderState, Row as RenderRow};
use ghostty_terminal::style::{self, Style};
use ghostty_terminal::terminal::ScrollViewport;
use ghostty_vt::color::Rgb;
use ghostty_vt::modes::Mode;
use ghostty_vt::sgr::Underline;

use crate::backend::Health;
use crate::cell::{self, Contents, Key as CellKey};
use crate::cursor::{self, Style as CursorStyle};
use crate::image::{self, Gpu};
use crate::link;
use crate::math;
use crate::opengl::buffer::Buffer;
use crate::opengl::gl::{self, Gl, Primitive};
use crate::opengl::render_pass::{Attachment, AttachmentTarget, Draw, Step};
use crate::opengl::shaders::{
    BgImage, BgImageFit, BgImageInfo, BgImagePosition, Bools, CellBg, CellText, CellTextAtlas,
    PaddingExtend, Shaders, Uniforms,
};
use crate::opengl::{self, OpenGL, Sampler, Target, Texture};
use crate::options::Options;
use crate::overlay::{self, Overlay};
use crate::row::never_extend_bg;
use crate::shadertoy::{self, PathItem};
use crate::size::{CellSize, GridSize, Padding, Size};
use crate::state::{Preedit, SharedState};

/// The graphics API used by the renderer on Linux.
pub type GraphicsApi = OpenGL;

/// Errors surfaced by the generic renderer.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Api(#[from] opengl::Error),
    #[error(transparent)]
    Gl(#[from] gl::Error),
    #[error(transparent)]
    Texture(#[from] opengl::texture::Error),
    #[error(transparent)]
    Shaper(#[from] ghostty_font::shaper::harfbuzz::ShapeError),
    #[error(transparent)]
    ShapeRun(#[from] ghostty_font::shaper::harfbuzz::ShapeRunError),
    #[error(transparent)]
    Grid(#[from] ghostty_font::shared_grid::Error),
    #[error(transparent)]
    Link(#[from] link::Error),
    #[error("terminal state error: {0}")]
    Terminal(page::Error),
    #[error("the swap chain is defunct (display unrealized)")]
    Defunct,
}

/// What the renderer needs from the surface/apprt (Zig: `surface_mailbox`
/// and the `redraw_surface` app message). Implementations must not block for
/// long: `scrollbar` and `redraw_surface` are "instant" pushes in Zig.
pub trait Host: Send + Sync {
    /// `surface_mailbox.push(.{ .scrollbar = .. }, .instant)`. Returns true if
    /// the message was accepted (otherwise the renderer retries next frame).
    fn scrollbar(&self, scrollbar: Scrollbar) -> bool;

    /// `surface_mailbox.push(.{ .renderer_health = .. }, .forever)`.
    fn renderer_health(&self, health: Health);

    /// Ask the apprt to call [`Renderer::draw_frame`] from its main thread
    /// (`app_mailbox.push(.{ .redraw_surface = surface }, .instant)`). Called
    /// from the renderer thread because GTK must draw on the main thread
    /// (`must_draw_from_app_thread = true`).
    fn redraw_surface(&self) -> bool;
}

/// A host that discards everything. Useful for tests and headless use.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullHost;

impl Host for NullHost {
    fn scrollbar(&self, _: Scrollbar) -> bool {
        true
    }
    fn renderer_health(&self, _: Health) {}
    fn redraw_surface(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum HighlightTag {
    SearchMatch = 0,
    SearchMatchSelected = 1,
}

/// The configuration for this renderer that is derived from the main
/// configuration (`DerivedConfig`). Owned and cheap to send in messages.
#[derive(Clone)]
pub struct DerivedConfig {
    pub font_thicken: bool,
    pub font_thicken_strength: u8,
    pub font_features: Vec<String>,
    pub font_styles: [bool; 4],
    pub font_shaping_break: FontShapingBreak,
    pub cursor_color: Option<TerminalColor>,
    pub cursor_opacity: f64,
    pub cursor_text: Option<TerminalColor>,
    pub background: Rgb,
    pub background_opacity: f64,
    pub background_opacity_cells: bool,
    pub foreground: Rgb,
    pub selection_background: Option<TerminalColor>,
    pub selection_foreground: Option<TerminalColor>,
    pub search_background: TerminalColor,
    pub search_foreground: TerminalColor,
    pub search_selected_background: TerminalColor,
    pub search_selected_foreground: TerminalColor,
    pub bold_color: Option<BoldColor>,
    pub faint_opacity: u8,
    pub min_contrast: f32,
    pub padding_color: WindowPaddingColor,
    pub custom_shaders: Vec<PathItem>,
    pub bg_image: Option<ConfigPath>,
    pub bg_image_opacity: f32,
    pub bg_image_position: BackgroundImagePosition,
    pub bg_image_fit: BackgroundImageFit,
    pub bg_image_repeat: bool,
    pub links: Arc<link::Set>,
    pub vsync: bool,
    pub colorspace: WindowColorspace,
    pub blending: AlphaBlending,
    pub background_blur: BackgroundBlur,
    pub scroll_to_bottom_on_output: bool,
}

impl DerivedConfig {
    pub fn new(config: &Config) -> Result<DerivedConfig, link::Error> {
        use ghostty_config::types_a::FontStyle as ConfigFontStyle;

        // Copy our shaders
        let custom_shaders = config
            .custom_shader
            .value
            .iter()
            .map(|p| match p {
                ConfigPath::Optional(s) => PathItem::Optional(s.clone()),
                ConfigPath::Required(s) => PathItem::Required(s.clone()),
            })
            .collect();

        // Get our font styles (index order: regular, bold, italic, bold_italic)
        let font_styles = [
            true,
            config.font_style_bold != ConfigFontStyle::False,
            config.font_style_italic != ConfigFontStyle::False,
            config.font_style_bold_italic != ConfigFontStyle::False,
        ];

        // Our link configs
        let links = link::Set::from_config(&config.link.links)?;

        Ok(DerivedConfig {
            background_opacity: config.background_opacity.clamp(0.0, 1.0),
            background_opacity_cells: config.background_opacity_cells,
            font_thicken: config.font_thicken,
            font_thicken_strength: config.font_thicken_strength,
            font_features: config.font_feature.list.clone(),
            font_styles,
            font_shaping_break: config.font_shaping_break,

            cursor_color: config.cursor_color,
            cursor_text: config.cursor_text,
            cursor_opacity: config.cursor_opacity.clamp(0.0, 1.0),

            background: config.background.to_terminal_rgb(),
            foreground: config.foreground.to_terminal_rgb(),
            bold_color: config.bold_color,
            faint_opacity: (config.faint_opacity * 255.0).ceil().clamp(0.0, 255.0) as u8,

            min_contrast: config.minimum_contrast as f32,
            padding_color: config.window_padding_color,

            selection_background: config.selection_background,
            selection_foreground: config.selection_foreground,
            search_background: config.search_background,
            search_foreground: config.search_foreground,
            search_selected_background: config.search_selected_background,
            search_selected_foreground: config.search_selected_foreground,

            custom_shaders,
            bg_image: config.background_image.clone(),
            bg_image_opacity: config.background_image_opacity,
            bg_image_position: config.background_image_position,
            bg_image_fit: config.background_image_fit,
            bg_image_repeat: config.background_image_repeat,
            links: Arc::new(links),
            vsync: config.window_vsync,
            colorspace: config.window_colorspace,
            blending: config.alpha_blending,
            background_blur: config.background_blur,
            scroll_to_bottom_on_output: config.scroll_to_bottom.output,
        })
    }
}

impl std::fmt::Debug for DerivedConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DerivedConfig").finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// GPU frame state

/// State we need duplicated for every frame. Any state that could be in a
/// data race between the GPU and CPU while a frame is being drawn should be
/// in this struct.
///
/// With OpenGL `swap_chain_count == 1` and frames complete synchronously
/// (`glFinish`), so the semaphore of the Zig swap chain is not needed: the
/// draw mutex already serializes `draw_frame`.
struct FrameState {
    uniforms: Buffer<Uniforms>,
    cells: Buffer<CellText>,
    cells_bg: Buffer<CellBg>,

    grayscale: Texture,
    grayscale_modified: usize,
    color: Texture,
    color_modified: usize,

    target: Arc<Target>,
    /// See property of same name on Renderer for explanation.
    target_config_modified: usize,

    /// Buffer with the vertex data for our background image.
    bg_image_buffer: Buffer<BgImage>,
    /// See property of same name on Renderer for explanation.
    bg_image_buffer_modified: usize,

    /// Custom shader state, this is None if we have no custom shaders.
    custom_shader_state: Option<CustomShaderState>,
}

impl FrameState {
    fn new(api: &OpenGL, custom_shaders: bool) -> Result<FrameState, Error> {
        let gl = api.gl().ok_or(opengl::Error::NoContext)?;

        // Uniform buffer contains exactly 1 uniform struct. The
        // uniform data will be zeroed so this must be set before
        // a frame is drawn.
        let uniforms = Buffer::<Uniforms>::init(gl, api.uniform_buffer_options(), 1)?;

        // Create GPU buffers for our cells.
        //
        // We start them off with a size of 1, which will of course be
        // too small, but they will be resized as needed.
        let cells = Buffer::<CellText>::init(gl, api.fg_buffer_options(), 1)?;
        let cells_bg = Buffer::<CellBg>::init(gl, api.bg_buffer_options(), 1)?;

        // Create a GPU buffer for our background image info.
        let bg_image_buffer = Buffer::<BgImage>::init(gl, api.bg_image_buffer_options(), 1)?;

        // Initialize our textures for our font atlas.
        //
        // As with the buffers above, we start these off as small
        // as possible since they'll inevitably be resized anyway.
        //
        // (2x2 rather than Zig's 1x1: `Atlas::new` keeps a 1px border so the
        // smallest valid atlas is 2px; the texture is replaced on first sync.)
        let grayscale =
            api.init_atlas_texture(&Atlas::new(2, ghostty_font::atlas::Format::Grayscale))?;
        let color = api.init_atlas_texture(&Atlas::new(2, ghostty_font::atlas::Format::Bgra))?;

        let custom_shader_state = if custom_shaders {
            Some(CustomShaderState::new(api)?)
        } else {
            None
        };

        // Initialize the target. Just as with the other resources,
        // start it off as small as we can since it'll be resized.
        let target = Arc::new(api.init_target(1, 1)?);

        Ok(FrameState {
            uniforms,
            cells,
            cells_bg,
            grayscale,
            grayscale_modified: 0,
            color,
            color_modified: 0,
            target,
            target_config_modified: 0,
            bg_image_buffer,
            bg_image_buffer_modified: 0,
            custom_shader_state,
        })
    }

    fn resize(&mut self, api: &OpenGL, width: usize, height: usize) -> Result<(), Error> {
        if let Some(state) = &mut self.custom_shader_state {
            state.resize(api, width, height)?;
        }
        self.target = Arc::new(api.init_target(width, height)?);
        Ok(())
    }
}

/// State relevant to our custom shaders if we have any.
struct CustomShaderState {
    /// When we have a custom shader state, we maintain a front
    /// and back texture which we use as a swap chain to render
    /// between when multiple custom shaders are defined.
    front_texture: Texture,
    back_texture: Texture,

    /// Shadertoy uses a sampler for accessing the various channel textures.
    /// Samplers are immutable and describe sampling properties so we can
    /// share the sampler across front/back textures.
    sampler: Sampler,

    uniforms: Buffer<shadertoy::Uniforms>,
}

impl CustomShaderState {
    fn new(api: &OpenGL) -> Result<CustomShaderState, Error> {
        let gl = api.gl().ok_or(opengl::Error::NoContext)?;

        // Create a GPU buffer to hold our uniforms.
        let uniforms = Buffer::<shadertoy::Uniforms>::init(gl, api.uniform_buffer_options(), 1)?;

        // Initialize the front and back textures at 1x1 px, this
        // is slightly wasteful but it's only done once so whatever.
        let front_texture = Texture::init(gl, api.texture_options(), 1, 1, None)?;
        let back_texture = Texture::init(gl, api.texture_options(), 1, 1, None)?;

        let sampler = Sampler::init(gl, api.sampler_options())
            .map_err(|_| gl::Error::CreateFailed("sampler".into()))?;

        Ok(CustomShaderState {
            front_texture,
            back_texture,
            sampler,
            uniforms,
        })
    }

    /// Swap the front and back textures.
    fn swap(&mut self) {
        std::mem::swap(&mut self.front_texture, &mut self.back_texture);
    }

    fn resize(&mut self, api: &OpenGL, width: usize, height: usize) -> Result<(), Error> {
        let gl = api.gl().ok_or(opengl::Error::NoContext)?;
        self.front_texture = Texture::init(gl, api.texture_options(), width, height, None)?;
        self.back_texture = Texture::init(gl, api.texture_options(), width, height, None)?;
        Ok(())
    }
}

/// Swap chain which maintains multiple copies of the state needed to render
/// a frame (always one for OpenGL, see [`FrameState`]).
struct SwapChain {
    frames: Vec<Option<FrameState>>,
    /// Index of the most recently used frame state struct.
    frame_index: usize,
    /// Set to true when deinited, so that a defunct swap chain is ignored
    /// instead of being used or freed twice (`displayUnrealized`).
    defunct: bool,
}

impl SwapChain {
    fn new(api: &OpenGL, custom_shaders: bool) -> Result<SwapChain, Error> {
        let mut frames = Vec::with_capacity(opengl::SWAP_CHAIN_COUNT);
        for _ in 0..opengl::SWAP_CHAIN_COUNT {
            frames.push(Some(FrameState::new(api, custom_shaders)?));
        }
        Ok(SwapChain {
            frames,
            frame_index: 0,
            defunct: false,
        })
    }

    fn deinit(&mut self) {
        if self.defunct {
            return;
        }
        self.defunct = true;
        self.frames.clear();
    }

    /// Get the next frame state to draw to.
    fn next_frame(&mut self) -> Result<(usize, FrameState), Error> {
        if self.defunct || self.frames.is_empty() {
            return Err(Error::Defunct);
        }
        self.frame_index = (self.frame_index + 1) % self.frames.len();
        let frame = self.frames[self.frame_index].take().ok_or(Error::Defunct)?;
        Ok((self.frame_index, frame))
    }

    fn release_frame(&mut self, idx: usize, frame: FrameState) {
        if let Some(slot) = self.frames.get_mut(idx) {
            *slot = Some(frame);
        }
    }

    /// Mutable access to all frames (for invalidating atlas textures).
    fn frames_mut(&mut self) -> impl Iterator<Item = &mut FrameState> {
        self.frames.iter_mut().flatten()
    }
}

/// The background image state (`bg_image: ?imagepkg.Image`).
type BgImageState = Option<image::Image>;

// ---------------------------------------------------------------------------
// Renderer state

/// State protected by the draw mutex: everything `draw_frame` reads.
struct DrawState {
    /// The configuration we need derived from the main config.
    config: DerivedConfig,

    /// Current font metrics defining our grid.
    grid_metrics: ghostty_font::Metrics,

    /// The size of everything.
    size: Size,

    /// True if the window is focused
    focused: bool,

    /// Flag to indicate that our focus state changed for custom
    /// shaders to update their state.
    custom_shader_focused_changed: bool,

    /// The most recent scrollbar state. We use this as a cache to
    /// determine if we need to notify the apprt that there was a
    /// scrollbar change.
    scrollbar: Scrollbar,
    scrollbar_dirty: bool,

    /// The current set of cells to render. This is rebuilt on every frame
    /// but we keep this around so that we don't reallocate. Each set of
    /// cells goes into a separate shader.
    cells: Contents,

    /// Set to true after rebuild_cells is called. This can be used
    /// to determine if any possible changes have been made to the
    /// cells for the draw call.
    cells_rebuilt: bool,

    /// The current GPU uniform values.
    uniforms: Uniforms,

    /// Custom shader uniform values.
    custom_shader_uniforms: shadertoy::Uniforms,

    /// Timestamp we rendered out first frame.
    first_frame_time: Option<Instant>,
    /// Timestamp when we rendered out more recent frame.
    last_frame_time: Option<Instant>,

    /// The font structures.
    font_grid: Arc<SharedGrid>,

    /// The images that we may render.
    images: image::State,

    /// True when a debug overlay image is currently present (Zig: `overlay != null`).
    has_overlay: bool,

    /// Background image, if we have one.
    bg_image: BgImageState,
    /// Background image vertex buffer.
    bg_image_buffer: BgImage,
    /// This value is used to force-update the swap chain copy
    /// of the background image buffer whenever we change it.
    bg_image_buffer_modified: usize,

    /// Graphics API state.
    api: OpenGL,

    /// Our swap chain (multiple buffering)
    swap_chain: SwapChain,

    /// This value is used to force-update swap chain targets in the
    /// event of a config change that requires it (such as blending mode).
    target_config_modified: usize,

    /// If something happened that requires us to reinitialize our shaders,
    /// this is set to true so that we can do that whenever possible.
    reinitialize_shaders: bool,

    /// Whether or not we have custom shaders.
    has_custom_shaders: bool,

    /// Our shader pipelines. None while defunct (display unrealized).
    shaders: Option<Shaders>,
}

/// State only touched by the renderer thread.
struct ThreadState {
    /// The thread's copy of the derived config (kept in sync with
    /// `DrawState::config`).
    config: DerivedConfig,

    /// The render state we update per loop.
    terminal_state: RenderState,

    /// The number of frames since the last terminal state reset.
    /// We reset the terminal state after ~100,000 frames (about 10 to
    /// 15 minutes at 120Hz) to prevent wasted memory buildup from
    /// a large screen.
    terminal_state_frame_count: usize,

    /// Tracks the last bottom-right pin of the screen to detect new output.
    /// Used for scroll-to-bottom on output feature.
    last_bottom: Option<(ghostty_terminal::page_list::NodeId, u16)>,

    /// The most recent viewport matches so that we can render search
    /// matches in the visible frame. This is provided asynchronously
    /// from the search thread so we have the dirty flag to also note
    /// if we need to rebuild our cells to include search highlights.
    ///
    /// Note that the selections MAY BE INVALID (point to PageList nodes
    /// that do not exist anymore). These are validated by the render state.
    search_matches: Option<Vec<Flattened>>,
    search_selected_match: Option<Flattened>,
    search_matches_dirty: bool,

    /// Our overlay state, if any.
    overlay: Option<Overlay>,

    font_shaper: shape::Shaper,
    font_shaper_cache: ShaperCache,

    /// Scratch buffers reused while rebuilding rows.
    run_cells: Vec<RunCell>,
    shaped_cells: Vec<shape::Cell>,
}

/// The generic renderer. See the module docs for the threading model.
pub struct Renderer {
    draw: Mutex<DrawState>,
    thread: Mutex<ThreadState>,

    /// Health of the most recently completed frame.
    health: AtomicU8,

    /// Mirrors `DrawState::has_custom_shaders` so the renderer thread can
    /// read it without the draw lock (`hasAnimations`).
    has_animations: AtomicBool,

    /// The surface/apprt callbacks.
    host: Arc<dyn Host>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How far a [`PreeditRange`] reaches within a row.
struct PreeditRange {
    y: u16,
    x: [u16; 2],
    cp_offset: usize,
}

/// Load the custom shaders (`initShaders`). Errors are logged and result in
/// no custom shaders.
fn load_custom_shaders(config: &DerivedConfig) -> Vec<String> {
    match shadertoy::load_from_files(&config.custom_shaders, opengl::CUSTOM_SHADER_TARGET) {
        Ok(v) => v,
        Err(err) => {
            log::warn!("error loading custom shaders err={err}");
            Vec::new()
        }
    }
}

impl DrawState {
    /// Rebuild the shader pipelines (`initShaders`).
    fn init_shaders(&mut self) -> Result<(), Error> {
        // Load our custom shaders
        let custom_shaders = load_custom_shaders(&self.config);
        let has_custom_shaders = !custom_shaders.is_empty();

        let shaders = self.api.init_shaders(&custom_shaders)?;

        self.shaders = Some(shaders);
        self.has_custom_shaders = has_custom_shaders;
        Ok(())
    }

    /// Update uniforms that are based on the font grid.
    fn update_font_grid_uniforms(&mut self) {
        self.uniforms.cell_size = [
            self.grid_metrics.cell_width as f32,
            self.grid_metrics.cell_height as f32,
        ];
    }

    /// Update uniforms that are based on the screen size.
    fn update_screen_size_uniforms(&mut self) {
        let terminal_size = self.size.terminal();

        // Blank space around the grid.
        let blank: Padding = self.size.screen.blank_padding(
            self.size.padding,
            GridSize {
                columns: self.cells.size.columns,
                rows: self.cells.size.rows,
            },
            CellSize {
                width: self.grid_metrics.cell_width,
                height: self.grid_metrics.cell_height,
            },
        ) + self.size.padding;

        // Setup our uniforms
        self.uniforms.projection_matrix = math::ortho2d(
            -(self.size.padding.left as f32),
            (terminal_size.width + self.size.padding.right) as f32,
            (terminal_size.height + self.size.padding.bottom) as f32,
            -(self.size.padding.top as f32),
        );
        self.uniforms.grid_padding = [
            blank.top as f32,
            blank.right as f32,
            blank.bottom as f32,
            blank.left as f32,
        ];
        self.uniforms.screen_size = [
            self.size.screen.width as f32,
            self.size.screen.height as f32,
        ];
    }

    /// Update the background image vertex buffer (CPU-side).
    ///
    /// This should be called if and when configs change that
    /// could affect the background image.
    fn update_bg_image_buffer(&mut self) {
        self.bg_image_buffer = BgImage {
            opacity: self.config.bg_image_opacity,
            info: BgImageInfo {
                position: match self.config.bg_image_position {
                    BackgroundImagePosition::TopLeft => BgImagePosition::Tl,
                    BackgroundImagePosition::TopCenter => BgImagePosition::Tc,
                    BackgroundImagePosition::TopRight => BgImagePosition::Tr,
                    BackgroundImagePosition::CenterLeft => BgImagePosition::Ml,
                    BackgroundImagePosition::CenterCenter | BackgroundImagePosition::Center => {
                        BgImagePosition::Mc
                    }
                    BackgroundImagePosition::CenterRight => BgImagePosition::Mr,
                    BackgroundImagePosition::BottomLeft => BgImagePosition::Bl,
                    BackgroundImagePosition::BottomCenter => BgImagePosition::Bc,
                    BackgroundImagePosition::BottomRight => BgImagePosition::Br,
                },
                fit: match self.config.bg_image_fit {
                    BackgroundImageFit::Contain => BgImageFit::Contain,
                    BackgroundImageFit::Cover => BgImageFit::Cover,
                    BackgroundImageFit::Stretch => BgImageFit::Stretch,
                    BackgroundImageFit::None => BgImageFit::None,
                },
                repeat: self.config.bg_image_repeat,
            }
            .to_bits(),
            pad: [0; 3],
        };
        // Signal that the buffer was modified.
        self.bg_image_buffer_modified = self.bg_image_buffer_modified.wrapping_add(1);
    }

    /// Call this any time the background image path changes.
    fn prep_background_image(&mut self) {
        // Then we try to load the background image if we have a path.
        if let Some(p) = &self.config.bg_image
            && let Some(image) = load_background_image(p.path())
        {
            // If we have an existing background image, replace it.
            // Otherwise, set this as our background image directly.
            match &mut self.bg_image {
                Some(img) => img.mark_for_replace(image),
                None => self.bg_image = Some(image),
            }
        } else if self.config.bg_image.is_none()
            && let Some(img) = &mut self.bg_image
        {
            // If we don't have a background image path, mark our
            // background image for unload if we currently have one.
            img.mark_for_unload();
        }
    }

    fn upload_background_image(&mut self, gpu: &Gpu<'_>) {
        // Make sure our bg image is uploaded if it needs to be.
        if let Some(bg) = &mut self.bg_image {
            if bg.is_unloading() {
                self.bg_image = None;
                return;
            }
            if bg.is_pending()
                && let Err(err) = bg.upload(gpu)
            {
                log::warn!("error uploading background image err={err}");
            }
        }
    }

    /// Update custom shader uniforms that depend on terminal state.
    ///
    /// This should be called in `update_frame` when terminal state changes.
    fn update_custom_shader_uniforms_from_state(
        &mut self,
        state: &RenderState,
        cursor_text: Option<TerminalColor>,
    ) {
        fn f4(c: Rgb) -> [f32; 4] {
            [
                f32::from(c.r) / 255.0,
                f32::from(c.g) / 255.0,
                f32::from(c.b) / 255.0,
                1.0,
            ]
        }

        // We only need to do this if we have custom shaders.
        if !self.has_custom_shaders {
            return;
        }

        // Only update when terminal state is dirty.
        if state.dirty == Dirty::False {
            return;
        }

        let selection_background = self.config.selection_background;
        let selection_foreground = self.config.selection_foreground;
        let uniforms = &mut self.custom_shader_uniforms;
        let colors = &state.colors;

        // 256-color palette
        for (i, color) in colors.palette.iter().enumerate() {
            uniforms.palette[i] = f4(*color);
        }

        // Background color
        uniforms.background_color = f4(colors.background);

        // Foreground color
        uniforms.foreground_color = f4(colors.foreground);

        // Cursor color
        if let Some(cursor_color) = colors.cursor {
            uniforms.cursor_color = f4(cursor_color);
        }

        // NOTE: the following could be optimized to follow a change in
        // config for a slight optimization however this is only 12 bytes
        // each being updated and likely isn't a cause for concern
        //
        // Deviation: Zig reads `.color` unconditionally, which is invalid for
        // the `cell-foreground`/`cell-background` variants; those are skipped.

        // Cursor text color
        if let Some(TerminalColor::Color(c)) = cursor_text {
            uniforms.cursor_text = f4(c.to_terminal_rgb());
        }

        // Selection background color
        if let Some(TerminalColor::Color(c)) = selection_background {
            uniforms.selection_background_color = f4(c.to_terminal_rgb());
        }

        // Selection foreground color
        if let Some(TerminalColor::Color(c)) = selection_foreground {
            uniforms.selection_foreground_color = f4(c.to_terminal_rgb());
        }

        // Cursor visibility
        uniforms.cursor_visible = i32::from(state.cursor.visible);

        // Cursor style
        let cursor_style = CursorStyle::from_terminal(state.cursor.visual_style);
        uniforms.previous_cursor_style = uniforms.current_cursor_style;
        uniforms.current_cursor_style = cursor_style as i32;
    }

    /// Update per-frame custom shader uniforms.
    ///
    /// This should be called exactly once per frame, inside `draw_frame`.
    #[allow(clippy::float_cmp)]
    fn update_custom_shader_uniforms_for_frame(&mut self) {
        // We only need to do this if we have custom shaders.
        if !self.has_custom_shaders {
            return;
        }

        let now = Instant::now();
        let first_frame_time = *self.first_frame_time.get_or_insert(now);
        let last_frame_time = self.last_frame_time.unwrap_or(now);
        self.last_frame_time = Some(now);

        let screen = self.size.screen;
        let padding = self.size.padding;
        let cell = self.size.cell;
        let cursor_glyph = self.cells.get_cursor_glyph();
        let focused = self.focused;
        let y_is_down = opengl::CUSTOM_SHADER_Y_IS_DOWN;

        let uniforms = &mut self.custom_shader_uniforms;

        let since_ns = now.duration_since(first_frame_time).as_nanos() as f32;
        uniforms.time = since_ns / 1_000_000_000.0;

        let delta_ns = now.duration_since(last_frame_time).as_nanos() as f32;
        uniforms.time_delta = delta_ns / 1_000_000_000.0;

        uniforms.frame += 1;

        uniforms.resolution = [screen.width as f32, screen.height as f32, 1.0];
        uniforms.channel_resolution[0] = [screen.width as f32, screen.height as f32, 1.0, 0.0];

        if let Some(cursor) = cursor_glyph {
            let cursor_width = cursor.glyph_size[0] as f32;
            let cursor_height = cursor.glyph_size[1] as f32;

            // Left edge of the cell the cursor is in.
            let mut pixel_x = (u32::from(cursor.grid_pos[0]) * cell.width + padding.left) as f32;
            // Top edge, relative to the top of the
            // screen, of the cell the cursor is in.
            let mut pixel_y = (u32::from(cursor.grid_pos[1]) * cell.height + padding.top) as f32;

            // If +Y is up in our shaders, we need to flip the coordinate
            // so that it's instead the top edge of the cell relative to
            // the *bottom* of the screen.
            if !y_is_down {
                pixel_y = screen.height as f32 - pixel_y;
            }

            // Add the X bearing to get the -X (left) edge of the cursor.
            pixel_x += f32::from(cursor.bearings[0]);

            // How we deal with the Y bearing depends on which direction
            // is "up", since we want our final `pixel_y` value to be the
            // +Y edge of the cursor.
            if y_is_down {
                // As a reminder, the Y bearing is the distance from the
                // bottom of the cell to the top of the glyph, so to get
                // the +Y edge we need to add the cell height, subtract
                // the Y bearing, and add the glyph height to get the +Y
                // (bottom) edge of the cursor.
                pixel_y += cell.height as f32;
                pixel_y -= f32::from(cursor.bearings[1]);
                pixel_y += cursor.glyph_size[1] as f32;
            } else {
                // If the Y direction is reversed though, we instead want
                // the *top* edge of the cursor, which means we just need
                // to subtract the cell height and add the Y bearing.
                pixel_y -= cell.height as f32;
                pixel_y += f32::from(cursor.bearings[1]);
            }

            let new_cursor = [pixel_x, pixel_y, cursor_width, cursor_height];
            let cursor_color = [
                f32::from(cursor.color[0]) / 255.0,
                f32::from(cursor.color[1]) / 255.0,
                f32::from(cursor.color[2]) / 255.0,
                f32::from(cursor.color[3]) / 255.0,
            ];

            let cursor_changed = new_cursor != uniforms.current_cursor
                || cursor_color != uniforms.current_cursor_color;

            if cursor_changed {
                uniforms.previous_cursor = uniforms.current_cursor;
                uniforms.previous_cursor_color = uniforms.current_cursor_color;
                uniforms.current_cursor = new_cursor;
                uniforms.current_cursor_color = cursor_color;
                uniforms.cursor_change_time = uniforms.time;
            }
        }

        // Update focus uniforms
        uniforms.focus = i32::from(focused);

        // If we need to update the time our focus state changed
        // then update it to our current frame time. This may not be
        // exactly correct since it is frame time, not exact focus
        // time, but focus time on its own isn't exactly correct anyways
        // since it comes async from a message.
        if self.custom_shader_focused_changed && self.focused {
            self.custom_shader_uniforms.time_focus = self.custom_shader_uniforms.time;
            self.custom_shader_focused_changed = false;
        }
    }

    /// Sync the atlas data to the given texture. This copies the bytes
    /// associated with the atlas to the given texture. If the atlas no
    /// longer fits into the texture, the texture will be resized.
    fn sync_atlas_texture(api: &OpenGL, atlas: &Atlas, texture: &mut Texture) -> Result<(), Error> {
        if atlas.size as usize > texture.width {
            // Reallocate (the old texture is dropped/deleted)
            *texture = api.init_atlas_texture(atlas)?;
        }

        texture.replace_region(0, 0, atlas.size as usize, atlas.size as usize, &atlas.data)?;
        Ok(())
    }
}

/// Decode a background image file (`prepBackgroundImage`). Failures are
/// logged and result in no image, as in Zig.
fn load_background_image(path: &str) -> Option<image::Image> {
    // Read it (Max size of 4 GiB, for now.)
    let contents = match std::fs::read(path) {
        Ok(c) if u32::try_from(c.len()).is_ok() => c,
        Ok(_) => {
            log::warn!("background image file \"{path}\" is too large");
            return None;
        }
        Err(err) => {
            log::warn!("error opening background image file \"{path}\": {err}");
            return None;
        }
    };

    // Figure out what type it probably is.
    let file_type = match detect_file_type(&contents) {
        FileType::Unknown => guess_file_type_from_extension(path),
        t => t,
    };

    // Decode it if we know how.
    let (width, height, data) = match file_type {
        FileType::Png => match decode_png(&contents) {
            Ok(v) => v,
            Err(err) => {
                log::warn!("error decoding png background image \"{path}\": {err}");
                return None;
            }
        },
        FileType::Jpeg => match decode_jpeg(&contents) {
            Ok(v) => v,
            Err(err) => {
                log::warn!("error decoding jpeg background image \"{path}\": {err}");
                return None;
            }
        },
        FileType::Unknown => {
            log::warn!("Cannot determine file type for background image file \"{path}\"!");
            return None;
        }
        f => {
            log::warn!("Unsupported file type {f:?} for background image file \"{path}\"!");
            return None;
        }
    };

    Some(image::Image::Pending(image::Pending {
        width,
        height,
        pixel_format: image::PixelFormat::Rgba,
        data,
    }))
}

/// `file_type.zig` (the subset needed here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileType {
    Jpeg,
    Png,
    Gif,
    Bmp,
    Qoi,
    Webp,
    Unknown,
}

fn detect_file_type(contents: &[u8]) -> FileType {
    const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    if contents.starts_with(PNG) {
        FileType::Png
    } else if contents.starts_with(&[0xFF, 0xD8, 0xFF]) {
        FileType::Jpeg
    } else if contents.starts_with(b"GIF87a") || contents.starts_with(b"GIF89a") {
        FileType::Gif
    } else if contents.starts_with(b"BM") {
        FileType::Bmp
    } else if contents.starts_with(b"qoif") {
        FileType::Qoi
    } else if contents.len() >= 12 && &contents[..4] == b"RIFF" && &contents[8..12] == b"WEBP" {
        FileType::Webp
    } else {
        FileType::Unknown
    }
}

fn guess_file_type_from_extension(path: &str) -> FileType {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("jpg" | "jpeg" | "jfif") => FileType::Jpeg,
        Some("png") => FileType::Png,
        Some("gif") => FileType::Gif,
        Some("bmp") => FileType::Bmp,
        Some("qoi") => FileType::Qoi,
        Some("webp") => FileType::Webp,
        _ => FileType::Unknown,
    }
}

/// Decode a PNG to straight (non-premultiplied) RGBA8.
#[allow(clippy::chunks_exact_to_as_chunks)]
fn decode_png(contents: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(contents));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("png too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let n = info.width as usize * info.height as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("unexpanded indexed png".into()),
    };
    if rgba.len() != n * 4 {
        return Err("unexpected png buffer size".into());
    }
    Ok((info.width, info.height, rgba))
}

/// Decode a JPEG to RGBA8.
#[allow(clippy::chunks_exact_to_as_chunks)]
fn decode_jpeg(contents: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(contents));
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("missing jpeg info")?;
    let rgba: Vec<u8> = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        jpeg_decoder::PixelFormat::L16 => pixels
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], 255])
            .collect(),
        jpeg_decoder::PixelFormat::CMYK32 => pixels
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
    };
    Ok((u32::from(info.width), u32::from(info.height), rgba))
}

/// Convert the config bold color to the terminal style's bold policy.
fn bold_policy(c: Option<BoldColor>) -> Option<style::BoldColor> {
    c.map(|c| match c {
        BoldColor::Color(c) => style::BoldColor::Color(c.to_terminal_rgb()),
        BoldColor::Bright => style::BoldColor::Bright,
    })
}

/// Resolve a configured [`TerminalColor`] against the cell colors
/// (the repeated `.color / .cell-foreground / .cell-background` switch).
/// `cell-foreground` is the foreground unless the cell is inverse, in which
/// case it is the background (and vice versa for `cell-background`).
fn resolve_terminal_color(
    c: TerminalColor,
    inverse: bool,
    fg_style: Option<Rgb>,
    bg_style: Option<Rgb>,
) -> Option<Rgb> {
    match c {
        TerminalColor::Color(color) => Some(color.to_terminal_rgb()),
        TerminalColor::CellForeground => {
            if inverse {
                bg_style
            } else {
                fg_style
            }
        }
        TerminalColor::CellBackground => {
            if inverse {
                fg_style
            } else {
                bg_style
            }
        }
    }
}

/// Mutable shaping context for a row rebuild.
struct ShapeCtx<'a> {
    shaper: &'a mut shape::Shaper,
    cache: &'a mut ShaperCache,
    run_cells: &'a mut Vec<RunCell>,
    shaped: &'a mut Vec<shape::Cell>,
}

/// Fill `shaped` with the shaped cells for `run`, from the cache if possible
/// (a copy is made because the shaper's buffer is only valid until the next
/// shaping call).
fn shaped_cells_for_run(
    shaper: &mut shape::Shaper,
    cache: &mut ShaperCache,
    shaped: &mut Vec<shape::Cell>,
    grid: &SharedGrid,
    run: &TextRun,
) -> Result<(), Error> {
    shaped.clear();
    // Try to read the cells from the shaping cache if we can.
    if let Some(cells) = cache.get(run) {
        shaped.extend_from_slice(cells);
        return Ok(());
    }

    // Otherwise we have to shape them.
    let new_cells = shaper.shape_run(grid, run)?;

    // Cache them: this is just a performance optimization.
    cache.put(run, new_cells);
    shaped.extend_from_slice(new_cells);
    Ok(())
}

impl Renderer {
    /// Create the renderer (`Renderer.init`).
    ///
    /// `api` must already have its GL context prepared
    /// ([`OpenGL::prepare_context`], i.e. Zig `Renderer.surfaceInit`), and the
    /// context must be current on the calling thread: swap chain buffers,
    /// textures and shaders are created here.
    pub fn new(opts: Options<DerivedConfig>, api: OpenGL) -> Result<Renderer, Error> {
        let config = opts.config;
        let has_custom_shaders = !config.custom_shaders.is_empty();

        // Prepare our swap chain
        let swap_chain = SwapChain::new(&api, has_custom_shaders)?;

        // Create the font shaper.
        let font_shaper = shape::Shaper::new(&shape::Options {
            features: config.font_features.clone(),
        })?;

        // Initialize all the data that requires a critical font section.
        let grid_metrics = opts.font_grid.metrics();

        let mut draw = DrawState {
            config: config.clone(),
            grid_metrics,
            size: opts.size,
            focused: true,
            custom_shader_focused_changed: false,
            scrollbar: Scrollbar::default(),
            scrollbar_dirty: false,

            // Render state
            cells: Contents::new(),
            cells_rebuilt: false,
            uniforms: Uniforms {
                min_contrast: config.min_contrast,
                cursor_pos: [u16::MAX, u16::MAX],
                bg_color: [
                    config.background.r,
                    config.background.g,
                    config.background.b,
                    // Note that if we're on macOS with glass effects
                    // we'll disable background opacity but we handle
                    // that in update_frame.
                    (config.background_opacity * 255.0).round() as u8,
                ],
                bools: Bools {
                    cursor_wide: false,
                    use_display_p3: config.colorspace == WindowColorspace::DisplayP3,
                    use_linear_blending: config.blending.is_linear(),
                    use_linear_correction: config.blending == AlphaBlending::LinearCorrected,
                }
                .to_bits(),
                ..Uniforms::default()
            },
            custom_shader_uniforms: shadertoy::Uniforms {
                resolution: [0.0, 0.0, 1.0],
                frame_rate: 60.0, // not currently updated
                focus: 1,         // assume focused initially
                ..shadertoy::Uniforms::default()
            },
            first_frame_time: None,
            last_frame_time: None,

            font_grid: opts.font_grid,
            images: image::State::new(),
            has_overlay: false,
            bg_image: None,
            bg_image_buffer: BgImage::default(),
            bg_image_buffer_modified: 0,

            api,
            swap_chain,
            target_config_modified: 0,
            reinitialize_shaders: false,
            has_custom_shaders: false,
            shaders: None,
        };

        draw.init_shaders()?;

        // Ensure our default values above are correctly initialized.
        draw.update_font_grid_uniforms();
        draw.update_screen_size_uniforms();
        draw.update_bg_image_buffer();
        draw.prep_background_image();

        let thread = ThreadState {
            config,
            terminal_state: RenderState::new(),
            terminal_state_frame_count: 0,
            last_bottom: None,
            search_matches: None,
            search_selected_match: None,
            search_matches_dirty: false,
            overlay: None,
            font_shaper,
            font_shaper_cache: ShaperCache::new(),
            run_cells: Vec::new(),
            shaped_cells: Vec::new(),
        };

        let has_animations = AtomicBool::new(draw.has_custom_shaders);
        Ok(Renderer {
            draw: Mutex::new(draw),
            thread: Mutex::new(thread),
            health: AtomicU8::new(Health::Healthy as u8),
            has_animations,
            host: opts.host,
        })
    }

    /// This is called by the GTK apprt after the surface is reinitialized
    /// due to any of the events mentioned in the doc comment of
    /// [`Renderer::display_unrealized`]. The GL context must be current.
    pub fn display_realized<F>(&self, get_proc_address: F) -> Result<(), Error>
    where
        F: FnMut(&str) -> *const std::ffi::c_void,
    {
        // Lock the draw mutex so that we can
        // safely reinitialize our GPU resources.
        let mut d = lock(&self.draw);

        // If our API has to do things on realize, let it.
        d.api.display_realized(get_proc_address);

        // We assume that the swap chain was deinited in
        // `display_unrealized`, in which case it should be
        // marked defunct. If not, we have a problem.
        debug_assert!(d.swap_chain.defunct);

        // We reinitialize our shaders and our swap chain.
        d.init_shaders()?;
        self.has_animations
            .store(d.has_custom_shaders, Ordering::Relaxed);
        d.swap_chain = SwapChain::new(&d.api, d.has_custom_shaders)?;
        d.reinitialize_shaders = false;
        d.target_config_modified = 1;
        d.api.clear_last_target();
        // The new atlas textures and buffers need to be refilled.
        d.update_bg_image_buffer();
        Ok(())
    }

    /// This is called by the GTK apprt when the surface is being destroyed.
    /// This can happen because the surface is being closed but also when
    /// moving the window between displays or splitting. The GL context
    /// must be current.
    pub fn display_unrealized(&self) {
        // Lock the draw mutex so that we can
        // safely deinitialize our GPU resources. (The thread state is locked
        // first, per the lock order.)
        let mut t = lock(&self.thread);
        let mut d = lock(&self.draw);

        // We deinit our swap chain and shaders.
        //
        // This will mark them as defunct so that they
        // can't be double-freed or used in draw calls.
        d.swap_chain.deinit();
        d.shaders = None;
        d.api.clear_last_target();
        // GPU textures of images and the background image die with the
        // context; drop them so they are re-uploaded after realize.
        d.images = image::State::new();
        d.images.force_kitty_update = true;
        d.bg_image = None;
        d.prep_background_image();
        t.terminal_state.dirty = Dirty::Full;
    }

    /// Mark the full screen as dirty so that we redraw everything.
    pub fn mark_dirty(&self) {
        lock(&self.thread).terminal_state.dirty = Dirty::Full;
    }

    /// True if our renderer has animations so that a higher frequency
    /// timer is used.
    pub fn has_animations(&self) -> bool {
        self.has_animations.load(Ordering::Relaxed)
    }

    /// True if our renderer is using vsync. If true, the renderer or apprt
    /// is responsible for triggering draw_now calls to the render thread.
    /// That is the only way to trigger a `draw_frame`. (Always false for
    /// OpenGL on Linux; there is no display link.)
    pub fn has_vsync(&self) -> bool {
        false
    }

    /// The health of the most recently completed frame.
    pub fn health(&self) -> Health {
        if self.health.load(Ordering::SeqCst) == Health::Healthy as u8 {
            Health::Healthy
        } else {
            Health::Unhealthy
        }
    }

    /// The current size of everything as the renderer knows it.
    pub fn size(&self) -> Size {
        lock(&self.draw).size
    }

    /// Callback when the focus changes for the terminal this is rendering.
    ///
    /// Must be called on the render thread.
    pub fn set_focus(&self, focus: bool) {
        let mut d = lock(&self.draw);
        debug_assert_ne!(d.focused, focus);

        d.focused = focus;

        // Flag that we need to update our custom shaders
        d.custom_shader_focused_changed = true;
    }

    /// Callback when the window is visible or occluded.
    ///
    /// Must be called on the render thread. (There is no display link on
    /// Linux, so there is nothing to do.)
    pub fn set_visible(&self, _visible: bool) {}

    /// Set the new font grid.
    ///
    /// Must be called on the render thread.
    pub fn set_font_grid(&self, grid: Arc<SharedGrid>) {
        let mut t = lock(&self.thread);
        let mut d = lock(&self.draw);

        // Get our metrics from the grid. This doesn't require a lock because
        // the metrics are never recalculated.
        let metrics = grid.metrics();

        // Update our grid
        d.font_grid = grid;

        // Update all our textures so that they sync on the next frame.
        for frame in d.swap_chain.frames_mut() {
            frame.grayscale_modified = 0;
            frame.color_modified = 0;
        }

        d.grid_metrics = metrics;

        // Reset our shaper cache. If our font changed (not just the size) then
        // the data in the shaper cache may be invalid and cannot be used, so we
        // always clear the cache just in case.
        t.font_shaper_cache = ShaperCache::new();

        // Update cell size.
        d.size.cell = CellSize {
            width: metrics.cell_width,
            height: metrics.cell_height,
        };

        // Update relevant uniforms
        d.update_font_grid_uniforms();

        // Force a full rebuild, because cached rows may still reference
        // an outdated atlas from the old grid and this can cause garbage
        // to be rendered.
        t.terminal_state.dirty = Dirty::Full;
    }

    /// Update the configuration.
    #[allow(clippy::float_cmp)]
    pub fn change_config(&self, config: DerivedConfig) -> Result<(), Error> {
        let mut t = lock(&self.thread);
        let mut d = lock(&self.draw);

        // We always redo the font shaper in case font features changed. We
        // could check to see if there was an actual config change but this is
        // easier and rare enough to not cause performance issues.
        t.font_shaper = shape::Shaper::new(&shape::Options {
            features: config.font_features.clone(),
        })?;

        // We also need to reset the shaper cache so shaper info
        // from the previous font isn't reused for the new font.
        t.font_shaper_cache = ShaperCache::new();

        // Set our new minimum contrast
        d.uniforms.min_contrast = config.min_contrast;

        // Set our new color space and blending
        let mut bools = Bools::from_bits(d.uniforms.bools);
        bools.use_display_p3 = config.colorspace == WindowColorspace::DisplayP3;
        bools.use_linear_blending = config.blending.is_linear();
        bools.use_linear_correction = config.blending == AlphaBlending::LinearCorrected;
        d.uniforms.bools = bools.to_bits();

        let bg_image_config_changed = d.config.bg_image_fit != config.bg_image_fit
            || d.config.bg_image_position != config.bg_image_position
            || d.config.bg_image_repeat != config.bg_image_repeat
            || d.config.bg_image_opacity != config.bg_image_opacity;

        let bg_image_changed = d.config.bg_image != config.bg_image;

        let old_blending = d.config.blending;
        let custom_shaders_changed = d.config.custom_shaders != config.custom_shaders;

        d.config = config.clone();
        t.config = config;

        // If our background image path changed, prepare the new bg image.
        if bg_image_changed {
            d.prep_background_image();
        }

        // If our background image config changed, update the vertex buffer.
        if bg_image_config_changed {
            d.update_bg_image_buffer();
        }

        // Reset our viewport to force a rebuild, in case of a font change.
        t.terminal_state.dirty = Dirty::Full;

        let blending_changed = old_blending != d.config.blending;

        if blending_changed {
            // We update our API's blending mode.
            d.api.blending = d.config.blending;
            // And indicate that we need to reinitialize our shaders.
            d.reinitialize_shaders = true;
            // And indicate that our swap chain targets need to
            // be re-created to account for the new blending mode.
            d.target_config_modified = d.target_config_modified.wrapping_add(1);
        }

        if custom_shaders_changed {
            d.reinitialize_shaders = true;
        }
        Ok(())
    }

    /// Resize the screen.
    pub fn set_screen_size(&self, size: Size) {
        let mut d = lock(&self.draw);

        // We only actually need the padding from this,
        // everything else is derived elsewhere.
        d.size.padding = size.padding;

        d.update_screen_size_uniforms();

        log::debug!("screen size size={size:?}");
    }

    /// Set the search highlights (from the renderer thread mailbox).
    pub fn set_search_viewport_matches(&self, matches: Vec<Flattened>) {
        let mut t = lock(&self.thread);
        t.search_matches = Some(matches);
        t.search_matches_dirty = true;
    }

    /// Set the selected search match (from the renderer thread mailbox).
    pub fn set_search_selected_match(&self, m: Option<Flattened>) {
        let mut t = lock(&self.thread);
        t.search_selected_match = m;
        t.search_matches_dirty = true;
    }

    /// Read back the pixels (RGBA8, bottom-up like `glReadPixels`) of the
    /// most recently presented frame. Intended for tests and screenshots;
    /// requires the GL context to be current.
    pub fn last_frame_pixels(&self) -> Option<(usize, usize, Vec<u8>)> {
        let d = lock(&self.draw);
        let target = Arc::clone(d.api.last_target()?);
        let gl = d.api.gl()?.clone();
        let bind = gl.bind_framebuffer(target.framebuffer(), gl::FramebufferTarget::Read);
        let px = gl.read_pixels_rgba8(0, 0, target.width as i32, target.height as i32);
        gl.unbind_framebuffer(bind);
        Some((target.width, target.height, px))
    }

    // Callback from the graphics API when a frame is completed.
    fn frame_completed(&self, health: Health) {
        // If our health value hasn't changed, then we do nothing. We don't
        // do a cmpxchg here because strict atomicity isn't important.
        if self.health.load(Ordering::SeqCst) != health as u8 {
            self.health.store(health as u8, Ordering::SeqCst);

            // Our health value changed, so we notify the surface so that it
            // can do something about it.
            self.host.renderer_health(health);
        }
    }
}

// ---------------------------------------------------------------------------
// update_frame (renderer thread)

/// Data extracted from the terminal inside the critical section.
struct Critical {
    links: HashSet<Coordinate>,
    mouse_point: Option<Coordinate>,
    mouse_mods: Mods,
    preedit: Option<Preedit>,
    scrollbar: Scrollbar,
    overlay_features: Vec<overlay::Feature>,
}

/// The max number of frames before we throw away the `RenderState`
/// (~12 minutes at 120Hz).
const MAX_TERMINAL_STATE_FRAME_COUNT: usize = 100_000;

impl Renderer {
    /// Update the frame data.
    ///
    /// This is the renderer thread half of the renderer: it locks
    /// `state` (the renderer mutex) briefly to snapshot the terminal and then
    /// rebuilds the CPU copy of the GPU cell buffers. It performs no GL calls.
    pub fn update_frame(
        &self,
        state: &SharedState,
        cursor_blink_visible: bool,
    ) -> Result<(), Error> {
        let mut guard = lock(&self.thread);
        let t = &mut *guard;

        // We fully deinit and reset the terminal state every so often
        // so that a particularly large terminal state doesn't cause
        // the renderer to hold on to retained memory.
        if t.terminal_state_frame_count >= MAX_TERMINAL_STATE_FRAME_COUNT {
            t.terminal_state = RenderState::new();
        }
        t.terminal_state_frame_count += 1;

        // Update all our data as tightly as possible within the mutex.
        let mut critical = {
            let mut st = lock(state);
            let st = &mut *st;

            // If we're in a synchronized output state, we pause all rendering.
            if st.terminal.modes.get(Mode::SynchronizedOutput) {
                log::debug!("synchronized output started, skipping render");
                return Ok(());
            }

            // If scroll-to-bottom on output is enabled, check if the final line
            // changed by comparing the bottom-right pin. If the node or
            // y offset changed, new content was added to the screen.
            // Update this BEFORE we update our render state so we can
            // draw the new scrolled data immediately.
            if t.config.scroll_to_bottom_on_output
                && let Some(br) = st.terminal.screen().pages.get_bottom_right(Tag::Screen)
                && t.last_bottom != Some((br.node, br.y))
            {
                // Update tracked pin state for next frame
                t.last_bottom = Some((br.node, br.y));

                // Scroll
                st.terminal.scroll_viewport(ScrollViewport::Bottom);
            }

            // Update our terminal state
            t.terminal_state
                .update(&mut st.terminal)
                .map_err(Error::Terminal)?;

            // If our terminal state is dirty at all we need to redo
            // the viewport search.
            if t.terminal_state.dirty != Dirty::False {
                st.terminal.flags.search_viewport_dirty = true;
            }

            // Get our scrollbar out of the terminal. We synchronize
            // the scrollbar read with frame data updates because this
            // naturally limits the number of calls to this method (it
            // can be expensive) and also makes it so we don't need another
            // cross-thread mailbox message within the IO path.
            let scrollbar = st.terminal.screen().pages.scrollbar();

            // Get our preedit state
            let preedit = st.preedit.clone();

            // If we have Kitty graphics data, we enter a SLOW SLOW SLOW path.
            // We only do this if the Kitty image state is dirty meaning only if
            // it changes.
            //
            // If we have any virtual references, we must also rebuild our
            // kitty state on every frame because any cell change can move
            // an image.
            {
                let mut d = lock(&self.draw);
                if d.images.kitty_requires_update(&st.terminal) {
                    let cell_size = CellSize {
                        width: d.grid_metrics.cell_width,
                        height: d.grid_metrics.cell_height,
                    };
                    update_kitty(&mut d.images, &mut st.terminal, cell_size);
                }
            }

            // Get our OSC8 links we're hovering if we have a mouse.
            // This requires terminal state because of URLs.
            let links = 'osc8: {
                // If our mouse isn't hovering, we have no links.
                let Some(vp) = st.mouse.point else {
                    break 'osc8 HashSet::new();
                };

                // If the right mods aren't pressed, then we can't match.
                if !st
                    .mouse
                    .mods
                    .equal(ghostty_input::ctrl_or_super(Mods::default()))
                {
                    break 'osc8 HashSet::new();
                }

                t.terminal_state.link_cells(&st.terminal.screen().pages, vp)
            };

            Critical {
                links,
                mouse_point: st.mouse.point,
                mouse_mods: st.mouse.mods,
                preedit,
                scrollbar,
                overlay_features: st.overlay_features.clone(),
            }
        };

        // Outside the critical area we can update our links to contain
        // our regex results.
        if let Err(err) = t.config.links.render_cell_map(
            &mut critical.links,
            &t.terminal_state,
            critical.mouse_point,
            critical.mouse_mods,
        ) {
            log::warn!("error searching for regex links err={err}");
        }

        // Clear our highlight state and update.
        if t.search_matches_dirty || t.terminal_state.dirty != Dirty::False {
            t.search_matches_dirty = false;

            // Clear the prior highlights
            let mut any_dirty = false;
            for row in &mut t.terminal_state.row_data {
                if !row.highlights.is_empty() {
                    row.highlights.clear();
                    row.dirty = true;
                    any_dirty = true;
                }
            }
            if any_dirty && t.terminal_state.dirty == Dirty::False {
                t.terminal_state.dirty = Dirty::Partial;
            }

            // NOTE: The order below matters. Highlights added earlier
            // will take priority.

            if let Some(m) = &t.search_selected_match {
                t.terminal_state.update_highlights_flattened(
                    HighlightTag::SearchMatchSelected as u8,
                    std::slice::from_ref(m),
                );
            }

            if let Some(m) = &t.search_matches {
                t.terminal_state
                    .update_highlights_flattened(HighlightTag::SearchMatch as u8, m);
            }
        }

        // Rebuild the overlay image if we have one. We can do this
        // outside of any critical areas.
        if let Err(err) = self.rebuild_overlay(t, &critical.overlay_features) {
            log::warn!("error rebuilding overlay surface err={err}");
        }

        // Acquire the draw mutex for all remaining state updates.
        {
            let mut d = lock(&self.draw);

            // Build our GPU cells
            let cursor_style = cursor::style(
                &t.terminal_state,
                cursor::StyleOptions {
                    preedit: critical.preedit.is_some(),
                    focused: d.focused,
                    blink_visible: cursor_blink_visible,
                },
            );
            d.rebuild_cells(t, critical.preedit.as_ref(), cursor_style, &critical.links);

            // The scrollbar is only emitted during draws so we also
            // check the scrollbar cache here and update if needed.
            // This is pretty fast.
            if d.scrollbar != critical.scrollbar {
                d.scrollbar = critical.scrollbar;
                d.scrollbar_dirty = true;
            }

            // Update our background color
            d.uniforms.bg_color = [
                t.terminal_state.colors.background.r,
                t.terminal_state.colors.background.g,
                t.terminal_state.colors.background.b,
                (t.config.background_opacity * 255.0).round() as u8,
            ];

            // (On macOS glass styles the background opacity is removed; that
            // is out of scope on Linux.)

            // Prepare our overlay image for upload (or unload).
            d.has_overlay = t.overlay.is_some();
            if let Err(err) = d.images.overlay_update(t.overlay.as_ref()) {
                log::warn!("error updating overlay images err={err}");
            }

            // Update custom shader uniforms that depend on terminal state.
            let cursor_text = t.config.cursor_text;
            d.update_custom_shader_uniforms_from_state(&t.terminal_state, cursor_text);
        }

        // Reset our dirty state after updating.
        t.terminal_state.dirty = Dirty::False;

        // Notify our shaper we're done for the frame. For some shapers,
        // such as CoreText, this triggers off-thread cleanup logic.
        t.font_shaper.end_frame();
        Ok(())
    }

    /// Build the overlay as configured (`rebuildOverlay`).
    fn rebuild_overlay(
        &self,
        t: &mut ThreadState,
        features: &[overlay::Feature],
    ) -> Result<(), overlay::InitError> {
        // If we have no features enabled, don't build an overlay.
        // If we had a previous overlay, deallocate it.
        if features.is_empty() {
            t.overlay = None;
            return Ok(());
        }

        let size = lock(&self.draw).size;

        // If we had a previous overlay, clear it. Otherwise, init.
        let reuse = match &t.overlay {
            Some(v) => {
                // Verify that our overlay size matches our screen
                // size as we know it now. If not, deinit and reinit.
                let term_size = size.terminal();
                // We also depend on cell size.
                v.width() == term_size.width
                    && v.height() == term_size.height
                    && v.cell_size == size.cell
            }
            None => false,
        };

        if reuse {
            // Everything matches, so we can just reset the surface
            // and redraw.
            if let Some(v) = &mut t.overlay {
                v.reset();
            }
        } else {
            // If we reached this point we want to reset our overlay.
            t.overlay = None;
            t.overlay = Some(Overlay::new(&size)?);
        }

        if let Some(v) = &mut t.overlay {
            v.apply_features(&t.terminal_state, features);
        }
        Ok(())
    }
}

/// Update the kitty image state from the terminal (the `kittyUpdate` call in
/// `updateFrame`).
fn update_kitty(
    images: &mut image::State,
    terminal: &mut ghostty_terminal::terminal::Terminal,
    cell_size: CellSize,
) {
    let info = ghostty_terminal::kitty::graphics_storage::TerminalInfo::from_terminal(terminal);
    let screen = terminal.screen_mut();
    images.kitty_update(&screen.kitty_images, &screen.pages, &info, cell_size);
    images.force_kitty_update = false;
    screen.kitty_images.dirty = false;
}

// ---------------------------------------------------------------------------
// rebuild_cells (draw lock held, renderer thread)

/// Read-only data shared by every row rebuild.
struct RowEnv<'a> {
    config: &'a DerivedConfig,
    colors: &'a ghostty_terminal::render::Colors,
    cursor: &'a RenderCursor,
    cols: u16,
}

/// Which highlight (if any) covers a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
enum Selected {
    False,
    Selection,
    Search,
    SearchSelected,
}

impl DrawState {
    /// Convert the terminal state to GPU cells stored in CPU memory. These
    /// are then synced to the GPU in the next frame. This only updates CPU
    /// memory and doesn't touch the GPU.
    ///
    /// This requires the draw mutex.
    ///
    /// Dirty state on terminal state won't be reset by this.
    fn rebuild_cells(
        &mut self,
        t: &mut ThreadState,
        preedit: Option<&Preedit>,
        cursor_style_: Option<CursorStyle>,
        links: &HashSet<Coordinate>,
    ) {
        let ThreadState {
            config,
            terminal_state: state,
            font_shaper,
            font_shaper_cache,
            run_cells,
            shaped_cells,
            ..
        } = t;

        let grid_size_diff =
            self.cells.size.rows != state.rows || self.cells.size.columns != state.cols;

        if grid_size_diff {
            let new_size = GridSize {
                rows: state.rows,
                columns: state.cols,
            };
            self.cells.resize(new_size);

            // Update our uniforms accordingly, otherwise
            // our background cells will be out of place.
            self.uniforms.grid_size = [new_size.columns, new_size.rows];
        }

        let rebuild = state.dirty == Dirty::Full || grid_size_diff;
        if rebuild {
            // If we are doing a full rebuild, then we clear the entire cell buffer.
            self.cells.reset();

            // We also reset our padding extension depending on the screen type
            match config.padding_color {
                WindowPaddingColor::Background => {}

                // For extension, assume we are extending in all directions.
                // For "extend" this may be disabled due to heuristics below.
                WindowPaddingColor::Extend | WindowPaddingColor::ExtendAlways => {
                    self.uniforms.padding_extend = PaddingExtend {
                        up: true,
                        down: true,
                        left: true,
                        right: true,
                    }
                    .to_bits();
                }
            }
        }

        let RenderState {
            rows,
            cols,
            colors,
            cursor,
            row_data,
            ..
        } = &mut *state;
        let (rows, cols) = (*rows, *cols);

        // If our cell contents buffer is shorter than the screen viewport,
        // we render the rows that fit, starting from the bottom. If instead
        // the viewport is shorter than the cell contents buffer, we align
        // the top of the viewport with the top of the contents buffer.
        let row_len = usize::from(rows.min(self.cells.size.rows));

        // Determine our x/y range for preedit. We don't want to render anything
        // here because we will render the preedit separately.
        let preedit_range: Option<PreeditRange> = preedit.and_then(|preedit_v| {
            // We base the preedit on the position of the cursor in the
            // viewport. If the cursor isn't visible in the viewport we
            // don't show it.
            let cursor_vp = cursor.viewport?;

            // If our preedit row isn't dirty then we don't need the
            // preedit range. This also avoids an issue later where we
            // unconditionally add preedit cells when this is set.
            if !rebuild && !row_data[usize::from(cursor_vp.y)].dirty {
                return None;
            }

            let range = preedit_v.range(cursor_vp.x, cols - 1);
            Some(PreeditRange {
                y: cursor_vp.y,
                x: [range.start, range.end],
                cp_offset: range.cp_offset,
            })
        });

        let env = RowEnv {
            config,
            colors,
            cursor,
            cols,
        };
        let mut sh = ShapeCtx {
            shaper: font_shaper,
            cache: font_shaper_cache,
            run_cells,
            shaped: shaped_cells,
        };

        for (y_usize, row) in row_data.iter_mut().enumerate().take(row_len) {
            let y = y_usize as u16;

            if !rebuild {
                // Only rebuild if we are doing a full rebuild or this row is dirty.
                if !row.dirty {
                    continue;
                }

                // Clear the cells if the row is dirty
                self.cells.clear(y);
            }

            // Unmark the dirty state in our render state.
            row.dirty = false;

            if let Err(err) = self.rebuild_row(&env, y, row, preedit_range.as_ref(), links, &mut sh)
            {
                // This should never happen except under exceptional
                // scenarios. In this case, we don't want to corrupt
                // our render state so just clear this row and keep
                // trying to finish it out.
                log::warn!("error building row y={y} err={err}");
                self.cells.clear(y);
            }
        }

        // Setup our cursor rendering information.
        self.setup_cursor(&env, row_data, cursor_style_, preedit.is_some());

        // Setup our preedit text.
        if let Some(preedit_v) = preedit
            && let Some(range) = &preedit_range
        {
            let mut x = range.x[0];
            for cp in &preedit_v.codepoints[range.cp_offset..] {
                if let Err(err) = self.add_preedit_cell(
                    config,
                    cp.codepoint,
                    cp.wide,
                    Coordinate {
                        x,
                        y: u32::from(range.y),
                    },
                    colors.foreground,
                ) {
                    log::warn!(
                        "error building preedit cell, will be invalid x={x} y={}, err={err}",
                        range.y
                    );
                }

                x += if cp.wide { 2 } else { 1 };
            }
        }

        // Update that our cells rebuilt
        self.cells_rebuilt = true;
    }

    fn setup_cursor(
        &mut self,
        env: &RowEnv<'_>,
        row_data: &[RenderRow],
        cursor_style_: Option<CursorStyle>,
        has_preedit: bool,
    ) {
        let config = env.config;
        let colors = env.colors;

        // Clear our cursor by default.
        self.cells.set_cursor(None, None);
        self.uniforms.cursor_pos = [u16::MAX, u16::MAX];

        // If the cursor isn't visible on the viewport, don't show
        // a cursor. Otherwise, get our cursor cell, because we may
        // need it for styling.
        let Some(cursor_vp) = env.cursor.viewport else {
            return;
        };
        let cursor_style: Style = {
            let cell = row_data[usize::from(cursor_vp.y)]
                .cells
                .get(usize::from(cursor_vp.x));
            if cell.raw.has_styling() {
                cell.style
            } else {
                Style::default()
            }
        };

        // If we have preedit text, we don't setup a cursor
        if has_preedit {
            return;
        }

        // If there isn't a cursor visual style requested then
        // we don't render a cursor.
        let Some(style) = cursor_style_ else {
            return;
        };

        let bold = bold_policy(config.bold_color);
        let fg_style = cursor_style.fg(style::Fg {
            default: colors.foreground,
            palette: &colors.palette,
            bold,
        });
        let bg_style = cursor_style
            .bg(&env.cursor.cell, &colors.palette)
            .unwrap_or(colors.background);

        // Determine the cursor color.
        let cursor_color = 'color: {
            // If an explicit cursor color was set by OSC 12, use that.
            if let Some(v) = colors.cursor {
                break 'color v;
            }

            // Use our configured color if specified
            if let Some(v) = config.cursor_color {
                break 'color resolve_terminal_color(
                    v,
                    cursor_style.flags.inverse,
                    Some(fg_style),
                    Some(bg_style),
                )
                .unwrap_or(colors.foreground);
            }

            colors.foreground
        };

        self.add_cursor(env, style, cursor_color);

        // If the cursor is visible then we set our uniforms.
        if style == CursorStyle::Block {
            let wide = env.cursor.cell.wide();

            self.uniforms.cursor_pos = [
                // If we are a spacer tail of a wide cell, our cursor needs
                // to move back one cell. The saturate is to ensure we don't
                // overflow but this shouldn't happen with well-formed input.
                match wide {
                    Wide::Narrow | Wide::SpacerHead | Wide::Wide => cursor_vp.x,
                    Wide::SpacerTail => cursor_vp.x.saturating_sub(1),
                },
                cursor_vp.y,
            ];

            let mut bools = Bools::from_bits(self.uniforms.bools);
            bools.cursor_wide = match wide {
                Wide::Narrow | Wide::SpacerHead => false,
                Wide::Wide | Wide::SpacerTail => true,
            };
            self.uniforms.bools = bools.to_bits();

            let uniform_color = if let Some(txt) = config.cursor_text {
                // If cursor-text is set, then compute the correct color.
                // Otherwise, use the background color.
                resolve_terminal_color(
                    txt,
                    cursor_style.flags.inverse,
                    Some(fg_style),
                    Some(bg_style),
                )
                .unwrap_or(colors.background)
            } else {
                colors.background
            };

            self.uniforms.cursor_color = [uniform_color.r, uniform_color.g, uniform_color.b, 255];
        }
    }
}

impl DrawState {
    #[allow(clippy::too_many_arguments)]
    fn rebuild_row(
        &mut self,
        env: &RowEnv<'_>,
        y: u16,
        row: &RenderRow,
        preedit_range: Option<&PreeditRange>,
        links: &HashSet<Coordinate>,
        sh: &mut ShapeCtx<'_>,
    ) -> Result<(), Error> {
        let config = env.config;
        let colors = env.colors;
        let cursor = env.cursor;

        // If our viewport is wider than our cell contents buffer,
        // we still only process cells up to the width of the buffer.
        let cells = &row.cells;
        let cells_len = cells.len().min(usize::from(self.cells.size.columns));
        let cells_raw = &cells.raw;

        // On primary screen, we still apply vertical padding
        // extension under certain conditions we feel are safe.
        //
        // This helps make some scenarios look better while
        // avoiding scenarios we know do NOT look good.
        match config.padding_color {
            // These already have the correct values set above.
            WindowPaddingColor::Background | WindowPaddingColor::ExtendAlways => {}

            // Apply heuristics for padding extension.
            WindowPaddingColor::Extend => {
                if y == 0 {
                    let mut pe = PaddingExtend::from_bits(self.uniforms.padding_extend);
                    pe.up = !never_extend_bg(
                        row.raw,
                        cells_raw,
                        &cells.style,
                        &colors.palette,
                        colors.background,
                    );
                    self.uniforms.padding_extend = pe.to_bits();
                } else if y == self.cells.size.rows - 1 {
                    let mut pe = PaddingExtend::from_bits(self.uniforms.padding_extend);
                    pe.down = !never_extend_bg(
                        row.raw,
                        cells_raw,
                        &cells.style,
                        &colors.palette,
                        colors.background,
                    );
                    self.uniforms.padding_extend = pe.to_bits();
                }
            }
        }

        // The run iterator wants an owned-cell slice (`RunCell`).
        sh.run_cells.clear();
        sh.run_cells.extend(cells.iter().map(|c| RunCell {
            raw: c.raw,
            grapheme: c.grapheme.to_vec(),
            style: c.style,
        }));

        // Iterator of runs for shaping.
        let font_grid = Arc::clone(&self.font_grid);
        let mut run_iter_opts = RunOptions::new(&font_grid, sh.run_cells.as_slice());
        run_iter_opts.selection = row.selection;
        // We want to do font shaping as long as the cursor is
        // visible on this viewport.
        run_iter_opts.cursor_x = cursor
            .viewport
            .filter(|vp| vp.y == y)
            .map(|vp| usize::from(vp.x));
        run_iter_opts.apply_break_config(config.font_shaping_break);
        let mut run_iter = RunIterator::new(run_iter_opts);
        let mut shaper_run: Option<TextRun> = run_iter.next(&mut *sh.shaper)?;
        // `sh.shaped` holds the shaped cells of `shaper_run` when `have_cells`.
        let mut have_cells = false;
        let mut shaper_cells_i: usize = 0;

        for x in 0..cells_len {
            // If this cell falls within our preedit range then we
            // skip this because preedits are setup separately.
            if let Some(range) = preedit_range
                && range.y == y
                && x >= usize::from(range.x[0])
            {
                // We're in the preedit range, skip this cell.
                if x <= usize::from(range.x[1]) {
                    continue;
                }

                // After exiting the preedit range we need to catch
                // the run position up because of the missed cells.
                // In all other cases, no action is necessary.
                if x == usize::from(range.x[1]) + 1 {
                    // Step the run iterator until we find a run that ends
                    // after the current cell, which will be the soonest run
                    // that might contain glyphs for our cell.
                    while let Some(run) = shaper_run {
                        if usize::from(run.offset) + usize::from(run.cells) > x {
                            break;
                        }
                        shaper_run = run_iter.next(&mut *sh.shaper)?;
                        have_cells = false;
                        shaper_cells_i = 0;
                    }

                    if let Some(run) = shaper_run {
                        // If we haven't shaped this run, do so now.
                        if !have_cells {
                            shaped_cells_for_run(sh.shaper, sh.cache, sh.shaped, &font_grid, &run)?;
                            have_cells = true;
                        }

                        // Advance our index until we reach or pass
                        // our current x position in the shaper cells.
                        while shaper_cells_i < sh.shaped.len()
                            && usize::from(run.offset) + usize::from(sh.shaped[shaper_cells_i].x)
                                < x
                        {
                            shaper_cells_i += 1;
                        }
                    }
                }
            }

            let cell = cells_raw[x];
            let wide = cell.wide();
            let cell_style: Style = if cell.has_styling() {
                cells.style(x)
            } else {
                Style::default()
            };

            // True if this cell is selected
            let selected = 'selected: {
                // Order below matters for precedence.

                // Selection should take the highest precedence.
                let x_compare = if wide == Wide::SpacerTail {
                    x.saturating_sub(1)
                } else {
                    x
                };
                if let Some(sel) = row.selection
                    && x_compare >= usize::from(sel[0])
                    && x_compare <= usize::from(sel[1])
                {
                    break 'selected Selected::Selection;
                }

                // If we're highlighted, then we're selected. In the
                // future we want to use a different style for this
                // but this to get started.
                for hl in &row.highlights {
                    if x_compare >= usize::from(hl.range[0])
                        && x_compare <= usize::from(hl.range[1])
                    {
                        break 'selected if hl.tag == HighlightTag::SearchMatchSelected as u8 {
                            Selected::SearchSelected
                        } else {
                            Selected::Search
                        };
                    }
                }

                Selected::False
            };

            // The `_style` suffixed values are the colors based on
            // the cell style (SGR), before applying any additional
            // configuration, inversions, selections, etc.
            let bg_style = cell_style.bg(&cell, &colors.palette);
            let fg_style = cell_style.fg(style::Fg {
                default: colors.foreground,
                palette: &colors.palette,
                bold: bold_policy(config.bold_color),
            });
            let inverse = cell_style.flags.inverse;

            // The final background color for the cell.
            let bg: Option<Rgb> = match selected {
                // If we have an explicit selection background color
                // specified in the config, use that.
                //
                // If no configuration, then our selection background
                // is our foreground color.
                Selected::Selection => match config.selection_background {
                    Some(v) => resolve_terminal_color(v, inverse, Some(fg_style), bg_style),
                    None => Some(colors.foreground),
                },

                Selected::Search => resolve_terminal_color(
                    config.search_background,
                    inverse,
                    Some(fg_style),
                    bg_style,
                ),

                Selected::SearchSelected => resolve_terminal_color(
                    config.search_selected_background,
                    inverse,
                    Some(fg_style),
                    bg_style,
                ),

                // Not selected
                Selected::False => {
                    if inverse == cell::is_covering(cell.codepoint()) {
                        // Otherwise they cancel out.
                        bg_style
                    } else {
                        // Two cases cause us to invert (use the fg color as the bg)
                        // - The "inverse" style flag.
                        // - A "covering" glyph; we use fg for bg in that
                        //   case to help make sure that padding extension
                        //   works correctly.
                        //
                        // If one of these is true (but not the other)
                        // then we use the fg style color for the bg.
                        Some(fg_style)
                    }
                }
            };

            let fg: Rgb = {
                // Our happy-path non-selection background color
                // is our style or our configured defaults.
                let final_bg = bg_style.unwrap_or(colors.background);

                // Whether we need to use the bg color as our fg color:
                // - Cell is selected, inverted, and set to cell-foreground
                // - Cell is selected, not inverted, and set to cell-background
                // - Cell is inverted and not selected
                match selected {
                    Selected::Selection => match config.selection_foreground {
                        Some(v) => {
                            resolve_terminal_color(v, inverse, Some(fg_style), Some(final_bg))
                        }
                        None => Some(colors.background),
                    },

                    Selected::Search => resolve_terminal_color(
                        config.search_foreground,
                        inverse,
                        Some(fg_style),
                        Some(final_bg),
                    ),

                    Selected::SearchSelected => resolve_terminal_color(
                        config.search_selected_foreground,
                        inverse,
                        Some(fg_style),
                        Some(final_bg),
                    ),

                    Selected::False => Some(if inverse { final_bg } else { fg_style }),
                }
                .unwrap_or(fg_style)
            };

            // Foreground alpha for this cell.
            let alpha: u8 = if cell_style.flags.faint {
                config.faint_opacity
            } else {
                255
            };

            // Set the cell's background color.
            {
                let rgb = bg.unwrap_or(colors.background);

                // Determine our background alpha. If we have transparency configured
                // then this is dynamic depending on some situations. This is all
                // in an attempt to make transparency look the best for various
                // situations. See inline comments.
                let bg_alpha: u8 = 'bg_alpha: {
                    let default: u8 = 255;

                    // Cells that are selected should be fully opaque.
                    if selected != Selected::False {
                        break 'bg_alpha default;
                    }

                    // Cells that are reversed should be fully opaque.
                    if inverse {
                        break 'bg_alpha default;
                    }

                    // If the user requested to have opacity on all cells, apply it.
                    if config.background_opacity_cells && bg_style.is_some() {
                        let mut opacity = f64::from(default);
                        opacity *= config.background_opacity;
                        break 'bg_alpha opacity as u8;
                    }

                    // Cells that have an explicit bg color should be fully opaque.
                    if bg_style.is_some() {
                        break 'bg_alpha default;
                    }

                    // Otherwise, we won't draw the bg for this cell,
                    // we'll let the already-drawn background color
                    // show through.
                    0
                };

                *self.cells.bg_cell(usize::from(y), x) = [rgb.r, rgb.g, rgb.b, bg_alpha];
            }

            // If the invisible flag is set on this cell then we
            // don't need to render any foreground elements, so
            // we just skip all glyphs with this x coordinate.
            //
            // NOTE: This behavior matches xterm. Some other terminal
            // emulators, e.g. Alacritty, still render text decorations
            // and only make the text itself invisible. The decision
            // has been made here to match xterm's behavior for this.
            if cell_style.flags.invisible {
                continue;
            }

            // Give links a single underline, unless they already have
            // an underline, in which case use a double underline to
            // distinguish them.
            let underline = if links.contains(&Coordinate {
                x: x as u16,
                y: u32::from(y),
            }) {
                if cell_style.flags.underline == Underline::Single {
                    Underline::Double
                } else {
                    Underline::Single
                }
            } else {
                cell_style.flags.underline
            };

            // We draw underlines first so that they layer underneath text.
            // This improves readability when a colored underline is used
            // which intersects parts of the text (descenders).
            if underline != Underline::None
                && let Err(err) = self.add_underline(
                    x as u16,
                    y,
                    underline,
                    cell_style.underline_color(&colors.palette).unwrap_or(fg),
                    alpha,
                )
            {
                log::warn!(
                    "error adding underline to cell, will be invalid x={x} y={y}, err={err}"
                );
            }

            if cell_style.flags.overline
                && let Err(err) = self.add_overline(x as u16, y, fg, alpha)
            {
                log::warn!("error adding overline to cell, will be invalid x={x} y={y}, err={err}");
            }

            // If we're at or past the end of our shaper run then
            // we need to get the next run from the run iterator.
            if have_cells && shaper_cells_i >= sh.shaped.len() {
                shaper_run = run_iter.next(&mut *sh.shaper)?;
                have_cells = false;
                shaper_cells_i = 0;
            }

            if let Some(run) = shaper_run {
                // If we haven't shaped this run yet, do so.
                if !have_cells {
                    shaped_cells_for_run(sh.shaper, sh.cache, sh.shaped, &font_grid, &run)?;
                    have_cells = true;
                }

                // If there are no shaper cells for this run, ignore it.
                // This can occur for runs of empty cells, and is fine.
                if !sh.shaped.is_empty() && shaper_cells_i < sh.shaped.len() {
                    // If we encounter a shaper cell to the left of the current
                    // cell then we have some problems. This logic relies on x
                    // position monotonically increasing.
                    debug_assert!(
                        usize::from(run.offset) + usize::from(sh.shaped[shaper_cells_i].x) >= x
                    );

                    // NOTE: An assumption is made here that a single cell will never
                    // be present in more than one shaper run. If that assumption is
                    // violated, this logic breaks.

                    while shaper_cells_i < sh.shaped.len()
                        && usize::from(run.offset) + usize::from(sh.shaped[shaper_cells_i].x) == x
                    {
                        let shaped_cell = sh.shaped[shaper_cells_i];
                        if let Err(err) = self.add_glyph(
                            config,
                            x as u16,
                            y,
                            usize::from(env.cols),
                            cells_raw,
                            shaped_cell,
                            &run,
                            fg,
                            alpha,
                        ) {
                            log::warn!(
                                "error adding glyph to cell, will be invalid x={x} y={y}, err={err}"
                            );
                        }
                        shaper_cells_i += 1;
                    }
                }
            }

            // Finally, draw a strikethrough if necessary.
            if cell_style.flags.strikethrough
                && let Err(err) = self.add_strikethrough(x as u16, y, fg, alpha)
            {
                log::warn!(
                    "error adding strikethrough to cell, will be invalid x={x} y={y}, err={err}"
                );
            }
        }

        Ok(())
    }

    /// Render options for a sprite glyph that spans a single cell.
    fn sprite_options(&self, cell_width: u8) -> RenderOptions {
        RenderOptions {
            cell_width: Some(cell_width),
            ..RenderOptions::new(self.grid_metrics)
        }
    }

    /// Add a sprite decoration (underline, overline, strikethrough) to the
    /// specified cell.
    fn add_sprite_decoration(
        &mut self,
        key: CellKey,
        sprite: Sprite,
        x: u16,
        y: u16,
        color: Rgb,
        alpha: u8,
    ) -> Result<(), Error> {
        let render =
            self.font_grid
                .render_glyph(SPRITE_INDEX, sprite as u32, self.sprite_options(1))?;

        self.cells.add(
            key,
            CellText {
                atlas: CellTextAtlas::Grayscale as u8,
                grid_pos: [x, y],
                color: [color.r, color.g, color.b, alpha],
                glyph_pos: [render.glyph.atlas_x, render.glyph.atlas_y],
                glyph_size: [render.glyph.width, render.glyph.height],
                bearings: [render.glyph.offset_x as i16, render.glyph.offset_y as i16],
                ..CellText::default()
            },
        );
        Ok(())
    }

    /// Add an underline decoration to the specified cell
    fn add_underline(
        &mut self,
        x: u16,
        y: u16,
        style: Underline,
        color: Rgb,
        alpha: u8,
    ) -> Result<(), Error> {
        let sprite = match style {
            Underline::None => unreachable!(),
            Underline::Single => Sprite::Underline,
            Underline::Double => Sprite::UnderlineDouble,
            Underline::Dotted => Sprite::UnderlineDotted,
            Underline::Dashed => Sprite::UnderlineDashed,
            Underline::Curly => Sprite::UnderlineCurly,
        };
        self.add_sprite_decoration(CellKey::Underline, sprite, x, y, color, alpha)
    }

    /// Add a overline decoration to the specified cell
    fn add_overline(&mut self, x: u16, y: u16, color: Rgb, alpha: u8) -> Result<(), Error> {
        self.add_sprite_decoration(CellKey::Overline, Sprite::Overline, x, y, color, alpha)
    }

    /// Add a strikethrough decoration to the specified cell
    fn add_strikethrough(&mut self, x: u16, y: u16, color: Rgb, alpha: u8) -> Result<(), Error> {
        self.add_sprite_decoration(
            CellKey::Strikethrough,
            Sprite::Strikethrough,
            x,
            y,
            color,
            alpha,
        )
    }

    /// Add a glyph to the specified cell.
    #[allow(clippy::too_many_arguments)]
    fn add_glyph(
        &mut self,
        config: &DerivedConfig,
        x: u16,
        y: u16,
        cols: usize,
        cell_raws: &[PageCell],
        shaper_cell: shape::Cell,
        shaper_run: &TextRun,
        color: Rgb,
        alpha: u8,
    ) -> Result<(), Error> {
        let cell = cell_raws[usize::from(x)];
        let cp = cell.codepoint();

        // Render
        let render = self.font_grid.render_glyph(
            shaper_run.font_index,
            shaper_cell.glyph_index,
            RenderOptions {
                thicken: config.font_thicken,
                thicken_strength: config.font_thicken_strength,
                cell_width: Some(cell.grid_width()),
                // If there's no Nerd Font constraint for this codepoint
                // then, if it's a symbol, we constrain it to fit inside
                // its cell(s), we don't modify the alignment at all.
                constraint: get_constraint(cp).unwrap_or(if cell::is_symbol(cp) {
                    Constraint {
                        size: ConstraintSize::Fit,
                        ..Constraint::NONE
                    }
                } else {
                    Constraint::NONE
                }),
                constraint_width: cell::constraint_width(cell_raws, usize::from(x), cols),
                ..RenderOptions::new(self.grid_metrics)
            },
        )?;

        // If the glyph is 0 width or height, it will be invisible
        // when drawn, so don't bother adding it to the buffer.
        if render.glyph.width == 0 || render.glyph.height == 0 {
            return Ok(());
        }

        self.cells.add(
            CellKey::Text,
            CellText {
                atlas: match render.presentation {
                    Presentation::Emoji => CellTextAtlas::Color as u8,
                    Presentation::Text => CellTextAtlas::Grayscale as u8,
                },
                bools: if cell::no_min_contrast(cp) {
                    CellText::NO_MIN_CONTRAST
                } else {
                    0
                },
                grid_pos: [x, y],
                color: [color.r, color.g, color.b, alpha],
                glyph_pos: [render.glyph.atlas_x, render.glyph.atlas_y],
                glyph_size: [render.glyph.width, render.glyph.height],
                bearings: [
                    (render.glyph.offset_x + i32::from(shaper_cell.x_offset)) as i16,
                    (render.glyph.offset_y + i32::from(shaper_cell.y_offset)) as i16,
                ],
                ..CellText::default()
            },
        );
        Ok(())
    }

    fn add_cursor(&mut self, env: &RowEnv<'_>, cursor_style: CursorStyle, cursor_color: Rgb) {
        let cursor_state = env.cursor;
        let Some(cursor_vp) = cursor_state.viewport else {
            return;
        };

        // Add the cursor. We render the cursor over the wide character if
        // we're on the wide character tail.
        let (wide, x) = if cursor_vp.wide_tail {
            // If we're part of a wide character, we move the cursor back
            // to the actual character.
            (true, cursor_vp.x.saturating_sub(1))
        } else {
            // The cursor goes over the screen cursor position.
            (cursor_state.cell.wide() == Wide::Wide, cursor_vp.x)
        };

        let alpha: u8 = if self.focused {
            (255.0 * env.config.cursor_opacity).ceil() as u8
        } else {
            255
        };

        let render = match cursor_style {
            CursorStyle::Block
            | CursorStyle::BlockHollow
            | CursorStyle::Bar
            | CursorStyle::Underline => {
                let sprite = match cursor_style {
                    CursorStyle::Block => Sprite::CursorRect,
                    CursorStyle::BlockHollow => Sprite::CursorHollowRect,
                    CursorStyle::Bar => Sprite::CursorBar,
                    CursorStyle::Underline => Sprite::CursorUnderline,
                    CursorStyle::Lock => unreachable!(),
                };

                match self.font_grid.render_glyph(
                    SPRITE_INDEX,
                    sprite as u32,
                    self.sprite_options(if wide { 2 } else { 1 }),
                ) {
                    Ok(r) => r,
                    Err(err) => {
                        log::warn!("error rendering cursor glyph err={err}");
                        return;
                    }
                }
            }

            CursorStyle::Lock => {
                match self.font_grid.render_codepoint(
                    0xF023, // lock symbol
                    FontStyle::Regular,
                    Some(Presentation::Text),
                    self.sprite_options(if wide { 2 } else { 1 }),
                ) {
                    Ok(Some(r)) => r,
                    Ok(None) => {
                        // This should never happen because we embed nerd
                        // fonts so we just log and return instead of fallback.
                        log::warn!("failed to find lock symbol for cursor codepoint=0xF023");
                        return;
                    }
                    Err(err) => {
                        log::warn!("error rendering cursor glyph err={err}");
                        return;
                    }
                }
            }
        };

        self.cells.set_cursor(
            Some(CellText {
                atlas: CellTextAtlas::Grayscale as u8,
                bools: CellText::IS_CURSOR_GLYPH,
                grid_pos: [x, cursor_vp.y],
                color: [cursor_color.r, cursor_color.g, cursor_color.b, alpha],
                glyph_pos: [render.glyph.atlas_x, render.glyph.atlas_y],
                glyph_size: [render.glyph.width, render.glyph.height],
                bearings: [render.glyph.offset_x as i16, render.glyph.offset_y as i16],
                ..CellText::default()
            }),
            Some(cursor_style),
        );
    }

    fn add_preedit_cell(
        &mut self,
        config: &DerivedConfig,
        codepoint: u32,
        wide: bool,
        coord: Coordinate,
        screen_fg: Rgb,
    ) -> Result<(), Error> {
        let _ = config;
        // Render the glyph for our preedit text
        let render = match self.font_grid.render_codepoint(
            codepoint,
            FontStyle::Regular,
            Some(Presentation::Text),
            RenderOptions::new(self.grid_metrics),
        ) {
            Ok(Some(r)) => r,
            Ok(None) => {
                log::warn!("failed to find font for preedit codepoint={codepoint:X}");
                return Ok(());
            }
            Err(err) => {
                log::warn!("error rendering preedit glyph err={err}");
                return Ok(());
            }
        };

        // Add our text
        self.cells.add(
            CellKey::Text,
            CellText {
                atlas: CellTextAtlas::Grayscale as u8,
                grid_pos: [coord.x, coord.y as u16],
                color: [screen_fg.r, screen_fg.g, screen_fg.b, 255],
                glyph_pos: [render.glyph.atlas_x, render.glyph.atlas_y],
                glyph_size: [render.glyph.width, render.glyph.height],
                bearings: [render.glyph.offset_x as i16, render.glyph.offset_y as i16],
                ..CellText::default()
            },
        );

        // Add underline
        self.add_underline(coord.x, coord.y as u16, Underline::Single, screen_fg, 255)?;
        if wide && coord.x < self.cells.size.columns - 1 {
            self.add_underline(
                coord.x + 1,
                coord.y as u16,
                Underline::Single,
                screen_fg,
                255,
            )?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// draw_frame (GL thread)

impl Renderer {
    /// Draw the frame to the screen.
    ///
    /// If `sync` is true, this will synchronously block until
    /// the frame is finished drawing and has been presented.
    ///
    /// Must be called with the GL context current (for GTK: from the
    /// `GtkGLArea` `render` signal, on the main thread).
    pub fn draw_frame(&self, sync: bool) -> Result<(), Error> {
        // We hold a the draw mutex to prevent changes to any
        // data we access while we're in the middle of drawing.
        let mut d = lock(&self.draw);

        let result = self.draw_frame_locked(&mut d, sync);

        // After the graphics API is complete (so we defer) we want to
        // update our scrollbar state.
        if d.scrollbar_dirty {
            // Fail instantly if the surface mailbox if full, we'll just
            // get it on the next frame.
            if self.host.scrollbar(d.scrollbar) {
                d.scrollbar_dirty = false;
            }
        }

        // Let our graphics API do any bookkeeping, etc.
        // that it needs to do before / after `draw_frame`.
        d.api.draw_frame_end();

        result
    }

    fn draw_frame_locked(&self, d: &mut DrawState, sync: bool) -> Result<(), Error> {
        d.api.draw_frame_start();

        // Retrieve the most up-to-date surface size from the Graphics API
        let (surface_width, surface_height) = d.api.surface_size()?;

        // If either of our surface dimensions is zero
        // then drawing is absurd, so we just return.
        if surface_width == 0 || surface_height == 0 {
            return Ok(());
        }

        let size_changed =
            d.size.screen.width != surface_width || d.size.screen.height != surface_height;

        // Conditions under which we need to draw the frame, otherwise we
        // don't need to since the previous frame should be identical.
        let needs_redraw = size_changed || d.cells_rebuilt || self.has_animations() || sync;

        if !needs_redraw {
            // We still need to present the last target again, because the
            // apprt may be swapping buffers and display an outdated frame
            // if we don't draw something new.
            d.api.present_last_target()?;
            return Ok(());
        }
        d.cells_rebuilt = false;

        // Get a frame state to draw to.
        let (idx, mut frame) = d.swap_chain.next_frame()?;
        let result = d.draw_with_frame(
            &mut frame,
            sync,
            (surface_width, surface_height),
            size_changed,
        );
        d.swap_chain.release_frame(idx, frame);

        let health = result?;
        self.has_animations
            .store(d.has_custom_shaders, Ordering::Relaxed);
        self.frame_completed(health);
        Ok(())
    }
}

impl DrawState {
    fn draw_with_frame(
        &mut self,
        frame: &mut FrameState,
        sync: bool,
        surface_size: (u32, u32),
        size_changed: bool,
    ) -> Result<Health, Error> {
        let (surface_width, surface_height) = surface_size;

        // If we need to reinitialize our shaders, do so.
        if self.reinitialize_shaders {
            self.reinitialize_shaders = false;
            self.shaders = None;
            self.init_shaders()?;
        }

        // Our shaders should not be defunct at this point.
        if self.shaders.is_none() {
            return Err(Error::Defunct);
        }

        // If we have custom shaders, make sure we have the
        // custom shader state in our frame state, otherwise
        // if we have a state but don't need it we remove it.
        if self.has_custom_shaders {
            if frame.custom_shader_state.is_none() {
                let mut state = CustomShaderState::new(&self.api)?;
                state.resize(&self.api, surface_width as usize, surface_height as usize)?;
                frame.custom_shader_state = Some(state);
            }
        } else {
            frame.custom_shader_state = None;
        }

        // If our stored size doesn't match the
        // surface size we need to update it.
        if size_changed {
            self.size.screen = crate::size::ScreenSize {
                width: surface_width,
                height: surface_height,
            };
            self.update_screen_size_uniforms();
        }

        // If this frame's target isn't the correct size, or the target
        // config has changed (such as when the blending mode changes),
        // remove it and replace it with a new one with the right values.
        if frame.target.width != self.size.screen.width as usize
            || frame.target.height != self.size.screen.height as usize
            || frame.target_config_modified != self.target_config_modified
        {
            frame.resize(
                &self.api,
                self.size.screen.width as usize,
                self.size.screen.height as usize,
            )?;
            frame.target_config_modified = self.target_config_modified;
            // The remembered last target (if any) is the old one.
            self.api.clear_last_target();
        }

        let gl: Gl = self.api.gl().ok_or(opengl::Error::NoContext)?.clone();
        let gpu = Gpu {
            gl: &gl,
            texture_options: self
                .api
                .image_texture_options(opengl::ImageTextureFormat::Rgba, true),
            buffer_options: self.api.image_buffer_options(),
        };

        // Upload images to the GPU as necessary.
        let _ = self.images.upload(&gpu);

        // Upload the background image to the GPU as necessary.
        self.upload_background_image(&gpu);

        // Update per-frame custom shader uniforms.
        self.update_custom_shader_uniforms_for_frame();

        // Setup our frame data
        frame.uniforms.sync(&[self.uniforms])?;
        frame.cells_bg.sync(&self.cells.bg_cells)?;
        let fg_lists: Vec<&[CellText]> = self.cells.fg_rows.iter().map(Vec::as_slice).collect();
        let fg_count = frame.cells.sync_from_slices(&fg_lists)?;
        drop(fg_lists);

        // If our background image buffer has changed, sync it.
        if frame.bg_image_buffer_modified != self.bg_image_buffer_modified {
            frame.bg_image_buffer.sync(&[self.bg_image_buffer])?;

            frame.bg_image_buffer_modified = self.bg_image_buffer_modified;
        }

        // If our font atlas changed, sync the texture data
        {
            let grid = Arc::clone(&self.font_grid);
            let modified = grid.read().atlas_grayscale.modified.load(Ordering::Relaxed);
            if modified > frame.grayscale_modified {
                let guard = grid.read();
                frame.grayscale_modified = guard.atlas_grayscale.modified.load(Ordering::Relaxed);
                DrawState::sync_atlas_texture(
                    &self.api,
                    &guard.atlas_grayscale,
                    &mut frame.grayscale,
                )?;
            }
            let modified = grid.read().atlas_color.modified.load(Ordering::Relaxed);
            if modified > frame.color_modified {
                let guard = grid.read();
                frame.color_modified = guard.atlas_color.modified.load(Ordering::Relaxed);
                DrawState::sync_atlas_texture(&self.api, &guard.atlas_color, &mut frame.color)?;
            }
        }

        // Sync the custom shader uniforms now while we can still use `self`.
        if let Some(state) = &mut frame.custom_shader_state {
            state.uniforms.sync(&[self.custom_shader_uniforms])?;
        }

        // Get a frame context from the graphics API.
        let DrawState {
            api,
            shaders,
            images,
            bg_image,
            has_overlay,
            ..
        } = self;
        let Some(shaders) = shaders.as_ref() else {
            return Err(Error::Defunct);
        };
        let frame_ctx = api.begin_frame(Arc::clone(&frame.target));

        {
            let attachments = [Attachment {
                target: match &frame.custom_shader_state {
                    Some(state) => AttachmentTarget::Texture(&state.back_texture),
                    None => AttachmentTarget::Target(&frame.target),
                },
                clear_color: Some([0.0, 0.0, 0.0, 0.0]),
            }];
            let mut pass = frame_ctx.render_pass(&attachments);

            // First we draw our background image, if we have one.
            // The bg image shader also draws the main bg color.
            //
            // Otherwise, if we don't have a background image, we
            // draw the background color by itself in its own step.
            //
            // NOTE: We don't use the clear_color for this because that
            //       would require us to do color space conversion on the
            //       CPU-side. In the future when we have utilities for
            //       that we should remove this step and use clear_color.
            match bg_image {
                Some(img) => {
                    if let image::Image::Ready(texture) = img {
                        pass.step(&Step {
                            pipeline: &shaders.bg_image,
                            uniforms: Some(frame.uniforms.raw()),
                            buffers: &[Some(frame.bg_image_buffer.raw())],
                            textures: &[Some(texture)],
                            samplers: &[],
                            draw: Draw::new(Primitive::Triangle, 3),
                        });
                    }
                }
                None => {
                    pass.step(&Step {
                        pipeline: &shaders.bg_color,
                        uniforms: Some(frame.uniforms.raw()),
                        buffers: &[None, Some(frame.cells_bg.raw())],
                        textures: &[],
                        samplers: &[],
                        draw: Draw::new(Primitive::Triangle, 3),
                    });
                }
            }

            // Then we draw any kitty images that need
            // to be behind text AND cell backgrounds.
            images.draw(
                &gpu,
                &shaders.image,
                &mut pass,
                image::DrawPlacements::KittyBelowBg,
            );

            // Then we draw any opaque cell backgrounds.
            pass.step(&Step {
                pipeline: &shaders.cell_bg,
                uniforms: Some(frame.uniforms.raw()),
                buffers: &[None, Some(frame.cells_bg.raw())],
                textures: &[],
                samplers: &[],
                draw: Draw::new(Primitive::Triangle, 3),
            });

            // Kitty images between cell backgrounds and text.
            images.draw(
                &gpu,
                &shaders.image,
                &mut pass,
                image::DrawPlacements::KittyBelowText,
            );

            // Text.
            pass.step(&Step {
                pipeline: &shaders.cell_text,
                uniforms: Some(frame.uniforms.raw()),
                buffers: &[Some(frame.cells.raw()), Some(frame.cells_bg.raw())],
                textures: &[Some(&frame.grayscale), Some(&frame.color)],
                samplers: &[],
                draw: Draw {
                    ty: Primitive::TriangleStrip,
                    vertex_count: 4,
                    instance_count: fg_count,
                },
            });

            // Kitty images in front of text.
            images.draw(
                &gpu,
                &shaders.image,
                &mut pass,
                image::DrawPlacements::KittyAboveText,
            );

            // Debug overlay. We do this before any custom shader state
            // because our debug overlay is aligned with the grid.
            if *has_overlay {
                images.draw(
                    &gpu,
                    &shaders.image,
                    &mut pass,
                    image::DrawPlacements::Overlay,
                );
            }

            pass.complete();
        }

        // If we have custom shaders, then we render them.
        if let Some(state) = &mut frame.custom_shader_state {
            let post_count = shaders.post_pipelines.len();
            for (i, pipeline) in shaders.post_pipelines.iter().enumerate() {
                {
                    let attachments = [Attachment {
                        target: if i < post_count - 1 {
                            AttachmentTarget::Texture(&state.front_texture)
                        } else {
                            AttachmentTarget::Target(&frame.target)
                        },
                        clear_color: Some([0.0, 0.0, 0.0, 0.0]),
                    }];
                    let mut pass = frame_ctx.render_pass(&attachments);

                    pass.step(&Step {
                        pipeline,
                        uniforms: Some(state.uniforms.raw()),
                        buffers: &[],
                        textures: &[Some(&state.back_texture)],
                        samplers: &[Some(&state.sampler)],
                        draw: Draw::new(Primitive::Triangle, 3),
                    });
                    pass.complete();
                }
                state.swap();
            }
        }

        // Complete the frame: glFinish, error check, present.
        Ok(frame_ctx.complete(sync))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghostty_config::types_a::Color as ConfigColor;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    #[test]
    fn derived_config_from_default_config() {
        let config = Config::default();
        let d = DerivedConfig::new(&config).unwrap();
        // The default config installs the URL link.
        assert!(!d.links.links.is_empty());
        assert_eq!(d.background, config.background.to_terminal_rgb());
        assert_eq!(d.foreground, config.foreground.to_terminal_rgb());
        assert!((d.background_opacity - 1.0).abs() < f64::EPSILON);
        // faint-opacity 0.5 -> ceil(127.5) = 128
        assert_eq!(d.faint_opacity, 128);
        assert_eq!(d.font_styles, [true; 4]);
        assert_eq!(d.custom_shaders.len(), 0);
        assert!(d.bg_image.is_none());
        assert!(!d.scroll_to_bottom_on_output);
    }

    #[test]
    fn derived_config_clamps_opacity() {
        let config = Config {
            background_opacity: 3.0,
            cursor_opacity: -1.0,
            ..Config::default()
        };
        let d = DerivedConfig::new(&config).unwrap();
        assert!((d.background_opacity - 1.0).abs() < f64::EPSILON);
        assert!(d.cursor_opacity.abs() < f64::EPSILON);
    }

    #[test]
    fn terminal_color_resolution() {
        let fg = Some(rgb(1, 1, 1));
        let bg = Some(rgb(2, 2, 2));
        let explicit = TerminalColor::Color(ConfigColor { r: 9, g: 9, b: 9 });
        assert_eq!(
            resolve_terminal_color(explicit, false, fg, bg),
            Some(rgb(9, 9, 9))
        );
        assert_eq!(
            resolve_terminal_color(TerminalColor::CellForeground, false, fg, bg),
            fg
        );
        assert_eq!(
            resolve_terminal_color(TerminalColor::CellForeground, true, fg, bg),
            bg
        );
        assert_eq!(
            resolve_terminal_color(TerminalColor::CellBackground, false, fg, bg),
            bg
        );
        assert_eq!(
            resolve_terminal_color(TerminalColor::CellBackground, true, fg, bg),
            fg
        );
    }

    #[test]
    fn bold_color_policy() {
        assert!(bold_policy(None).is_none());
        assert!(matches!(
            bold_policy(Some(BoldColor::Bright)),
            Some(style::BoldColor::Bright)
        ));
        let c = ConfigColor { r: 1, g: 2, b: 3 };
        assert!(matches!(
            bold_policy(Some(BoldColor::Color(c))),
            Some(style::BoldColor::Color(Rgb { r: 1, g: 2, b: 3 }))
        ));
    }

    #[test]
    fn file_type_detection() {
        assert_eq!(
            detect_file_type(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]),
            FileType::Png
        );
        assert_eq!(detect_file_type(&[0xFF, 0xD8, 0xFF, 0xE0]), FileType::Jpeg);
        assert_eq!(detect_file_type(b"GIF89a.."), FileType::Gif);
        assert_eq!(detect_file_type(b"hello"), FileType::Unknown);
        assert_eq!(guess_file_type_from_extension("/a/b.JPG"), FileType::Jpeg);
        assert_eq!(guess_file_type_from_extension("/a/b.png"), FileType::Png);
        assert_eq!(guess_file_type_from_extension("/a/b"), FileType::Unknown);
    }

    #[test]
    fn background_image_png_roundtrip() {
        // Encode a 2x1 RGBA png, write it to disk and load it back.
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, 2, 1);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[1, 2, 3, 255, 4, 5, 6, 128]).unwrap();
        }
        let dir = std::env::temp_dir().join(format!("ghostty-bg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bg.png");
        std::fs::write(&path, &bytes).unwrap();

        let img = load_background_image(path.to_str().unwrap()).expect("decoded");
        match img {
            image::Image::Pending(p) => {
                assert_eq!((p.width, p.height), (2, 1));
                assert_eq!(p.data, [1, 2, 3, 255, 4, 5, 6, 128]);
            }
            _ => panic!("expected a pending image"),
        }

        // Missing / undecodable files are not an error, just no image.
        assert!(load_background_image(dir.join("missing.png").to_str().unwrap()).is_none());
        let bad = dir.join("bad.txt");
        std::fs::write(&bad, b"not an image").unwrap();
        assert!(load_background_image(bad.to_str().unwrap()).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
