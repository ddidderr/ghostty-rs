//! `performBindingAction`, search, and screen file dumps (Zig `Surface.zig`:
//! `Search`, `searchCallback`, `dumpText`, `performBindingAction`,
//! `closingAction`, `writeScreenFile`).

use std::{io::Write as _, sync::Arc, thread::JoinHandle};

use ghostty_apprt::{
    action::{
        Action as RtAction,
        CloseTabMode as RtCloseTabMode,
        FloatWindow,
        Fullscreen,
        GotoSplit as RtGotoSplit,
        GotoTab,
        GotoWindow as RtGotoWindow,
        Inspector as RtInspector,
        KeyTable,
        MoveTab,
        OpenUrl,
        OpenUrlKind,
        PromptTitle,
        Readonly,
        ReloadConfig,
        ResizeSplit as RtResizeSplit,
        ResizeSplitDirection,
        SecureInput,
        SetTitle,
        SplitDirection as RtSplit,
        StartSearch,
        Target,
    },
    runtime_api::{CoreError, RtApp, SurfaceId},
    structs::{Clipboard, ClipboardRequest},
    surface::Message as SurfaceMessage,
};
use ghostty_config::types_a::NonNativeFullscreen;
use ghostty_datastruct::blocking_queue::Timeout;
use ghostty_input::binding::{
    Action,
    AdjustSelection,
    CloseTabMode,
    CrashThread,
    InspectorMode,
    NavigateSearch,
    Scope,
    SplitDirection,
    SplitFocusDirection,
    SplitResizeDirection,
    WriteScreen,
    WriteScreenAction,
    WriteScreenFormat,
};
use ghostty_renderer::{
    message::Message as RendererMessage,
    state::SharedState,
    thread::Handle as RendererHandle,
};
use ghostty_terminal::{
    formatter::{self, Content, Format, ScreenFormatter},
    point::Tag,
    screen::Scroll,
    search::{
        screen::Select,
        thread::{
            Event,
            Message as SearchMessage,
            Options as SearchOptions,
            TerminalLock,
            TerminalRef,
            Thread as SearchThread,
        },
    },
    selection::{Adjustment, Order, Selection},
    terminal::{ScrollViewport, Terminal},
};
use ghostty_termio::{SharedStateExt, TerminalGuard, message::Message as IoMessage};

use super::Surface;
use crate::app::Mailbox as AppMailbox;

/// The maximum number of key tables that can be active at any given time
/// (Zig: `max_active_key_tables`). Enforced by the keyboard module.
pub const MAX_ACTIVE_KEY_TABLES: usize = 8;

// ----------------------------------------------------------------------
// Search

/// The search state for the surface (Zig: `Surface.Search`).
pub struct Search {
    handle: ghostty_terminal::search::thread::Handle,
    thread: Option<JoinHandle<()>>,
}

/// Lets the search thread lock the terminal inside the shared render state.
struct StateLock(SharedState);

impl TerminalLock for StateLock {
    fn lock_terminal(&self) -> Box<dyn TerminalRef + '_> {
        Box::new(self.0.lock_terminal())
    }
}

// `TerminalGuard` derefs mutably to the terminal.
fn _assert_guard(g: TerminalGuard<'_>) -> impl TerminalRef + '_ {
    g
}

impl Search {
    /// Start the search thread (nothing is searched until a needle is sent).
    fn start(surface: &Surface) -> Result<Search, CoreError> {
        let renderer = surface.renderer_handle.clone();
        let mailbox = surface.surface_mailbox();
        let cb = Arc::new(move |event: Event| search_callback(&renderer, &mailbox, event));
        let (handle, thread) = SearchThread::spawn(SearchOptions {
            terminal: Arc::new(StateLock(Arc::clone(&surface.renderer_state))),
            event_cb: Some(cb),
        })?;
        Ok(Search {
            handle,
            thread: Some(thread),
        })
    }

    /// Stop the search thread and free the state.
    pub fn deinit(mut self) {
        // Notify the thread to stop
        self.handle.stop();

        // Wait for the OS thread to quit
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            log::warn!("search thread panicked");
        }
    }
}

/// Events from the search thread (Zig: `searchCallback_`).
///
/// IMPORTANT: this runs on the SEARCH THREAD! It must only touch things that
/// are safe from any thread.
fn search_callback(
    renderer: &RendererHandle,
    mailbox: &ghostty_apprt::surface::Mailbox<SurfaceId, AppMailbox>,
    event: Event,
) {
    match event {
        Event::ViewportMatches(matches) => {
            renderer.send(RendererMessage::SearchViewportMatches(matches));
        }

        Event::SelectedMatch(selected) => {
            if let Some(sel) = selected {
                renderer.send(RendererMessage::SearchSelectedMatch(Some(sel.highlight)));

                // Send the selected index to the surface mailbox
                mailbox.push(
                    SurfaceMessage::SearchSelected(Some(sel.idx)),
                    Timeout::Forever,
                );
            } else {
                // Reset our selected match
                renderer.send(RendererMessage::SearchSelectedMatch(None));

                // Reset the selected index
                mailbox.push(SurfaceMessage::SearchSelected(None), Timeout::Forever);
            }
        }

        Event::TotalMatches(total) => {
            mailbox.push(SurfaceMessage::SearchTotal(Some(total)), Timeout::Forever);
        }

        // When we quit, tell our renderer to reset any search state.
        Event::Quit => {
            renderer.send(RendererMessage::SearchSelectedMatch(None));
            renderer.send(RendererMessage::SearchViewportMatches(Vec::new()));

            // Reset search totals in the surface
            mailbox.push(SurfaceMessage::SearchTotal(None), Timeout::Forever);
            mailbox.push(SurfaceMessage::SearchSelected(None), Timeout::Forever);
        }

        // Unhandled, so far.
        Event::Complete => {}
    }
}

// ----------------------------------------------------------------------
// Text dumping

