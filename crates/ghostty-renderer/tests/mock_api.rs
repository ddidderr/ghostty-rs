//! The generic renderer over a mock graphics API: no GPU, no GL context.
//!
//! The mock completes frames on a worker thread (like Metal's command buffer
//! completion handlers) with a swap chain of three frames, which exercises the
//! swap chain hand-back, health reporting and the step/slot conventions the
//! real backends rely on.

#![allow(clippy::unwrap_used)]

use std::{
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytemuck::Pod;
use ghostty_config::{config::Config, types_b::AlphaBlending};
use ghostty_font::{
    Atlas,
    face::DesiredSize,
    shared_grid_set::{DerivedConfig as FontDerivedConfig, SharedGridSet},
};
use ghostty_renderer::{
    Health,
    generic::{DerivedConfig, Host, Renderer},
    graphics_api::{
        ApiError,
        Attachment,
        FrameCompletion,
        GpuBuffer,
        GpuFrame,
        GpuRenderPass,
        GpuShaders,
        GpuTarget,
        GpuTexture,
        GraphicsApi,
        Step,
        TextureKind,
    },
    options::Options,
    shader_types::Uniforms,
    shadertoy,
    size::{CellSize, Padding, ScreenSize, Size},
    state::State,
};
use ghostty_terminal::{
    page_list::Scrollbar,
    terminal::{Options as TermOptions, Terminal},
};

/// What the mock saw: one entry per render pass step, as
/// `pipeline:instances`.
type Log = Arc<Mutex<Vec<String>>>;

struct MockApi {
    blending: AlphaBlending,
    log: Log,
    completed: Arc<AtomicUsize>,
    surface: (u32, u32),
}

struct MockTarget(usize, usize);
impl GpuTarget for MockTarget {
    fn width(&self) -> usize {
        self.0
    }
    fn height(&self) -> usize {
        self.1
    }
}

struct MockTexture {
    w: usize,
    h: usize,
}
impl GpuTexture for MockTexture {
    fn width(&self) -> usize {
        self.w
    }
    fn height(&self) -> usize {
        self.h
    }
    fn replace_region(
        &self,
        _x: usize,
        _y: usize,
        _w: usize,
        _h: usize,
        _data: &[u8],
    ) -> Result<(), ApiError> {
        Ok(())
    }
}

struct MockBuffer<T> {
    id: usize,
    len: usize,
    _marker: std::marker::PhantomData<fn() -> T>,
}
impl<T: Pod> GpuBuffer<MockApi, T> for MockBuffer<T> {
    fn raw(&self) -> usize {
        self.id
    }
    fn sync(&mut self, data: &[T]) -> Result<(), ApiError> {
        self.len = data.len();
        Ok(())
    }
    fn sync_from_slices(&mut self, lists: &[&[T]]) -> Result<usize, ApiError> {
        self.len = lists.iter().map(|l| l.len()).sum();
        Ok(self.len)
    }
}

struct MockPipeline(&'static str);

struct MockShaders {
    pipelines: [MockPipeline; 5],
    post: Vec<MockPipeline>,
}
impl GpuShaders<MockApi> for MockShaders {
    fn bg_color(&self) -> &MockPipeline {
        &self.pipelines[0]
    }
    fn cell_bg(&self) -> &MockPipeline {
        &self.pipelines[1]
    }
    fn cell_text(&self) -> &MockPipeline {
        &self.pipelines[2]
    }
    fn image(&self) -> &MockPipeline {
        &self.pipelines[3]
    }
    fn bg_image(&self) -> &MockPipeline {
        &self.pipelines[4]
    }
    fn post_pipelines(&self) -> &[MockPipeline] {
        &self.post
    }
}

struct MockFrame {
    log: Log,
    completed: Arc<AtomicUsize>,
}
struct MockPass {
    log: Log,
}
impl GpuRenderPass<'_, MockApi> for MockPass {
    fn step(&mut self, step: &Step<'_, MockApi>) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:{}", step.pipeline.0, step.draw.instance_count));
    }
    fn complete(self) {
        self.log.lock().unwrap().push("pass-end".into());
    }
}
impl GpuFrame<MockApi> for MockFrame {
    fn render_pass<'a>(&'a self, _attachments: &'a [Attachment<'a, MockApi>]) -> MockPass
    where
        MockApi: 'a,
    {
        MockPass {
            log: self.log.clone(),
        }
    }
    fn complete(self, sync: bool, completion: FrameCompletion) {
        let completed = self.completed;
        let done = move || {
            std::thread::sleep(Duration::from_millis(5));
            completion(Health::Healthy);
            completed.fetch_add(1, Ordering::SeqCst);
        };
        if sync {
            done();
        } else {
            std::thread::spawn(done);
        }
    }
}