/// Represents text read from the terminal and some metadata about it that is
/// often useful to apprts (Zig: `Surface.Text`).
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    /// The text that was read from the terminal.
    pub text: String,

    /// The viewport information about this text, if it is visible in the
    /// viewport.
    pub viewport: Option<TextViewport>,
}

/// See [`Text::viewport`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextViewport {
    /// The top-left corner of the selection in pixels within the viewport.
    pub tl_px_x: f64,
    pub tl_px_y: f64,

    /// The linear offset of the start of the selection and the length. This
    /// is "linear" in the sense that it is the offset in the flattened
    /// viewport as a single array of text.
    ///
    /// Note: these values are currently wrong if there is a partially
    /// visible selection in the viewport (i.e. the top-left or bottom-right
    /// of the selection is outside the viewport). The wrong values will
    /// always be within the text bounds so we aren't risking an overflow.
    pub offset_start: u32,
    pub offset_len: u32,
}

/// Pixel metrics needed to place a text selection within the viewport.
#[derive(Debug, Clone, Copy)]
pub struct ViewportMetrics {
    pub cell_width: u32,
    pub cell_height: u32,
    pub cell_baseline: u32,
    pub padding_left: u32,
    pub padding_top: u32,
    pub scale_x: f32,
    pub scale_y: f32,
}

/// The viewport placement of `sel` (the second half of Zig's
/// `dumpTextLocked`).
#[must_use]
pub fn text_viewport(t: &Terminal, sel: Selection, m: ViewportMetrics) -> Option<TextViewport> {
    let screen = t.screen();
    let pages = &screen.pages;

    // If our bottom right pin is before the viewport, then we can't
    // possibly have this text be within the viewport.
    let vp_tl_pin = pages.get_top_left(Tag::Viewport)?;
    let br_pin = sel.bottom_right(pages);
    if br_pin.before(pages, vp_tl_pin) {
        return None;
    }

    // If our top-left pin is after the viewport, then we can't possibly have
    // this text be within the viewport.
    let Some(vp_br_pin) = pages.get_bottom_right(Tag::Viewport) else {
        log::warn!("viewport bottom-right pin not found, bug?");
        return None;
    };
    let tl_pin = sel.top_left(pages);
    if vp_br_pin.before(pages, tl_pin) {
        return None;
    }

    // We established that our top-left somewhere before the viewport
    // bottom-right and that our bottom-right is somewhere after the
    // top-left. This means that at least some portion of our selection is
    // within the viewport.

    // Our top-left point. If it doesn't exist in the viewport it must be
    // before and we can return (0,0).
    let tl_coord = pages
        .point_from_pin(Tag::Viewport, tl_pin)
        .map_or((0u32, 0u32), |p| {
            let c = p.coord();
            (u32::from(c.x), c.y)
        });

    // Our bottom-right point. If it doesn't exist in the viewport it must be
    // the bottom-right of the viewport.
    let br_coord = pages
        .point_from_pin(Tag::Viewport, br_pin)
        .or_else(|| pages.point_from_pin(Tag::Viewport, vp_br_pin))
        .map(|p| {
            let c = p.coord();
            (u32::from(c.x), c.y)
        })?;

    // Our sizes are all scaled so we need to send the unscaled values back.
    let x = {
        // Simple x * cell width gives the left
        let mut x = f64::from(tl_coord.0 * m.cell_width);
        // Add padding
        x += f64::from(m.padding_left);
        // Scale
        x / f64::from(m.scale_x)
    };
    let y = {
        // Simple y * cell height gives the top
        let mut y = f64::from(tl_coord.1 * m.cell_height);
        // We want the text baseline
        y += f64::from(m.cell_height);
        y -= f64::from(m.cell_baseline);
        // Add padding
        y += f64::from(m.padding_top);
        // Scale
        y / f64::from(m.scale_y)
    };

    // Utilize viewport sizing to convert to offsets
    let cols = u32::from(pages.cols);
    let start = tl_coord.1 * cols + tl_coord.0;
    let end = br_coord.1 * cols + br_coord.0;

    Some(TextViewport {
        tl_px_x: x,
        tl_px_y: y,
        offset_start: start,
        offset_len: end.saturating_sub(start),
    })
}

// ----------------------------------------------------------------------
// Screen files

/// The portion of the screen to write for `writeScreenFile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteScreenLoc {
    /// Full screen
    Screen,
    /// History (scrollback)
    History,
    /// Selected text
    Selection,
}

impl WriteScreenLoc {
    fn name(self) -> &'static str {
        match self {
            WriteScreenLoc::Screen => "screen",
            WriteScreenLoc::History => "history",
            WriteScreenLoc::Selection => "selection",
        }
    }
}

/// The file name used for a screen dump.
#[must_use]
pub fn screen_file_name(loc: WriteScreenLoc, emit: WriteScreenFormat) -> String {
    format!(
        "{}.{}",
        loc.name(),
        match emit {
            WriteScreenFormat::Plain | WriteScreenFormat::Vt => "txt",
            WriteScreenFormat::Html => "html",
        }
    )
}

/// Format the requested portion of the active screen. Returns `None` when
/// there is no data (no selection, no history, ...).
pub fn screen_file_contents(
    t: &Terminal,
    loc: WriteScreenLoc,
    emit: WriteScreenFormat,
) -> Result<Option<String>, CoreError> {
    // We only dump history if we have history. We still keep the file and
    // write the empty file to the pty so that this command always works on
    // the primary screen.
    let screen = t.screen();
    let pages = &screen.pages;
    let sel = match loc {
        WriteScreenLoc::History => {
            // We do not support this for alternate screens because they
            // don't have scrollback anyways.
            if t.screens.active_key == ghostty_terminal::screen_set::Key::Alternate {
                None
            } else {
                match (
                    pages.get_top_left(Tag::History),
                    pages.get_bottom_right(Tag::History),
                ) {
                    (Some(tl), Some(br)) => Some(Selection::new(tl, br, false)),
                    _ => None,
                }
            }
        }

        WriteScreenLoc::Screen => match (
            pages.get_top_left(Tag::Screen),
            pages.get_bottom_right(Tag::Screen),
        ) {
            (Some(tl), Some(br)) => Some(Selection::new(tl, br, false)),
            _ => None,
        },

        WriteScreenLoc::Selection => screen.selection,
    };

    // If we have no selection we have no data so we do nothing.
    let Some(sel) = sel else {
        return Ok(None);
    };

    let mut opts = formatter::Options::new(match emit {
        WriteScreenFormat::Plain => Format::Plain,
        WriteScreenFormat::Vt => Format::Vt,
        WriteScreenFormat::Html => Format::Html,
    });
    opts.unwrap = true;
    opts.trim = false;
    opts.background = t.colors.background.get();
    opts.foreground = t.colors.foreground.get();
    opts.palette = Some(t.colors.palette.current);
    let mut f = ScreenFormatter::new(screen, opts);
    f.content = Content::Selection(Some(sel.ordered(pages, Order::Forward)));
    Ok(Some(
        f.format_string()
            .map_err(|e| format!("error formatting screen: {e:?}"))?,
    ))
}

/// Write `contents` to `<tempdir>/<loc>.<ext>` (mode 0600) and return the
/// canonical path. The temporary directory is intentionally left behind so
/// the file stays around for the user (Zig does not delete it either).
/// The raw bytes of a path (what Zig copies/pastes for a screen file).
fn path_bytes(path: &std::path::Path) -> Vec<u8> {
    ghostty_os::osbytes::os_bytes(path.as_os_str())
}

pub fn write_screen_temp_file(
    contents: &str,
    loc: WriteScreenLoc,
    emit: WriteScreenFormat,
) -> Result<std::path::PathBuf, CoreError> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt as _;

    // Create a temporary directory to store our scrollback.
    let tmp_dir = ghostty_os::temp_dir::TempDir::init()?;
    let filename = screen_file_name(loc, emit);
    let file_path = tmp_dir.path().join(&filename);

    let result = (|| -> Result<std::path::PathBuf, CoreError> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        opts.mode(0o600);
        let mut file = opts.open(&file_path)?;
        file.write_all(contents.as_bytes())?;
        file.flush()?;
        Ok(std::fs::canonicalize(&file_path)?)
    })();

    match result {
        Ok(path) => {
            // Keep the directory (and thus the file) on disk.
            std::mem::forget(tmp_dir);
            Ok(path)
        }
        Err(err) => {
            tmp_dir.deinit();
            Err(err)
        }
    }
}

/// Returns true if performing the given action result in closing the
/// surface. This is used to determine if our self pointer is still valid
/// after performing some binding action.
#[must_use]
#[allow(dead_code)]
pub(crate) fn closing_action(action: &Action) -> bool {
    matches!(
        action,
        Action::CloseSurface | Action::CloseWindow | Action::CloseTab(_)
    )
}

/// The number of rows to scroll for `scroll_page_fractional`.
#[must_use]
pub fn fractional_scroll_delta(fraction: f32, rows: u16) -> isize {
    (fraction * f32::from(rows)).trunc() as isize
}

/// Clamp a font size delta like Zig (`@max(0, @min(255, delta))`).
#[must_use]
pub fn clamp_font_delta(delta: f32) -> f32 {
    delta.clamp(0.0, 255.0)
}

fn adjustment_for(a: AdjustSelection) -> Adjustment {
    match a {
        AdjustSelection::Left => Adjustment::Left,
        AdjustSelection::Right => Adjustment::Right,
        AdjustSelection::Up => Adjustment::Up,
        AdjustSelection::Down => Adjustment::Down,
        AdjustSelection::PageUp => Adjustment::PageUp,
        AdjustSelection::PageDown => Adjustment::PageDown,
        AdjustSelection::Home => Adjustment::Home,
        AdjustSelection::End => Adjustment::End,
        AdjustSelection::BeginningOfLine => Adjustment::BeginningOfLine,
        AdjustSelection::EndOfLine => Adjustment::EndOfLine,
    }
}

/// Adjust the selection of the active screen and scroll the endpoint into
/// view. Returns false if there is no selection (the action isn't
/// performed).
pub fn adjust_selection(t: &mut Terminal, direction: AdjustSelection) -> bool {
    let screen = t.screen_mut();
    let Some(mut sel) = screen.selection else {
        // If we don't have a selection we do not perform this action,
        // allowing the keybind to fall through to the terminal.
        return false;
    };
    sel.adjust(&mut screen.pages, adjustment_for(direction));
    screen.selection = Some(sel);

    // If the selection endpoint is outside of the current viewpoint, scroll
    // it in to view. Note we always specifically use sel.end because that is
    // what adjust modifies.
    'scroll: {
        let viewport_tl = screen
            .pages
            .get_top_left(Tag::Viewport)
            .expect("viewport top left");
        let Some(viewport_br) = screen.pages.get_bottom_right(Tag::Viewport) else {
            break 'scroll;
        };
        let end = sel.end(&screen.pages);
        if end.is_between(&screen.pages, viewport_tl, viewport_br) {
            break 'scroll;
        }

        // Our end point is not within the viewport. If the end point is
        // after the br then we need to adjust the end so that it is at the
        // bottom right of the viewport.
        let target = if end.before(&screen.pages, viewport_tl) {
            end
        } else {
            let rows = usize::from(screen.pages.rows);
            end.up(&screen.pages, rows - 1).unwrap_or(end)
        };

        screen.scroll(Scroll::Pin(target));
    }

    // Queue a render so its shown
    screen.dirty.selection = true;
    true
}