impl GraphicsApi for MockApi {
    const CUSTOM_SHADER_TARGET: shadertoy::Target = shadertoy::Target::Msl;
    const CUSTOM_SHADER_Y_IS_DOWN: bool = true;
    const SWAP_CHAIN_COUNT: usize = 3;

    type Target = MockTarget;
    type Texture = MockTexture;
    type Sampler = ();
    type Buffer<T: Pod> = MockBuffer<T>;
    type BufferRef = usize;
    type Pipeline = MockPipeline;
    type Shaders = MockShaders;
    type GpuUniforms = Uniforms;
    type Frame = MockFrame;
    type RenderPass<'a> = MockPass;

    fn gpu_uniforms(u: &Uniforms) -> Uniforms {
        *u
    }
    fn blending(&self) -> AlphaBlending {
        self.blending
    }
    fn set_blending(&mut self, b: AlphaBlending) {
        self.blending = b;
    }
    fn init_shaders(&self, _custom: &[String]) -> Result<MockShaders, ApiError> {
        Ok(MockShaders {
            pipelines: [
                MockPipeline("bg_color"),
                MockPipeline("cell_bg"),
                MockPipeline("cell_text"),
                MockPipeline("image"),
                MockPipeline("bg_image"),
            ],
            post: Vec::new(),
        })
    }
    fn surface_size(&self) -> Result<(u32, u32), ApiError> {
        Ok(self.surface)
    }
    fn init_target(&self, w: usize, h: usize) -> Result<MockTarget, ApiError> {
        Ok(MockTarget(w, h))
    }
    fn init_buffer<T: Pod>(&self, len: usize) -> Result<MockBuffer<T>, ApiError> {
        Ok(MockBuffer {
            id: 1,
            len,
            _marker: std::marker::PhantomData,
        })
    }
    fn init_buffer_fill<T: Pod>(&self, data: &[T]) -> Result<MockBuffer<T>, ApiError> {
        self.init_buffer(data.len())
    }
    fn init_texture(
        &self,
        _kind: TextureKind,
        w: usize,
        h: usize,
        _data: Option<&[u8]>,
    ) -> Result<MockTexture, ApiError> {
        Ok(MockTexture { w, h })
    }
    fn init_atlas_texture(&self, atlas: &Atlas) -> Result<MockTexture, ApiError> {
        Ok(MockTexture {
            w: atlas.size as usize,
            h: atlas.size as usize,
        })
    }
    fn init_sampler(&self) -> Result<(), ApiError> {
        Ok(())
    }
    fn begin_frame(&self, _target: Arc<MockTarget>) -> MockFrame {
        MockFrame {
            log: self.log.clone(),
            completed: self.completed.clone(),
        }
    }
    fn present_last_target(&self) -> Result<(), ApiError> {
        Ok(())
    }
    fn clear_last_target(&self) {}
}

#[derive(Default)]
struct TestHost {
    health: Mutex<Vec<Health>>,
}
impl Host for TestHost {
    fn scrollbar(&self, _: Scrollbar) -> bool {
        true
    }
    fn renderer_health(&self, h: Health) {
        self.health.lock().unwrap().push(h);
    }
    fn redraw_surface(&self) -> bool {
        true
    }
}