/// Perform an app-scoped binding action (Zig: `App.performAction`).
fn perform_app_action(rt: &dyn RtApp, action: &Action) {
    let a = match action {
        Action::Ignore => return,
        Action::Unbind => {
            log::warn!("unbind action reached the app");
            return;
        }
        Action::Quit => RtAction::Quit,
        Action::NewWindow => RtAction::NewWindow,
        Action::OpenConfig => RtAction::OpenConfig,
        Action::ReloadConfig => RtAction::ReloadConfig(ReloadConfig::default()),
        Action::CloseAllWindows => RtAction::CloseAllWindows,
        Action::ToggleQuickTerminal => RtAction::ToggleQuickTerminal,
        Action::ToggleVisibility => RtAction::ToggleVisibility,
        Action::CheckForUpdates => RtAction::CheckForUpdates,
        Action::ShowGtkInspector => RtAction::ShowGtkInspector,
        Action::Undo => RtAction::Undo,
        Action::Redo => RtAction::Redo,
        _ => return,
    };
    rt.perform_action(Target::App, a);
}

impl Surface {
    /// Grab the value of text at the given selection point. Note that the
    /// selection structure is used as a way to determine the area of the
    /// screen to read from, it doesn't have to match the user's current
    /// selection state (Zig: `dumpText`).
    pub fn dump_text(&self, sel: Selection) -> Result<Text, CoreError> {
        let t = self.lock_terminal();
        self.dump_text_locked(&t, sel)
    }

    /// Same as [`Surface::dump_text`] but assumes the renderer state mutex
    /// is already held.
    pub fn dump_text_locked(&self, t: &Terminal, sel: Selection) -> Result<Text, CoreError> {
        // Read out the text
        let text = t
            .screen()
            .selection_string(ghostty_terminal::screen::SelectionString { sel, trim: false });

        let scale = self.rt_surface.get_content_scale();
        let size = self.size.get();
        let baseline = self.font.borrow().metrics.cell_baseline;
        let viewport = text_viewport(
            t,
            sel,
            ViewportMetrics {
                cell_width: size.cell.width,
                cell_height: size.cell.height,
                cell_baseline: baseline,
                padding_left: size.padding.left,
                padding_top: size.padding.top,
                scale_x: scale.x,
                scale_y: scale.y,
            },
        );

        Ok(Text { text, viewport })
    }

    /// Write the screen/history/selection to a temp file and paste/copy/open
    /// the path (Zig: `writeScreenFile`).
    fn write_screen_file(
        &self,
        loc: WriteScreenLoc,
        write_screen: WriteScreen,
    ) -> Result<(), CoreError> {
        // Write the scrollback contents. This requires a lock.
        let contents = {
            let t = self.lock_terminal();
            screen_file_contents(&t, loc, write_screen.emit)?
        };
        let Some(contents) = contents else {
            return Ok(());
        };

        let path = write_screen_temp_file(&contents, loc, write_screen.emit)?;

        match write_screen.action {
            WriteScreenAction::Copy => {
                // The file name is copied as the raw bytes of the path.
                self.rt_surface.set_clipboard(
                    Clipboard::Standard,
                    &[("text/plain".to_owned(), path_bytes(&path))],
                    false,
                );
            }
            WriteScreenAction::Open => self.open_url(OpenUrl {
                kind: match write_screen.emit {
                    WriteScreenFormat::Plain | WriteScreenFormat::Vt => OpenUrlKind::Text,
                    WriteScreenFormat::Html => OpenUrlKind::Html,
                },
                url: ghostty_os::osbytes::EncodedOs::from_path(&path),
            })?,
            WriteScreenAction::Paste => {
                self.queue_io(IoMessage::write_req(&path_bytes(&path)));
            }
        }
        Ok(())
    }

    /// Update `mouse-reporting` on this surface's config (Zig mutates the
    /// derived config in place).
    fn toggle_mouse_reporting(&self) {
        let mut config = (*self.cfg().config).clone();
        config.mouse_reporting = !config.mouse_reporting;
        log::debug!("mouse reporting toggled: {}", config.mouse_reporting);
        match super::config::DerivedConfig::new(Arc::new(config)) {
            Ok(d) => *self.config.borrow_mut() = std::rc::Rc::new(d),
            Err(err) => log::warn!("failed to toggle mouse reporting err={err}"),
        }
    }

    /// Adjust the font size and mark whether it was manually adjusted.
    fn adjust_font_size(
        &self,
        f: impl FnOnce(f32, f32) -> f32,
        adjusted: bool,
    ) -> Result<(), CoreError> {
        let mut size = self.font.borrow().size;
        size.points = f(size.points, self.cfg().original_font_size);
        self.set_font_size(size)?;
        self.font.borrow_mut().adjusted = adjusted;
        Ok(())
    }