fn fixture() -> (Renderer<MockApi>, Arc<Mutex<State>>, Log, Arc<AtomicUsize>) {
    let config = Config::default();
    let set = Arc::new(SharedGridSet::new().unwrap());
    let (_key, grid) = set
        .ref_(
            &FontDerivedConfig::new(&config),
            DesiredSize {
                points: 12.0,
                xdpi: 96,
                ydpi: 96,
            },
        )
        .unwrap();
    let gc = grid.cell_size();
    let cell = CellSize {
        width: gc.width,
        height: gc.height,
    };
    let (w, h) = (cell.width * 20 + 20, cell.height * 5 + 20);
    let size = Size {
        screen: ScreenSize {
            width: w,
            height: h,
        },
        cell,
        padding: Padding {
            top: 10,
            bottom: 10,
            left: 10,
            right: 10,
        },
    };

    let mut term = Terminal::new(TermOptions::new(20, 5)).unwrap();
    term.vt_stream().next_slice(b"hello").unwrap();
    let state = State::new(term).into_shared();

    let log = Log::default();
    let completed = Arc::new(AtomicUsize::new(0));
    let api = MockApi {
        blending: AlphaBlending::Native,
        log: log.clone(),
        completed: completed.clone(),
        surface: (w, h),
    };
    let renderer = Renderer::<MockApi>::new(
        Options {
            config: DerivedConfig::new(&config).unwrap(),
            font_grid: grid,
            size,
            host: Arc::new(TestHost::default()),
        },
        api,
    )
    .unwrap();
    (renderer, state, log, completed)
}

#[test]
fn async_triple_buffered_frames_complete_and_release() {
    let (renderer, state, log, completed) = fixture();

    // More frames than the swap chain holds: `draw_frame` has to wait for
    // earlier frames to be handed back by their completion callbacks.
    for i in 0..10 {
        renderer.update_frame(&state, true).unwrap();
        renderer
            .draw_frame(true)
            .unwrap_or_else(|_| panic!("draw {i}"));
    }
    // sync = true completes inline, so each draw was fully completed.
    assert_eq!(completed.load(Ordering::SeqCst), 10);

    for _ in 0..10 {
        renderer.mark_dirty();
        renderer.update_frame(&state, true).unwrap();
        // Asynchronous completion on worker threads.
        renderer.draw_frame(false).unwrap();
        renderer.draw_frame(true).unwrap();
    }
    // Wait for the async completions.
    for _ in 0..200 {
        if completed.load(Ordering::SeqCst) >= 30 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(completed.load(Ordering::SeqCst), 30);
    assert_eq!(renderer.health(), Health::Healthy);

    // Every drawn frame ran bg_color, cell_bg and cell_text steps.
    let log = log.lock().unwrap();
    let bg = log.iter().filter(|l| l.starts_with("bg_color:")).count();
    let cell_bg = log.iter().filter(|l| l.starts_with("cell_bg:")).count();
    let text: Vec<_> = log.iter().filter(|l| l.starts_with("cell_text:")).collect();
    assert_eq!(bg, 30);
    assert_eq!(cell_bg, 30);
    assert_eq!(text.len(), 30);
    // "hello" is five glyph instances plus the cursor glyph.
    assert!(text.iter().all(|l| l.as_str() == "cell_text:6"), "{text:?}");
}

#[test]
fn display_unrealize_waits_for_inflight_frames() {
    let (renderer, state, _log, completed) = fixture();
    renderer.update_frame(&state, true).unwrap();
    renderer.draw_frame(false).unwrap();
    // Deinitializing the swap chain waits until in-flight frames are back.
    renderer.display_unrealized();
    assert_eq!(completed.load(Ordering::SeqCst), 1);
    renderer.display_realized_with(|_| {}).unwrap();
    renderer.draw_frame(true).unwrap();
    assert_eq!(completed.load(Ordering::SeqCst), 2);
}