    /// Perform a binding action. A binding is a keybinding. This function
    /// must be called from the GUI thread.
    ///
    /// This function returns true if the binding action was performed. This
    /// may return false if the binding action is not supported or if the
    /// binding action would do nothing (i.e. previous tab with no tabs).
    ///
    /// NOTE: At the time of writing this comment, only previous/next tab
    /// will ever return false. We can expand this in the future if it
    /// becomes useful. We did previous/next tab so we could implement #498.
    pub fn perform_binding_action(&self, action: &Action) -> Result<bool, CoreError> {
        // Forward app-scoped actions to the app. Some app-scoped actions are
        // special-cased here because they do some special things when
        // performed from the surface.
        if action.scope() == Scope::App {
            match action {
                // The new window has this surface as its parent.
                Action::NewWindow => {
                    self.rt_app
                        .perform_action(Target::Surface(self.id), RtAction::NewWindow);
                }

                // Undo and redo both support both surface and app targeting.
                // If we are triggering on a surface then we perform the
                // action with the surface target.
                Action::Undo => return Ok(self.perform(RtAction::Undo)),
                Action::Redo => return Ok(self.perform(RtAction::Redo)),

                _ => perform_app_action(self.rt_app.as_ref(), action),
            }
            return Ok(true);
        }

        match action {
            Action::Csi(data) | Action::Esc(data) => {
                // We need to send the CSI/ESC sequence as a single write
                // request. If you split it across two then the shell can
                // interpret it as two literals.
                let mut full: Vec<u8> = if matches!(action, Action::Csi(_)) {
                    b"\x1b[".to_vec()
                } else {
                    b"\x1b".to_vec()
                };
                // The payload is a config string in the reversible OS-byte
                // encoding: the pty gets the original bytes.
                full.extend(ghostty_os::osbytes::config_str_to_bytes(data));
                // Zig formats into a fixed 128 byte buffer (`NoSpaceLeft`).
                if full.len() > 128 {
                    return Err("csi/esc binding sequence too long (max 128 bytes)".into());
                }
                self.queue_io(IoMessage::write_req(full.as_slice()));

                // CSI/ESC triggers a scroll.
                self.scroll_to_bottom(&mut self.lock_terminal());
            }

            Action::Text(data) => {
                // Decoded bytes: escapes produce UTF-8, everything else is
                // copied byte for byte (Zig `config/string.zig`).
                let text = match ghostty_config::string::parse_bytes(data) {
                    Ok(t) => t,
                    Err(err) => {
                        log::warn!("error parsing text binding text={data} err={err:?}");
                        return Ok(true);
                    }
                };
                self.queue_io(IoMessage::write_req(text.as_slice()));

                // Text triggers a scroll.
                self.scroll_to_bottom(&mut self.lock_terminal());
            }

            Action::CursorKey(ck) => {
                // We send a different sequence depending on if we're in
                // cursor keys mode. We're in "normal" mode if cursor keys
                // mode is NOT set.
                let normal = {
                    let mut t = self.lock_terminal();

                    // With the lock held, we must scroll to the bottom. We
                    // always scroll to the bottom for these inputs.
                    self.scroll_to_bottom(&mut t);

                    !t.modes.get(ghostty_vt::modes::Mode::CursorKeys)
                };

                if normal {
                    self.queue_io(IoMessage::write_req(ck.normal.as_bytes()));
                } else {
                    self.queue_io(IoMessage::write_req(ck.application.as_bytes()));
                }
            }

            Action::Reset => {
                self.lock_terminal().full_reset();
            }

            Action::StartSearch => {
                // To save resources, we don't actually start a search here,
                // we just notify the apprt. The real thread will start when
                // the first needles are set.
                return Ok(self.perform(RtAction::StartSearch(StartSearch {
                    needle: String::new(),
                })));
            }

            Action::SearchSelection => {
                let Some(selection) = self.selection_string() else {
                    return Ok(false);
                };
                return Ok(self.perform(RtAction::StartSearch(StartSearch { needle: selection })));
            }

            Action::EndSearch => {
                // We only return that this was performed if we actually
                // stopped a search, but we also send the apprt end_search so
                // that GUIs can clean up stale stuff.
                let search = self.search.borrow_mut().take();
                let performed = search.is_some();
                if let Some(s) = search {
                    s.deinit();
                }

                self.perform(RtAction::EndSearch);
                return Ok(performed);
            }

            Action::Search(text) => {
                if self.search.borrow().is_none() {
                    // If we're stopping the search and we had no prior
                    // search, then there is nothing to do.
                    if text.is_empty() {
                        return Ok(false);
                    }

                    let s = Search::start(self)?;
                    *self.search.borrow_mut() = Some(s);
                }

                // Zero-length text means stop searching.
                if text.is_empty() {
                    let s = self.search.borrow_mut().take();
                    if let Some(s) = s {
                        s.deinit();
                    }
                } else if let Some(s) = self.search.borrow().as_ref() {
                    s.handle
                        // The payload is a config string in the reversible
                        // encoding: search for the original bytes.
                        .send(SearchMessage::ChangeNeedle(
                            ghostty_os::osbytes::config_str_to_bytes(text),
                        ));
                    s.handle.wakeup();
                }
            }

            Action::NavigateSearch(nav) => {
                let search = self.search.borrow();
                let Some(s) = search.as_ref() else {
                    return Ok(false);
                };
                s.handle.send(SearchMessage::Select(match nav {
                    NavigateSearch::Next => Select::Next,
                    NavigateSearch::Previous => Select::Prev,
                }));
                s.handle.wakeup();
            }

            Action::CopyToClipboard(format) => {
                let cfg = self.cfg();
                let mut t = self.lock_terminal();

                if let Some(sel) = t.screen().selection {
                    self.copy_selection_to_clipboards(&t, sel, &[Clipboard::Standard], *format);

                    // Clear the selection if configured to do so.
                    if cfg.selection_clear_on_copy {
                        if let Err(err) = self.set_selection(&mut t, None) {
                            log::warn!("failed to clear selection after copy err={err}");
                        } else {
                            self.queue_render();
                        }
                    }

                    return Ok(true);
                }

                return Ok(false);
            }

            Action::CopyUrlToClipboard => {
                // If the mouse isn't over a link, nothing we can do.
                let Some(url) = self.link_url_at_cursor() else {
                    return Ok(false);
                };
                self.rt_surface.set_clipboard(
                    Clipboard::Standard,
                    &[("text/plain".to_owned(), url)],
                    false,
                );
                return Ok(true);
            }

            Action::CopyTitleToClipboard => {
                return Ok(self.perform(RtAction::CopyTitleToClipboard));
            }

            Action::PasteFromClipboard => {
                return Ok(
                    self.start_clipboard_request(Clipboard::Standard, ClipboardRequest::Paste)
                );
            }

            Action::PasteFromSelection => {
                return Ok(
                    self.start_clipboard_request(Clipboard::Selection, ClipboardRequest::Paste)
                );
            }

            Action::IncreaseFontSize(delta) => {
                // Max delta is somewhat arbitrary.
                let clamped = clamp_font_delta(*delta);
                log::debug!("increase font size={clamped}");

                // Max point size is somewhat arbitrary.
                self.adjust_font_size(|p, _| (p + clamped).min(255.0), true)?;
            }

            Action::DecreaseFontSize(delta) => {
                // Max delta is somewhat arbitrary.
                let clamped = clamp_font_delta(*delta);
                log::debug!("decrease font size={clamped}");

                self.adjust_font_size(|p, _| (p - clamped).max(1.0), true)?;
            }

            Action::ResetFontSize => {
                log::debug!("reset font size");

                // Reset font size also resets the manual adjustment state
                self.adjust_font_size(|_, original| original, false)?;
            }

            Action::SetFontSize(points) => {
                log::debug!("set font size={points}");

                let points = *points;
                self.adjust_font_size(|_, _| points.clamp(1.0, 255.0), true)?;
            }

            Action::PromptSurfaceTitle => {
                return Ok(self.perform(RtAction::PromptTitle(PromptTitle::Surface)));
            }

            Action::PromptTabTitle => {
                return Ok(self.perform(RtAction::PromptTitle(PromptTitle::Tab)));
            }

            Action::SetSurfaceTitle(v) => {
                return Ok(self.perform(RtAction::SetTitle(SetTitle {
                    title: ghostty_os::osbytes::config_str_to_text(v),
                })));
            }

            Action::SetTabTitle(v) => {
                return Ok(self.perform(RtAction::SetTabTitle(SetTitle {
                    title: ghostty_os::osbytes::config_str_to_text(v),
                })));
            }

            Action::ClearScreen => {
                // This is a duplicate of some of the logic in
                // termio.clearScreen but we need to do this here so we can
                // know the answer before we send the message. If the
                // currently active screen is on the alternate screen then
                // clear screen does nothing so we want to return false so
                // the keybind can be unconsumed.
                if self.lock_terminal().screens.active_key
                    == ghostty_terminal::screen_set::Key::Alternate
                {
                    return Ok(false);
                }

                self.queue_io(IoMessage::ClearScreen { history: true });
            }

            Action::ScrollToTop => {
                self.queue_io(IoMessage::ScrollViewport(ScrollViewport::Top));
            }

            Action::ScrollToBottom => {
                self.queue_io(IoMessage::ScrollViewport(ScrollViewport::Bottom));
            }

            Action::ScrollToRow(n) => {
                self.lock_terminal().screen_mut().scroll(Scroll::Row(*n));
                self.queue_render();
            }

            Action::ScrollToSelection => {
                {
                    let mut t = self.lock_terminal();
                    let screen = t.screen_mut();
                    let Some(sel) = screen.selection else {
                        return Ok(false);
                    };
                    let tl = sel.top_left(&screen.pages);
                    screen.scroll(Scroll::Pin(tl));
                }

                self.queue_render();
            }

            Action::ScrollPageUp => {
                let rows = self.size.get().grid().rows as isize;
                self.queue_io(IoMessage::ScrollViewport(ScrollViewport::Delta(-rows)));
            }

            Action::ScrollPageDown => {
                let rows = self.size.get().grid().rows as isize;
                self.queue_io(IoMessage::ScrollViewport(ScrollViewport::Delta(rows)));
            }

            Action::ScrollPageFractional(fraction) => {
                let delta = fractional_scroll_delta(*fraction, self.size.get().grid().rows);
                self.queue_io(IoMessage::ScrollViewport(ScrollViewport::Delta(delta)));
            }

            Action::ScrollPageLines(lines) => {
                self.queue_io(IoMessage::ScrollViewport(ScrollViewport::Delta(
                    isize::from(*lines),
                )));
            }

            Action::JumpToPrompt(delta) => {
                self.queue_io(IoMessage::JumpToPrompt(isize::from(*delta)));
            }

            Action::WriteScreenFile(v) => self.write_screen_file(WriteScreenLoc::Screen, *v)?,
            Action::WriteScrollbackFile(v) => {
                self.write_screen_file(WriteScreenLoc::History, *v)?;
            }
            Action::WriteSelectionFile(v) => {
                self.write_screen_file(WriteScreenLoc::Selection, *v)?;
            }

            Action::NewTab => return Ok(self.perform(RtAction::NewTab)),

            Action::CloseTab(v) => {
                return Ok(self.perform(RtAction::CloseTab(match v {
                    CloseTabMode::This => RtCloseTabMode::This,
                    CloseTabMode::Other => RtCloseTabMode::Other,
                    CloseTabMode::Right => RtCloseTabMode::Right,
                })));
            }

            Action::PreviousTab => return Ok(self.perform(RtAction::GotoTab(GotoTab::Previous))),
            Action::NextTab => return Ok(self.perform(RtAction::GotoTab(GotoTab::Next))),
            Action::LastTab => return Ok(self.perform(RtAction::GotoTab(GotoTab::Last))),
            Action::GotoTab(v) => {
                return Ok(self.perform(RtAction::GotoTab(GotoTab::Index(*v as i32))));
            }

            Action::MoveTab(position) => {
                return Ok(self.perform(RtAction::MoveTab(MoveTab { amount: *position })));
            }

            Action::NewSplit(direction) => {
                let size = self.size.get();
                return Ok(self.perform(RtAction::NewSplit(match direction {
                    SplitDirection::Right => RtSplit::Right,
                    SplitDirection::Left => RtSplit::Left,
                    SplitDirection::Down => RtSplit::Down,
                    SplitDirection::Up => RtSplit::Up,
                    SplitDirection::Auto => {
                        if size.screen.width > size.screen.height {
                            RtSplit::Right
                        } else {
                            RtSplit::Down
                        }
                    }
                })));
            }

            Action::GotoSplit(direction) => {
                return Ok(self.perform(RtAction::GotoSplit(match direction {
                    SplitFocusDirection::Previous => RtGotoSplit::Previous,
                    SplitFocusDirection::Next => RtGotoSplit::Next,
                    SplitFocusDirection::Up => RtGotoSplit::Up,
                    SplitFocusDirection::Left => RtGotoSplit::Left,
                    SplitFocusDirection::Down => RtGotoSplit::Down,
                    SplitFocusDirection::Right => RtGotoSplit::Right,
                })));
            }

            Action::GotoWindow(direction) => {
                return Ok(self.perform(RtAction::GotoWindow(match direction {
                    ghostty_input::binding::GotoWindow::Previous => RtGotoWindow::Previous,
                    ghostty_input::binding::GotoWindow::Next => RtGotoWindow::Next,
                })));
            }

            Action::ResizeSplit(value) => {
                return Ok(self.perform(RtAction::ResizeSplit(RtResizeSplit {
                    amount: value.1,
                    direction: match value.0 {
                        SplitResizeDirection::Up => ResizeSplitDirection::Up,
                        SplitResizeDirection::Down => ResizeSplitDirection::Down,
                        SplitResizeDirection::Left => ResizeSplitDirection::Left,
                        SplitResizeDirection::Right => ResizeSplitDirection::Right,
                    },
                })));
            }

            Action::EqualizeSplits => return Ok(self.perform(RtAction::EqualizeSplits)),
            Action::ToggleSplitZoom => return Ok(self.perform(RtAction::ToggleSplitZoom)),

            Action::ToggleReadonly => {
                let ro = !self.readonly.get();
                self.readonly.set(ro);
                self.perform(RtAction::Readonly(if ro {
                    Readonly::On
                } else {
                    Readonly::Off
                }));
                return Ok(true);
            }

            Action::ResetWindowSize => return Ok(self.perform(RtAction::ResetWindowSize)),
            Action::ToggleMaximize => return Ok(self.perform(RtAction::ToggleMaximize)),

            Action::ToggleFullscreen => {
                return Ok(self.perform(RtAction::ToggleFullscreen(
                    match self.cfg().macos_non_native_fullscreen {
                        NonNativeFullscreen::False => Fullscreen::Native,
                        NonNativeFullscreen::True => Fullscreen::MacosNonNative,
                        NonNativeFullscreen::VisibleMenu => Fullscreen::MacosNonNativeVisibleMenu,
                        NonNativeFullscreen::PaddedNotch => Fullscreen::MacosNonNativePaddedNotch,
                    },
                )));
            }

            Action::ToggleWindowDecorations => {
                return Ok(self.perform(RtAction::ToggleWindowDecorations));
            }
            Action::ToggleTabOverview => return Ok(self.perform(RtAction::ToggleTabOverview)),
            Action::ToggleWindowFloatOnTop => {
                return Ok(self.perform(RtAction::FloatWindow(FloatWindow::Toggle)));
            }
            Action::ToggleSecureInput => {
                return Ok(self.perform(RtAction::SecureInput(SecureInput::Toggle)));
            }

            Action::ToggleMouseReporting => self.toggle_mouse_reporting(),

            Action::ToggleCommandPalette => {
                return Ok(self.perform(RtAction::ToggleCommandPalette));
            }
            Action::ToggleBackgroundOpacity => {
                return Ok(self.perform(RtAction::ToggleBackgroundOpacity));
            }
            Action::ShowOnScreenKeyboard => {
                return Ok(self.perform(RtAction::ShowOnScreenKeyboard));
            }

            Action::SelectAll => {
                let mut t = self.lock_terminal();
                let sel = t.screen().select_all();
                if let Some(s) = sel {
                    self.set_selection(&mut t, Some(s))?;
                    self.queue_render();
                }
            }

            Action::Inspector(mode) => {
                return Ok(self.perform(RtAction::Inspector(match mode {
                    InspectorMode::Toggle => RtInspector::Toggle,
                    InspectorMode::Show => RtInspector::Show,
                    InspectorMode::Hide => RtInspector::Hide,
                })));
            }

            Action::CloseSurface => self.close(),

            Action::CloseWindow => return Ok(self.perform(RtAction::CloseWindow)),

            Action::ActivateKeyTable(name) => return self.activate_key_table(name, false),
            Action::ActivateKeyTableOnce(name) => return self.activate_key_table(name, true),

            Action::DeactivateKeyTable => return Ok(self.deactivate_key_table()),

            Action::DeactivateAllKeyTables => return Ok(self.deactivate_all_key_tables()),

            Action::EndKeySequence => {
                // End the key sequence and flush queued keys to the
                // terminal, but don't encode the key that triggered this
                // action. This will do that because leaf keys (keys with
                // bindings) aren't in the queued encoding list.
                self.end_key_sequence(
                    super::keyboard::SequenceAction::Flush,
                    super::keyboard::SequenceFree::Retain,
                );
            }

            Action::Crash(location) => match location {
                CrashThread::Main => panic!("crash binding action, crashing intentionally"),

                CrashThread::Render => {
                    self.renderer_handle.send(RendererMessage::Crash);
                    self.queue_render();
                }

                CrashThread::Io => self.queue_io(IoMessage::Crash),
            },

            Action::AdjustSelection(direction) => {
                let performed = adjust_selection(&mut self.lock_terminal(), *direction);
                if !performed {
                    return Ok(false);
                }
                self.queue_render();
            }

            // App-scoped actions were handled above.
            Action::Ignore
            | Action::Unbind
            | Action::NewWindow
            | Action::OpenConfig
            | Action::ReloadConfig
            | Action::CloseAllWindows
            | Action::ToggleQuickTerminal
            | Action::ToggleVisibility
            | Action::CheckForUpdates
            | Action::ShowGtkInspector
            | Action::Undo
            | Action::Redo
            | Action::Quit => unreachable!("app-scoped actions are handled above"),
        }

        Ok(true)
    }

    /// Notify the runtime that a key table was activated (used by the
    /// keyboard module when it implements `activate_key_table`).
    #[allow(dead_code)]
    pub(crate) fn notify_key_table(&self, kt: KeyTable) {
        self.perform(RtAction::KeyTable(kt));
    }
}

#[cfg(test)]
mod tests {
    use ghostty_terminal::terminal::Options;

    use super::*;

    fn term(cols: u16, rows: u16) -> Terminal {
        Terminal::new(Options::new(cols, rows)).unwrap()
    }

    #[test]
    fn closing_action_table() {
        assert!(closing_action(&Action::CloseSurface));
        assert!(closing_action(&Action::CloseWindow));
        assert!(closing_action(&Action::CloseTab(CloseTabMode::This)));
        assert!(!closing_action(&Action::NewTab));
        assert!(!closing_action(&Action::Reset));
    }

    #[test]
    fn scroll_and_font_math() {
        assert_eq!(fractional_scroll_delta(0.5, 24), 12);
        assert_eq!(fractional_scroll_delta(-0.5, 25), -12);
        assert_eq!(fractional_scroll_delta(1.0, 10), 10);
        assert!((clamp_font_delta(-3.0) - 0.0).abs() < f32::EPSILON);
        assert!((clamp_font_delta(300.0) - 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn screen_file_names() {
        assert_eq!(
            screen_file_name(WriteScreenLoc::Screen, WriteScreenFormat::Plain),
            "screen.txt"
        );
        assert_eq!(
            screen_file_name(WriteScreenLoc::History, WriteScreenFormat::Vt),
            "history.txt"
        );
        assert_eq!(
            screen_file_name(WriteScreenLoc::Selection, WriteScreenFormat::Html),
            "selection.html"
        );
    }

    #[test]
    fn screen_file_contents_and_write() {
        let mut t = term(20, 3);
        t.print_string("hello\r\nworld").unwrap();
        let s = screen_file_contents(&t, WriteScreenLoc::Screen, WriteScreenFormat::Plain)
            .unwrap()
            .unwrap();
        assert!(s.contains("hello"));
        assert!(s.contains("world"));

        // No selection and no history: nothing to write.
        assert!(
            screen_file_contents(&t, WriteScreenLoc::Selection, WriteScreenFormat::Plain)
                .unwrap()
                .is_none()
        );

        // Another test in this binary changes TMPDIR via temp_env, so the
        // whole temp-file section runs under temp_env's lock with TMPDIR
        // unset (std::env::temp_dir() then cannot point into that test's
        // short-lived directory).
        temp_env::with_var_unset("TMPDIR", || {
            let own_tmp = std::env::temp_dir()
                .join(format!("ghostty-screen-file-test-{}", std::process::id()));
            std::fs::create_dir_all(&own_tmp).unwrap();
            let path = temp_env::with_var("TMPDIR", Some(&own_tmp), || {
                write_screen_temp_file(&s, WriteScreenLoc::Screen, WriteScreenFormat::Plain)
                    .unwrap()
            });
            assert!(path.ends_with("screen.txt"));
            let read = std::fs::read_to_string(&path).unwrap();
            assert_eq!(read, s);
            #[cfg(unix)]
            {
                let mode = std::os::unix::fs::PermissionsExt::mode(
                    &std::fs::metadata(&path).unwrap().permissions(),
                );
                assert_eq!(mode & 0o777, 0o600);
            }
            let _ = std::fs::remove_dir_all(&own_tmp);
        });
    }

    #[cfg(unix)]
    #[test]
    fn screen_temp_file_keeps_non_utf8_path_bytes() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let td = ghostty_os::temp_dir::TempDir::init().unwrap();
        let bad = td.path().join(std::ffi::OsStr::from_bytes(b"d\xff"));
        std::fs::create_dir_all(&bad).unwrap();
        let tmpdir = std::ffi::OsString::from_vec(bad.as_os_str().as_bytes().to_vec());
        // Force the non-UTF-8 directory through TMPDIR.
        let path = temp_env::with_var("TMPDIR", Some(&tmpdir), || {
            write_screen_temp_file("x", WriteScreenLoc::Screen, WriteScreenFormat::Plain).unwrap()
        });
        assert!(path.as_os_str().as_bytes().contains(&0xff));
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
        // Copy/paste carry the exact path bytes (no U+FFFD), and the open
        // action's EncodedOs decodes back to the same path.
        assert_eq!(path_bytes(&path), path.as_os_str().as_bytes());
        let url = ghostty_os::osbytes::EncodedOs::from_path(&path);
        assert_eq!(url.to_path(), path);
    }

    #[test]
    fn adjust_selection_without_selection() {
        let mut t = term(10, 3);
        assert!(!adjust_selection(&mut t, AdjustSelection::Left));
    }

    #[test]
    fn adjust_selection_moves_end() {
        let mut t = term(10, 3);
        t.print_string("abcdef").unwrap();
        let sel = {
            let pages = &t.screen().pages;
            let a = pages
                .pin(ghostty_terminal::point::Point::Active(
                    ghostty_terminal::point::Coordinate { x: 0, y: 0 },
                ))
                .unwrap();
            let b = pages
                .pin(ghostty_terminal::point::Point::Active(
                    ghostty_terminal::point::Coordinate { x: 2, y: 0 },
                ))
                .unwrap();
            Selection::new(a, b, false)
        };
        t.screen_mut().select(Some(sel)).unwrap();
        assert!(adjust_selection(&mut t, AdjustSelection::Right));
        let s = t.screen();
        let end = s.selection.unwrap().end(&s.pages);
        assert_eq!(end.x, 3);
        assert!(s.dirty.selection);
    }

    #[test]
    fn text_viewport_placement() {
        let mut t = term(10, 3);
        t.print_string("abcdef").unwrap();
        let (a, b) = {
            let pages = &t.screen().pages;
            let p = |x| {
                pages
                    .pin(ghostty_terminal::point::Point::Active(
                        ghostty_terminal::point::Coordinate { x, y: 0 },
                    ))
                    .unwrap()
            };
            (p(1), p(3))
        };
        let m = ViewportMetrics {
            cell_width: 10,
            cell_height: 20,
            cell_baseline: 4,
            padding_left: 2,
            padding_top: 3,
            scale_x: 2.0,
            scale_y: 1.0,
        };
        let v = text_viewport(&t, Selection::new(a, b, false), m).unwrap();
        assert!((v.tl_px_x - 6.0).abs() < 1e-9); // (10 + 2) / 2
        assert!((v.tl_px_y - 19.0).abs() < 1e-9); // 0 + 20 - 4 + 3
        assert_eq!(v.offset_start, 1);
        assert_eq!(v.offset_len, 2);
    }
}
