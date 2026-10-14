//! Mouse handling for a surface (Zig `Surface.zig`: mouse reporting, button,
//! cursor position, scroll, selection by mouse, links).
//!
//! Locking rules: the renderer-state lock is only held while touching the
//! terminal. Anything that calls into the runtime (actions, clipboard
//! requests, opening URLs) and any message for the IO thread is collected in
//! a [`Deferred`] and executed after the lock is released.

use std::time::Instant;

use ghostty_apprt::{
    action::{Action as RtAction, MouseOverLink, OpenUrl, OpenUrlKind},
    runtime_api::CoreError,
    structs::{Clipboard, ClipboardRequest, CursorPos},
};
use ghostty_config::{
    types_a::LinkPreviews,
    types_b::{CopyOnSelect, MouseShiftCapture as ConfigShiftCapture, RightClickAction},
};
use ghostty_input::{
    Mods,
    MouseButton,
    MouseButtonState,
    MousePressureStage,
    ScrollMods,
    link::{Action as LinkAction, Highlight},
};
use ghostty_renderer::{
    size::{Coordinate as SizeCoordinate, CoordinateTag, Size, SurfaceCoord},
    state::State as RenderStateData,
};
use ghostty_terminal::{
    page_list::{Direction, PageList, Pin, TrackedPin},
    point::{Coordinate as PointCoordinate, Point},
    screen::{Screen, SelectLine, SelectionString, SemanticClick},
    screen_set::Key as ScreenKey,
    selection::Selection,
    terminal::{
        MouseEvents,
        MouseFormat,
        MouseShiftCapture as TerminalShiftCapture,
        ScrollViewport,
        Terminal,
    },
};
use ghostty_termio::message::Message as IoMessage;
use ghostty_vt::modes::Mode;

use super::{MOUSE_BUTTON_COUNT, Surface};
use crate::surface_mouse;

/// Bound synchronous link lookup while the terminal mutex is held. Oversized
/// logical lines are skipped, rather than truncating a URL into another target.
const MAX_LINK_CELLS: usize = 64 * 1024;

/// Mouse state for the surface (Zig: `Surface.Mouse`).
pub struct Mouse {
    /// The last tracked mouse button state by button.
    pub click_state: [MouseButtonState; MOUSE_BUTTON_COUNT],

    /// The last mods state when the last mouse button (whatever it was) was
    /// pressed or release.
    pub mods: Mods,

    /// The point at which the left mouse click happened. This is in screen
    /// coordinates so that scrolling preserves the location. Tracked in the
    /// page list of `left_click_screen`.
    pub left_click_pin: Option<TrackedPin>,
    pub left_click_screen: ScreenKey,

    /// The starting xpos/ypos of the left click. Note that if scrolling
    /// occurs, these will point to different "cells", but the xpos/ypos will
    /// stay stable during scrolling relative to the surface.
    pub left_click_xpos: f64,
    pub left_click_ypos: f64,

    /// The count of clicks to count double and triple clicks and so on. The
    /// left click time was the last time the left click was done. This is
    /// always set on the first left click.
    pub left_click_count: u8,
    pub left_click_time: Option<Instant>,

    /// The last x/y sent for mouse reports.
    pub event_point: Option<PointCoordinate>,

    /// The pressure stage for the mouse. This should always be none if the
    /// mouse is not pressed.
    pub pressure_stage: MousePressureStage,

    /// Pending scroll amounts for high-precision scrolls
    pub pending_scroll_x: f64,
    pub pending_scroll_y: f64,

    /// True if the mouse is hidden
    pub hidden: bool,

    /// True if the mouse position is currently over a link.
    pub over_link: bool,

    /// The last x/y in the cursor position for links. We use this to only
    /// process link hover events when the mouse actually moves cells.
    pub link_point: Option<PointCoordinate>,
}

impl Default for Mouse {
    fn default() -> Self {
        Mouse {
            click_state: [MouseButtonState::Release; MOUSE_BUTTON_COUNT],
            mods: Mods::default(),
            left_click_pin: None,
            left_click_screen: ScreenKey::Primary,
            left_click_xpos: 0.0,
            left_click_ypos: 0.0,
            left_click_count: 0,
            left_click_time: None,
            event_point: None,
            pressure_stage: MousePressureStage::None,
            pending_scroll_x: 0.0,
            pending_scroll_y: 0.0,
            hidden: false,
            over_link: false,
            link_point: None,
        }
    }
}

/// The type of action to report for a mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MouseReportAction {
    Press,
    Release,
    Motion,
}

/// The amount to scroll. This structure is always normalized so that
/// negative is down, left and positive is up, right. Note that INTERNALLY,
/// vertical scroll on our terminal uses positive for down.
#[derive(Debug, Clone, Copy, Default)]
struct ScrollAmount {
    delta: isize,
}

impl ScrollAmount {
    fn up_right(self) -> bool {
        self.delta >= 0
    }

    fn magnitude(self) -> usize {
        self.delta.unsigned_abs()
    }
}

/// A link found under the mouse.
#[derive(Debug, Clone, Copy)]
struct Link {
    action: LinkAction,
    selection: Selection,
}

/// Work that must happen after the renderer lock is released.
#[derive(Default)]
struct Deferred {
    actions: Vec<RtAction>,
    io: Vec<IoMessage>,
    render: bool,
}

impl Deferred {
    fn run(self, s: &Surface) {
        for m in self.io {
            s.queue_io(m);
        }
        for a in self.actions {
            s.perform(a);
        }
        if self.render {
            s.queue_render();
        }
    }
}

// ----------------------------------------------------------------------
// Pure selection logic

/// Calculates the appropriate selection given pins and pixel x positions for
/// the click point and the drag point, as well as mouse mods and screen size.
#[must_use]
pub fn mouse_selection(
    list: &PageList,
    click_pin: Pin,
    drag_pin: Pin,
    click_x: u32,
    drag_x: u32,
    mods: Mods,
    size: Size,
) -> Option<Selection> {
    // Explanation:
    //
    // # Normal selections
    //
    // ## Left-to-right selections
    // - The clicked cell is included if it was clicked to the left of its
    //   threshold point and the drag location is right of the threshold
    //   point.
    // - The cell under the cursor (the "drag cell") is included if the drag
    //   location is right of its threshold point.
    //
    // ## Right-to-left selections
    // - The clicked cell is included if it was clicked to the right of its
    //   threshold point and the drag location is left of the threshold
    //   point.
    // - The cell under the cursor (the "drag cell") is included if the drag
    //   location is left of its threshold point.
    //
    // # Rectangular selections
    //
    // Rectangular selections are handled similarly, except that entire
    // columns are considered rather than individual cells.

    // We only include cells in the selection if the threshold point lies
    // between the start and end points of the selection. A threshold of 60%
    // of the cell width was chosen empirically because it felt good.
    let threshold_point = (f64::from(size.cell.width) * 0.6).round() as u32;

    // We use this to clamp the pixel positions below.
    let max_x = (u32::from(size.grid().columns) * size.cell.width).saturating_sub(1);

    // We need to know how far across in the cell the drag pos is, so we
    // subtract the padding and then take it modulo the cell width.
    let drag_x_frac = max_x.min(drag_x.saturating_sub(size.padding.left)) % size.cell.width;

    // We figure out the fractional part of the click x position similarly.
    let click_x_frac = max_x.min(click_x.saturating_sub(size.padding.left)) % size.cell.width;

    // Whether or not this is a rectangular selection.
    let rectangle_selection = surface_mouse::is_rectangle_select_state(mods);

    // Whether the click pin and drag pin are equal.
    let same_pin = drag_pin.eql(click_pin);

    // Whether or not the end point of our selection is before the start
    // point.
    let end_before_start = if same_pin {
        drag_x_frac < click_x_frac
    } else if rectangle_selection {
        // Special handling for rectangular selections, we only use x
        // position.
        match drag_pin.x.cmp(&click_pin.x) {
            std::cmp::Ordering::Equal => drag_x_frac < click_x_frac,
            std::cmp::Ordering::Less => true,
            std::cmp::Ordering::Greater => false,
        }
    } else {
        drag_pin.before(list, click_pin)
    };

    // Whether or not the the click pin cell should be included in the
    // selection.
    let include_click_cell = if end_before_start {
        click_x_frac >= threshold_point
    } else {
        click_x_frac < threshold_point
    };

    // Whether or not the the drag pin cell should be included in the
    // selection.
    let include_drag_cell = if end_before_start {
        drag_x_frac < threshold_point
    } else {
        drag_x_frac >= threshold_point
    };

    // If the click cell should be included in the selection then it's the
    // start, otherwise we get the previous or next cell to it depending on
    // the type and direction of the selection.
    let start_pin = if include_click_cell {
        click_pin
    } else if end_before_start {
        if rectangle_selection {
            click_pin.left_clamp(1)
        } else {
            click_pin.left_wrap(list, 1).unwrap_or(click_pin)
        }
    } else if rectangle_selection {
        click_pin.right_clamp(list, 1)
    } else {
        click_pin.right_wrap(list, 1).unwrap_or(click_pin)
    };

    // Likewise for the end pin with the drag cell.
    let end_pin = if include_drag_cell {
        drag_pin
    } else if end_before_start {
        if rectangle_selection {
            drag_pin.right_clamp(list, 1)
        } else {
            drag_pin.right_wrap(list, 1).unwrap_or(drag_pin)
        }
    } else if rectangle_selection {
        drag_pin.left_clamp(1)
    } else {
        drag_pin.left_wrap(list, 1).unwrap_or(drag_pin)
    };

    // If the click cell is the same as the drag cell and the click cell
    // shouldn't be included, or if the cells are adjacent such that the
    // start or end pin becomes the other cell, and that cell should not be
    // included, then we have no selection, so we set it to null.
    //
    // If in rectangular selection mode, we compare columns as well.
    if (!include_click_cell && same_pin)
        || (!include_click_cell && rectangle_selection && click_pin.x == drag_pin.x)
        || (!include_click_cell && end_pin.eql(click_pin))
        || (!include_click_cell && rectangle_selection && end_pin.x == click_pin.x)
        || (!include_drag_cell && start_pin.eql(drag_pin))
        || (!include_drag_cell && rectangle_selection && start_pin.x == drag_pin.x)
    {
        return None;
    }

    // TODO: Clamp selection to the screen area, don't let it extend past the
    // last written row.

    Some(Selection::new(start_pin, end_pin, rectangle_selection))
}

/// Double-click dragging moves the selection one "word" at a time. Returns
/// `None` when the selection should be cleared.
fn drag_left_click_double_selection(
    screen: &Screen,
    click_pin: Pin,
    drag_pin: Pin,
    word_chars: &[u32],
) -> Option<Selection> {
    let list = &screen.pages;

    // Get the word closest to our starting click.
    let word_start = screen.select_word_between(click_pin, drag_pin, word_chars)?;

    // Get the word closest to our current point.
    let word_current = screen.select_word_between(drag_pin, click_pin, word_chars)?;

    // If our current mouse position is before the starting position, then
    // the selection start is the word nearest our current position.
    if drag_pin.before(list, click_pin) {
        Some(Selection::new(
            word_current.start(list),
            word_start.end(list),
            false,
        ))
    } else {
        Some(Selection::new(
            word_start.start(list),
            word_current.end(list),
            false,
        ))
    }
}

/// Triple-click dragging moves the selection one "line" at a time. `None`
/// means do nothing.
fn drag_left_click_triple_selection(
    screen: &Screen,
    click_pin: Pin,
    drag_pin: Pin,
) -> Option<Selection> {
    let list = &screen.pages;

    // Get the line selection under our current drag point. If there isn't a
    // line, do nothing.
    let line = screen.select_line(SelectLine::new(drag_pin))?;

    // Get the selection under our click point. We first try to trim
    // whitespace if we've selected a word. But if no word exists then we
    // select the blank line.
    let sel = screen.select_line(SelectLine::new(click_pin)).or_else(|| {
        screen.select_line(SelectLine {
            whitespace: None,
            ..SelectLine::new(click_pin)
        })
    })?;

    if drag_pin.before(list, click_pin) {
        Some(Selection::new(
            line.start(list),
            sel.end(list),
            sel.rectangle,
        ))
    } else {
        Some(Selection::new(
            sel.start(list),
            line.end(list),
            sel.rectangle,
        ))
    }
}

/// Resolve a relative file path to an absolute path using `pwd` (Zig
/// `resolvePathForOpening`). Returns `None` for absolute paths (use as-is),
/// when there's no pwd, or when the resolved path doesn't exist.
///
/// `pwd` is the raw OSC 7 byte string; `path` is plain terminal text.
fn resolve_path_for_opening(
    pwd: Option<&[u8]>,
    path: &str,
) -> Option<ghostty_os::osbytes::EncodedOs> {
    use std::path::{Component, Path, PathBuf};

    if ghostty_os::osbytes::zig_is_absolute(path.as_bytes()) {
        return None;
    }

    let pwd = ghostty_os::osbytes::os_from_bytes(pwd?.to_vec());
    let joined = Path::new(&pwd).join(path);

    // Lexically normalize (std.fs.path.resolve).
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }

    if !out.exists() {
        return None;
    }
    Some(ghostty_os::osbytes::EncodedOs::from_path(&out))
}

/// What to hand to the opener for a matched `open` link: a relative path
/// that resolves against the terminal's pwd becomes that (OS) path, anything
/// else (a URL, an absolute path) is the matched *plain text*, wrapped so it
/// is never mistaken for an escaped OS string.
fn open_link_target(pwd: Option<&[u8]>, text: &str) -> ghostty_os::osbytes::EncodedOs {
    resolve_path_for_opening(pwd, text)
        .unwrap_or_else(|| ghostty_os::osbytes::EncodedOs::from_plain_text(text))
}

impl Surface {
    // ------------------------------------------------------------------
    // Helpers

    /// Returns true if mouse reporting is enabled both in the config and
    /// the terminal state.
    fn is_mouse_reporting(&self, t: &Terminal) -> bool {
        self.cfg().mouse_reporting && t.flags.mouse_event != MouseEvents::None
    }

    /// Returns true if the shift modifier is allowed to be captured by
    /// modifier events. It is up to the caller to still verify it is a
    /// situation in which shift capture makes sense (i.e. left button, mouse
    /// click, etc.)
    pub(crate) fn mouse_shift_capture(&self, t: &Terminal) -> bool {
        let cfg = self.cfg();

        // Handle our never/always case where we don't need the terminal.
        match cfg.mouse_shift_capture {
            ConfigShiftCapture::Never => return false,
            ConfigShiftCapture::Always => return true,
            ConfigShiftCapture::False | ConfigShiftCapture::True => {}
        }

        // If the terminal explicitly requests it then we always allow it
        // since we processed never/always at this point.
        match t.flags.mouse_shift_capture {
            TerminalShiftCapture::False => return false,
            TerminalShiftCapture::True => return true,
            TerminalShiftCapture::Null => {}
        }

        // Otherwise, go with the user's preference
        matches!(cfg.mouse_shift_capture, ConfigShiftCapture::True)
    }

    /// Returns true if the mouse is currently captured by the terminal
    /// (i.e. reporting events).
    pub fn mouse_captured(&self) -> bool {
        self.lock_terminal().flags.mouse_event != MouseEvents::None
    }

    /// This returns the mouse mods to consider for link highlighting or
    /// other purposes taking into account when shift is pressed for
    /// releasing the mouse from capture. Takes the renderer lock.
    #[allow(dead_code)]
    pub(crate) fn mouse_mods_with_capture(&self, mods: Mods) -> Mods {
        let t = self.lock_terminal();
        self.mouse_mods_with_capture_locked(&t, mods)
    }

    fn mouse_mods_with_capture_locked(&self, t: &Terminal, mods: Mods) -> Mods {
        // In any of these scenarios, whatever mods are set (even shift) are
        // preserved.
        if t.flags.mouse_event == MouseEvents::None {
            return mods;
        }
        if !mods.shift {
            return mods;
        }
        if self.mouse_shift_capture(t) {
            return mods;
        }

        // We have mouse capture, shift set, and we're not allowed to
        // capture shift, so we can clear shift.
        let mut final_ = mods;
        final_.shift = false;
        final_
    }

    /// Call this when modifiers change. This is safe to call even if
    /// modifiers match the previous state.
    ///
    /// The renderer state mutex MUST NOT be held.
    pub(crate) fn mods_changed(&self, mods: Mods) {
        // The only place we keep track of mods currently is on the mouse.
        let new_mods = {
            let mut m = self.mouse.borrow_mut();
            if m.mods.equal(mods) {
                return;
            }

            // The mouse mods only contain binding modifiers since we don't
            // want caps/num lock or sided modifiers to affect the mouse.
            m.mods = mods.binding();
            m.mods
        };

        // We also need to update the renderer so it knows if it should
        // highlight links. Additionally, mark the screen as dirty so that
        // the highlight state of all links is properly updated.
        {
            let mut st = self.lock_state();
            let capture = self.mouse_mods_with_capture_locked(&st.terminal, new_mods);
            st.mouse.mods = capture;

            // We use the clear screen dirty flag to force a rebuild of all
            // rows because changing mouse mods can affect the highlight
            // state of a link. If there is no link this seems very wasteful
            // but its really only one frame so it's not so bad.
            st.terminal.flags.dirty.clear = true;
        }

        self.queue_render();
    }

    // ------------------------------------------------------------------
    // Mouse reporting

    /// Build the bytes for a mouse report (Zig `mouseReport`). Returns
    /// `None` if nothing should be sent. The caller holds the terminal
    /// lock and `m` is the (borrowed) mouse state.
    fn mouse_report(
        &self,
        t: &Terminal,
        m: &mut Mouse,
        button: Option<MouseButton>,
        action: MouseReportAction,
        mods: Mods,
        pos: CursorPos,
    ) -> Option<Vec<u8>> {
        // Mouse reporting must be enabled by both config and terminal state
        debug_assert!(self.cfg().mouse_reporting);
        debug_assert_ne!(t.flags.mouse_event, MouseEvents::None);

        // Depending on the event, we may do nothing at all.
        match t.flags.mouse_event {
            MouseEvents::None => return None,

            // X10 only reports clicks with mouse button 1, 2, 3. We verify
            // the button later.
            MouseEvents::X10 => {
                if action != MouseReportAction::Press
                    || !matches!(
                        button,
                        Some(MouseButton::Left | MouseButton::Right | MouseButton::Middle)
                    )
                {
                    return None;
                }
            }

            // Doesn't report motion
            MouseEvents::Normal => {
                if action == MouseReportAction::Motion {
                    return None;
                }
            }

            // Button must be pressed
            MouseEvents::Button => {
                button?;
            }

            // Everything
            MouseEvents::Any => {}
        }

        let size = self.size.get();

        // Handle scenarios where the mouse position is outside the
        // viewport. We always report release events no matter where they
        // happen.
        if action != MouseReportAction::Release {
            let max_x = size.screen.width as f32;
            let max_y = size.screen.height as f32;
            let pos_out_viewport = pos.x < 0.0 || pos.y < 0.0 || pos.x > max_x || pos.y > max_y;
            if pos_out_viewport {
                // If we don't have a motion-tracking event mode, do nothing.
                if !t.flags.mouse_event.motion() {
                    return None;
                }

                // If any button is pressed, we still do the report.
                // Otherwise, we do not do the report.
                if m.click_state
                    .iter()
                    .all(|s| *s == MouseButtonState::Release)
                {
                    return None;
                }
            }
        }

        // This format reports X/Y
        let viewport_point = self.pos_to_viewport(f64::from(pos.x), f64::from(pos.y));

        // Record our new point. We only want to send a mouse event if the
        // cell changed, unless we're tracking raw pixels.
        if action == MouseReportAction::Motion
            && t.flags.mouse_format != MouseFormat::SgrPixels
            && let Some(last) = m.event_point
            && last == viewport_point
        {
            return None;
        }
        m.event_point = Some(viewport_point);

        let format = t.flags.mouse_format;

        // Get the code we'll actually write
        let button_code: u8 = {
            // Determine our initial button value
            let mut acc: u8 = match button {
                // Null button means motion without a button pressed
                None => 3,
                // Release is 3. It is NOT 3 in SGR mode because SGR can tell
                // the application what button was released.
                Some(_)
                    if action == MouseReportAction::Release
                        && format != MouseFormat::Sgr
                        && format != MouseFormat::SgrPixels =>
                {
                    3
                }
                Some(b) => match b {
                    MouseButton::Left => 0,
                    MouseButton::Middle => 1,
                    MouseButton::Right => 2,
                    MouseButton::Four => 64,
                    MouseButton::Five => 65,
                    MouseButton::Six => 66,
                    MouseButton::Seven => 67,
                    MouseButton::Eight => 128,
                    MouseButton::Nine => 129,
                    _ => return None, // unsupported
                },
            };

            // X10 doesn't have modifiers
            if t.flags.mouse_event != MouseEvents::X10 {
                if mods.shift {
                    acc += 4;
                }
                if mods.alt {
                    acc += 8;
                }
                if mods.ctrl {
                    acc += 16;
                }
            }

            // Motion adds another bit
            if action == MouseReportAction::Motion {
                acc += 32;
            }

            acc
        };

        let x = u32::from(viewport_point.x);
        let y = viewport_point.y;

        match format {
            MouseFormat::X10 => {
                if x > 222 || y > 222 {
                    log::info!("X10 mouse format can only encode X/Y up to 223");
                    return None;
                }

                // + 1 below is because our x/y is 0-indexed and the protocol
                // wants 1
                Some(vec![
                    0x1b,
                    b'[',
                    b'M',
                    32 + button_code,
                    32 + x as u8 + 1,
                    32 + y as u8 + 1,
                ])
            }

            MouseFormat::Utf8 => {
                let mut data = vec![0x1b, b'[', b'M', 32 + button_code];

                // UTF-8 encode the x/y
                let mut buf = [0u8; 4];
                for v in [32 + x + 1, 32 + y + 1] {
                    let c = char::from_u32(v)?;
                    data.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                Some(data)
            }

            MouseFormat::Sgr => {
                // Final character to send in the CSI
                let final_ = if action == MouseReportAction::Release {
                    'm'
                } else {
                    'M'
                };
                Some(format!("\x1b[<{button_code};{};{}{final_}", x + 1, y + 1).into_bytes())
            }

            MouseFormat::Urxvt => Some(
                format!("\x1b[{};{};{}M", 32 + u32::from(button_code), x + 1, y + 1).into_bytes(),
            ),

            MouseFormat::SgrPixels => {
                // Final character to send in the CSI
                let final_ = if action == MouseReportAction::Release {
                    'm'
                } else {
                    'M'
                };

                // The position has to be adjusted to the terminal space.
                let coord = SizeCoordinate::Surface(SurfaceCoord {
                    x: f64::from(pos.x),
                    y: f64::from(pos.y),
                })
                .convert(CoordinateTag::Terminal, size);
                let (cx, cy) = match coord {
                    SizeCoordinate::Terminal(c) => (c.x, c.y),
                    _ => (0.0, 0.0),
                };

                Some(
                    format!(
                        "\x1b[<{button_code};{};{}{final_}",
                        cx.round() as i32,
                        cy.round() as i32
                    )
                    .into_bytes(),
                )
            }
        }
    }

    // ------------------------------------------------------------------
    // Scrolling

    /// Mouse scroll event. Negative is down, left. Positive is up, right.
    ///
    /// "Natural scrolling" is a macOS term for inverting the scroll
    /// direction. This should be handled by the apprt implementation. At
    /// this layer, negative is always down, left.
    pub fn scroll_callback(
        &self,
        xoff: f64,
        yoff: f64,
        scroll_mods: ScrollMods,
    ) -> Result<(), CoreError> {
        // Always show the mouse again if it is hidden
        let hidden = self.mouse.borrow().hidden;
        if hidden {
            self.show_mouse();
        }

        let cfg = self.cfg();
        let size = self.size.get();

        let (y, x) = {
            let mut m = self.mouse.borrow_mut();

            let y = if yoff == 0.0 {
                ScrollAmount::default()
            } else {
                // We use cell_size to determine if we have accumulated
                // enough to trigger a scroll
                let cell_size = f64::from(size.cell.height);

                // If we have precision scroll, yoff is the number of pixels
                // to scroll. In non-precision scroll, yoff is the number of
                // wheel ticks. Some mice are capable of reporting fractional
                // wheel ticks, which don't necessarily get reported as
                // precision scrolls. We normalize all scroll events to
                // pixels by multiplying the wheel tick value and the cell
                // size. This means that a wheel tick of 1 results in single
                // scroll event.
                let yoff_adjusted = if scroll_mods.precision {
                    yoff * cfg.mouse_scroll_multiplier.precision
                } else {
                    // Round out the yoff to an absolute minimum of 1. macos
                    // tries to simulate precision scrolling with non
                    // precision events by ramping up the magnitude of the
                    // offsets as it detects faster scrolling. Single click
                    // (very slow) scrolls are reported with a magnitude of
                    // 0.1 which would normally require a few clicks before
                    // we register an actual scroll event (depending on cell
                    // height and the mouse_scroll_multiplier setting).
                    let yoff_max = if yoff > 0.0 {
                        yoff.max(1.0)
                    } else {
                        yoff.min(-1.0)
                    };

                    yoff_max * cell_size * cfg.mouse_scroll_multiplier.discrete
                };

                // Add our previously saved pending amount to the offset to
                // get the new offset value. The signs of the pending and
                // yoff should match so that we move further away from zero,
                // but we don't assert this because in theory a user could
                // scroll in the opposite direction and undo a pending
                // scroll.
                let poff = m.pending_scroll_y + yoff_adjusted;

                // If the new offset is less than a single unit of scroll,
                // we save the new pending value and do not scroll yet.
                if poff.abs() < cell_size {
                    m.pending_scroll_y = poff;
                    ScrollAmount::default()
                } else {
                    // We scroll by the number of rows in the offset and save
                    // the remainder
                    let amount = poff / cell_size;
                    m.pending_scroll_y = poff - (amount * cell_size);

                    // Round towards zero.
                    ScrollAmount {
                        delta: amount.trunc() as isize,
                    }
                }
            };

            // For detailed comments see the y calculation above.
            let x = if xoff == 0.0 {
                ScrollAmount::default()
            } else if !scroll_mods.precision {
                ScrollAmount {
                    delta: xoff.round() as isize,
                }
            } else {
                let poff = m.pending_scroll_x + xoff;
                let cell_size = f64::from(size.cell.width);
                if poff.abs() < cell_size {
                    m.pending_scroll_x = poff;
                    ScrollAmount::default()
                } else {
                    let amount = poff / cell_size;
                    m.pending_scroll_x = poff - (amount * cell_size);
                    ScrollAmount {
                        delta: amount.trunc() as isize,
                    }
                }
            };

            (y, x)
        };

        // The cursor position is only needed when reporting; fetch it before
        // taking the lock.
        let pos = self.rt_surface.get_cursor_pos();

        let mut writes: Vec<IoMessage> = Vec::new();
        {
            let mut t = self.lock_terminal();

            // If we have an active mouse reporting mode, clear the
            // selection. The selection can occur if the user uses the shift
            // mod key to override mouse grabbing from the window.
            if self.is_mouse_reporting(&t) {
                let _ = self.set_selection(&mut t, None);
            }

            // If we're in alternate screen with alternate scroll enabled,
            // then we convert to cursor keys. This only happens if we're:
            // (1) alt screen (2) no explicit mouse reporting and (3)
            // alt scroll mode enabled.
            if t.screens.active_key == ScreenKey::Alternate
                && t.flags.mouse_event == MouseEvents::None
                && t.modes.get(Mode::MouseAlternateScroll)
            {
                if y.delta != 0 {
                    // When we send mouse events as cursor keys we always
                    // clear the selection.
                    let _ = self.set_selection(&mut t, None);

                    let seq: &'static [u8] = if t.modes.get(Mode::CursorKeys) {
                        // cursor key: application mode
                        if y.up_right() { b"\x1bOA" } else { b"\x1bOB" }
                    } else {
                        // cursor key: normal mode
                        if y.up_right() { b"\x1b[A" } else { b"\x1b[B" }
                    };
                    for _ in 0..y.magnitude() {
                        writes.push(IoMessage::write_stable(seq));
                    }
                }
                drop(t);
                for w in writes {
                    self.queue_io(w);
                }
                return Ok(());
            }

            // We have mouse events, are not in an alternate scroll buffer,
            // or have alternate scroll disabled. In this case, we just run
            // the normal logic.

            // If we're scrolling up or down, then send a mouse event.
            if self.is_mouse_reporting(&t) {
                let mods = self.mouse.borrow().mods;
                for _ in 0..y.magnitude() {
                    let button = if y.up_right() {
                        MouseButton::Four
                    } else {
                        MouseButton::Five
                    };
                    let mut m = self.mouse.borrow_mut();
                    if let Some(b) = self.mouse_report(
                        &t,
                        &mut m,
                        Some(button),
                        MouseReportAction::Press,
                        mods,
                        pos,
                    ) {
                        writes.push(IoMessage::write_req(&b));
                    }
                }

                for _ in 0..x.magnitude() {
                    let button = if x.up_right() {
                        MouseButton::Six
                    } else {
                        MouseButton::Seven
                    };
                    let mut m = self.mouse.borrow_mut();
                    if let Some(b) = self.mouse_report(
                        &t,
                        &mut m,
                        Some(button),
                        MouseReportAction::Press,
                        mods,
                        pos,
                    ) {
                        writes.push(IoMessage::write_req(&b));
                    }
                }

                // If mouse reporting is on, we do not want to scroll the
                // viewport.
                drop(t);
                for w in writes {
                    self.queue_io(w);
                }
                return Ok(());
            }

            if y.delta != 0 {
                // Modify our viewport, this requires a lock since it affects
                // rendering. We have to switch signs here because our delta
                // is negative down but our viewport is positive down.
                t.scroll_viewport(ScrollViewport::Delta(-y.delta));
            }
        }

        self.queue_render();
        Ok(())
    }

    // ------------------------------------------------------------------
    // Button

    /// Called for mouse button press/release events. This will return true
    /// if the mouse event was consumed in some way (i.e. the program is
    /// capturing mouse events). If the event was not consumed, then false is
    /// returned.
    pub fn mouse_button_callback(
        &self,
        action: MouseButtonState,
        button: MouseButton,
        mods: Mods,
    ) -> Result<bool, CoreError> {
        let cfg = self.cfg();

        // If we have an inspector, we always queue a render
        let _inspector_render = self
            .inspector
            .borrow()
            .is_some()
            .then(|| RenderOnDrop(self));

        // Always record our latest mouse state
        let hidden = {
            let mut m = self.mouse.borrow_mut();
            let idx = button as usize;
            if idx < MOUSE_BUTTON_COUNT {
                m.click_state[idx] = action;
            }
            m.hidden
        };

        // Always show the mouse again if it is hidden
        if hidden {
            self.show_mouse();
        }

        // Update our modifiers if they changed
        self.mods_changed(mods);

        // This is set to true if the terminal is allowed to capture the
        // shift modifier. Note we can do this more efficiently probably with
        // less locking/unlocking but clicking isn't that frequent enough to
        // be a bottleneck.
        let shift_capture = {
            let t = self.lock_terminal();
            self.mouse_shift_capture(&t)
        };

        // Shift-click continues the previous mouse state if we have a
        // selection. cursorPosCallback will also do a mouse report so we
        // don't need to do any of the logic below.
        if button == MouseButton::Left && action == MouseButtonState::Press {
            let click_count = self.mouse.borrow().left_click_count;
            if mods.shift && click_count > 0 && !shift_capture && self.has_selection() {
                // If we are within the interval that the click would
                // register an increment then we do not extend the selection.
                let within_interval = self
                    .mouse
                    .borrow()
                    .left_click_time
                    .is_some_and(|t| t.elapsed().as_nanos() as u64 <= cfg.mouse_interval);

                if !within_interval {
                    let pos = self.rt_surface.get_cursor_pos();
                    self.cursor_pos_callback(pos, None)?;
                    return Ok(true);
                }
            }
        }

        if button == MouseButton::Left && action == MouseButtonState::Release {
            // Stop selection scrolling when releasing the left mouse button
            // but only when selection scrolling is active.
            if self.selection_scroll_active.get() {
                self.queue_io(IoMessage::SelectionScroll(false));
            }

            // The selection clipboard is only updated for left-click drag
            // when the left button is released. This is to avoid the
            // clipboard being updated on every mouse move which would be
            // noisy.
            if cfg.copy_on_select != CopyOnSelect::False {
                let mut t = self.lock_terminal();
                if let Some(prev) = t.screen().selection {
                    let list = &t.screen().pages;
                    let sel = Selection::new(prev.start(list), prev.end(list), prev.rectangle);
                    self.copy_on_select(&mut t, sel);
                }
            }

            // Handle link clicking. We want to do this before we do mouse
            // reporting or any other mouse handling because a successfully
            // clicked link will swallow the event.
            let over_link = self.mouse.borrow().over_link;
            if over_link {
                let pos = self.rt_surface.get_cursor_pos();
                match self.process_links(pos) {
                    Ok(true) => return Ok(true),
                    Ok(false) => {}
                    Err(err) => log::warn!("error processing links err={err}"),
                }
            }

            // Handle prompt clicking. If we released our mouse on a prompt
            // and we support some kind of click events, then we need to move
            // to it.
            if self.maybe_prompt_click() {
                return Ok(true);
            }
        }

        // Report mouse events if enabled
        {
            let mut t = self.lock_terminal();
            if self.is_mouse_reporting(&t) {
                // If we have shift-pressed and we aren't allowed to capture
                // it, then we do not do a mouse report.
                if !(mods.shift && !shift_capture) {
                    // In any other mouse button scenario without shift
                    // pressed we clear the selection since the underlying
                    // application can handle that in any way (i.e.
                    // "scrolling").
                    let _ = self.set_selection(&mut t, None);

                    // We also set the left click count to 0 so that if mouse
                    // reporting is disabled in the middle of press (before
                    // release) we don't suddenly start selecting text.
                    let pos = self.rt_surface.get_cursor_pos();
                    let report_action = match action {
                        MouseButtonState::Press => MouseReportAction::Press,
                        MouseButtonState::Release => MouseReportAction::Release,
                    };

                    let bytes = {
                        let mut m = self.mouse.borrow_mut();
                        m.left_click_count = 0;
                        let mods = m.mods;
                        self.mouse_report(&t, &mut m, Some(button), report_action, mods, pos)
                    };
                    drop(t);
                    if let Some(b) = bytes {
                        self.queue_io(IoMessage::write_req(&b));
                    }

                    // If we're doing mouse reporting, we do not support any
                    // other selection or highlighting.
                    return Ok(true);
                }
            }
        }

        // For left button clicks we always record some information for
        // selection/highlighting purposes.
        if button == MouseButton::Left && action == MouseButtonState::Press {
            self.left_press(mods, &cfg);
        }

        // Middle-click pastes from our selection clipboard
        if button == MouseButton::Middle && action == MouseButtonState::Press {
            let clipboard = if self.rt_surface.supports_clipboard(Clipboard::Selection) {
                Clipboard::Selection
            } else {
                Clipboard::Standard
            };
            self.start_clipboard_request(clipboard, ClipboardRequest::Paste);
        }

        // Right-click down selects word for context menus. If the apprt
        // doesn't implement context menus this can be a bit weird but they
        // are supported by our two main apprts so we always do this. If we
        // want to be careful in the future we can add a function to apprts
        // that let's us know.
        if button == MouseButton::Right
            && action == MouseButtonState::Press
            && let Some(consumed) = self.right_press(&cfg)
        {
            return Ok(consumed);
        }

        Ok(false)
    }

    /// Copy the selection per `copy-on-select` (the clipboard half of Zig
    /// `setSelection`) without changing the selection.
    fn copy_on_select(&self, t: &mut Terminal, sel: Selection) {
        let cfg = self.cfg();
        let format = ghostty_input::binding::CopyToClipboard::Mixed;
        match cfg.copy_on_select {
            CopyOnSelect::False => {}
            CopyOnSelect::Clipboard => self.copy_selection_to_clipboards(
                t,
                sel,
                &[Clipboard::Standard, Clipboard::Selection],
                format,
            ),
            CopyOnSelect::True => {
                let clipboard = if self.rt_surface.supports_clipboard(Clipboard::Selection) {
                    Clipboard::Selection
                } else {
                    Clipboard::Standard
                };
                self.copy_selection_to_clipboards(t, sel, &[clipboard], format);
            }
        }
    }

    /// Left button press: record click state and start/extend selection by
    /// click count.
    fn left_press(&self, mods: Mods, cfg: &super::config::DerivedConfig) {
        let pos = self.rt_surface.get_cursor_pos();
        let (xpos, ypos) = (f64::from(pos.x), f64::from(pos.y));
        let size = self.size.get();
        let vp = self.pos_to_viewport(xpos, ypos);

        let mut t = self.lock_terminal();
        let active_key = t.screens.active_key;

        let Some(pin) = t.screen().pages.pin(Point::Viewport(vp)) else {
            // Weird... our viewport x/y that we just converted isn't found in
            // our pages. This is probably a bug but we don't want to crash
            // in releases because its harmless.
            debug_assert!(false, "viewport pin not found");
            return;
        };
        let Ok(tracked) = t.screen_mut().pages.track_pin(pin) else {
            return;
        };

        let count = {
            let mut m = self.mouse.borrow_mut();

            // If we move our cursor too much between clicks then we reset
            // the multi-click state.
            if m.left_click_count > 0 {
                let max_distance = f64::from(size.cell.width);
                let distance = ((xpos - m.left_click_xpos).powi(2)
                    + (ypos - m.left_click_ypos).powi(2))
                .sqrt();

                if distance > max_distance {
                    m.left_click_count = 0;
                }
            }

            if let Some(prev) = m.left_click_pin.take() {
                let key = m.left_click_screen;
                if let Some(pin_screen) = t.screens.get_mut(key) {
                    pin_screen.pages.untrack_pin(prev);
                }
            }

            // Store it
            m.left_click_pin = Some(tracked);
            m.left_click_screen = active_key;
            m.left_click_xpos = xpos;
            m.left_click_ypos = ypos;

            // Setup our click counter and timer. If we have mouse clicks,
            // then we check if the time elapsed is less than and our
            // interval and if so, increase the count.
            let now = Instant::now();
            if m.left_click_count > 0 {
                let since = m
                    .left_click_time
                    .map_or(u64::MAX, |t| now.duration_since(t).as_nanos() as u64);
                if since > cfg.mouse_interval {
                    m.left_click_count = 0;
                }
            }

            m.left_click_time = Some(now);
            m.left_click_count += 1;

            // We only support up to triple-clicks.
            if m.left_click_count > 3 {
                m.left_click_count = 1;
            }
            m.left_click_count
        };

        // In all cases below, we set the selection directly rather than use
        // `set_selection` because we want to avoid copying the selection to
        // the selection clipboard. For left mouse clicks we only set the
        // clipboard on release.
        match count {
            // Single click
            1 => {
                // If we have a selection, clear it. This always happens.
                if t.screen().selection.is_some() {
                    let _ = t.screen_mut().select(None);
                    self.queue_render();
                }
            }

            // Double click, select the word under our mouse. First try to
            // detect if we're clicking on a URL to select the entire URL.
            2 => {
                // Try link detection without requiring modifier keys
                let sel = self
                    .link_at_pin(t.screen(), pin, None)
                    .map(|l| l.selection)
                    .or_else(|| t.screen().select_word(pin, &cfg.selection_word_chars));
                if let Some(sel) = sel {
                    let _ = t.screen_mut().select(Some(sel));
                    self.queue_render();
                }
            }

            // Triple click, select the line under our mouse
            3 => {
                let screen = t.screen();
                let sel = if mods.ctrl_or_super() {
                    screen.select_output(pin)
                } else {
                    screen.select_line(SelectLine::new(pin))
                };
                if let Some(sel) = sel {
                    let _ = t.screen_mut().select(Some(sel));
                    self.queue_render();
                }
            }

            // We should be bounded by 1 to 3
            _ => unreachable!(),
        }
    }

    /// Right button press. Returns `Some(consumed)` if the event is
    /// finished, `None` to fall through.
    fn right_press(&self, cfg: &super::config::DerivedConfig) -> Option<bool> {
        let pos = self.rt_surface.get_cursor_pos();
        let vp = self.pos_to_viewport(f64::from(pos.x), f64::from(pos.y));

        let mut paste = false;
        let consumed;
        {
            let mut t = self.lock_terminal();

            // Get our viewport pin
            let Some(pin) = t.screen().pages.pin(Point::Viewport(vp)) else {
                debug_assert!(false, "viewport pin not found");
                return None;
            };

            match cfg.right_click_action {
                RightClickAction::Ignore => consumed = true,
                RightClickAction::ContextMenu => {
                    // If we already have a selection and the selection
                    // contains where we clicked then we don't want to modify
                    // the selection.
                    if let Some(prev_sel) = t.screen().selection
                        && prev_sel.contains(&t.screen().pages, pin)
                    {
                        return None;
                    }

                    // The selection doesn't contain our pin, so we
                    // create a new word selection where we clicked.

                    // If there is a link at this position, we want to select
                    // the link. Otherwise, select the word.
                    let link = {
                        let mods = self.mouse.borrow().mods;
                        let mods = self.mouse_mods_with_capture_locked(&t, mods);
                        self.link_at_pos(&t, vp, mods)
                    };
                    if let Some(link) = link {
                        let _ = self.set_selection(&mut t, Some(link.selection));
                    } else {
                        let Some(sel) = t.screen().select_word(pin, &cfg.selection_word_chars)
                        else {
                            return None;
                        };
                        let _ = self.set_selection(&mut t, Some(sel));
                    }
                    drop(t);
                    self.queue_render();

                    // Don't consume so that we show the context menu in
                    // apprt.
                    return Some(false);
                }
                RightClickAction::Copy => {
                    if let Some(sel) = t.screen().selection {
                        self.copy_selection_to_clipboards(
                            &t,
                            sel,
                            &[Clipboard::Standard],
                            ghostty_input::binding::CopyToClipboard::Mixed,
                        );
                    }

                    let _ = self.set_selection(&mut t, None);
                    self.queue_render();
                    consumed = true;
                }
                RightClickAction::CopyOrPaste => {
                    if let Some(sel) = t.screen().selection {
                        self.copy_selection_to_clipboards(
                            &t,
                            sel,
                            &[Clipboard::Standard],
                            ghostty_input::binding::CopyToClipboard::Mixed,
                        );
                        let _ = self.set_selection(&mut t, None);
                        self.queue_render();
                    } else {
                        // We don't need to clear selection because we didn't
                        // have one to begin with.
                        paste = true;
                    }
                    consumed = true;
                }
                RightClickAction::Paste => {
                    // Before we yield the lock, clear our selection if we
                    // have one.
                    let _ = self.set_selection(&mut t, None);
                    self.queue_render();
                    paste = true;
                    consumed = true;
                }
            }
        }

        // Pasting can trigger a lock grab in complete clipboard request so
        // we must not hold the lock.
        if paste {
            self.start_clipboard_request(Clipboard::Standard, ClipboardRequest::Paste);
        }

        // Consume the event such that the context menu is not displayed.
        Some(consumed)
    }

    /// Handle a click on the prompt (Zig `maybePromptClick`). Returns true
    /// if handled.
    fn maybe_prompt_click(&self) -> bool {
        let cfg = self.cfg();
        let pos = self.rt_surface.get_cursor_pos();
        let pos_vp = self.pos_to_viewport(f64::from(pos.x), f64::from(pos.y));

        let t = self.lock_terminal();
        let screen = t.screen();

        // If our screen doesn't handle any prompt clicks, then we never do
        // anything.
        if screen.semantic_prompt.click == SemanticClick::None {
            return false;
        }

        // If cursor-click-to-move is disabled, we don't do any prompt
        // clicking.
        if !cfg.cursor_click_to_move {
            return false;
        }

        // If our cursor isn't currently at a prompt then we don't handle
        // prompt clicks because we can't move if we're not in a prompt!
        if !t.cursor_is_at_prompt() {
            return false;
        }

        // If we have a selection currently, then releasing the mouse
        // completes the selection and we don't do prompt moving.
        if screen.selection.is_some() {
            return false;
        }

        // Get the pin for our mouse click.
        let Some(click_pin) = screen.pages.pin(Point::Viewport(pos_vp)) else {
            debug_assert!(false, "viewport pin not found");
            return false;
        };

        // Get our cursor's most current prompt.
        let Some(cursor_pin) = screen.pages.tracked_pin(screen.cursor.page_pin) else {
            return false;
        };
        let Some(prompt_pin) = cursor_pin
            .prompt_iterator(&screen.pages, Direction::LeftUp, None)
            .next()
        else {
            // This shouldn't be possible because we asserted we're at a
            // prompt above, so we MUST find some prompt in a left_up search.
            log::warn!("cursor is at prompt but no prompt found");
            return false;
        };

        // If our mouse click is before the prompt, we don't move. We DO
        // ALLOW clicks AFTER the prompt, specifically with Kitty's
        // click_events=1 since we rely on the shell to validate out of
        // bounds clicks. This matches Kitty's logic as best I can tell.
        if click_pin.before(&screen.pages, prompt_pin) {
            return false;
        }

        // At this point we've established:
        // - Screen supports prompt clicks
        // - Cursor is at a prompt
        // - Click is at or below our prompt
        let mut writes: Vec<IoMessage> = Vec::new();
        match screen.semantic_prompt.click {
            // Guarded at the start of this function
            SemanticClick::None => unreachable!(),

            SemanticClick::ClickEvents => {
                // For the event, we always send a left-click press event.
                // This matches what Kitty sends.
                let resp = format!("\x1b[<0;{};{}M", u32::from(pos_vp.x) + 1, pos_vp.y + 1);

                // Not that noisy since this only happens on prompt clicks.
                log::debug!("sending click_events=1 event=ESC{}", &resp[1..]);
                writes.push(IoMessage::write_req(resp.as_bytes()));
            }

            SemanticClick::Cl(_) => {
                let app = t.modes.get(Mode::CursorKeys);
                let left_arrow: &'static [u8] = if app { b"\x1bOD" } else { b"\x1b[D" };
                let right_arrow: &'static [u8] = if app { b"\x1bOC" } else { b"\x1b[C" };

                let mv = screen.prompt_click_move(click_pin);
                for _ in 0..mv.left {
                    writes.push(IoMessage::write_stable(left_arrow));
                }
                for _ in 0..mv.right {
                    writes.push(IoMessage::write_stable(right_arrow));
                }
            }
        }
        drop(t);
        for w in writes {
            self.queue_io(w);
        }

        true
    }

    // ------------------------------------------------------------------
    // Links

    /// Returns the link at the given viewport point, if any. Requires the
    /// renderer lock (pass the locked terminal) and the capture-adjusted
    /// mods.
    fn link_at_pos(&self, t: &Terminal, vp: PointCoordinate, mouse_mods: Mods) -> Option<Link> {
        // Convert our cursor position to a screen point.
        let screen = t.screen();
        let Some(mouse_pin) = screen.pages.pin(Point::Viewport(vp)) else {
            log::warn!("failed to get pin for clicked point");
            return None;
        };

        // If we have the proper modifiers set then we can check for OSC8
        // links.
        if mouse_mods.equal(ghostty_input::ctrl_or_super(Mods::default()))
            && let Some((_, cell)) = mouse_pin.row_and_cell(&screen.pages)
            && cell.hyperlink()
        {
            let sel = Selection::new(mouse_pin, mouse_pin, false);
            return Some(Link {
                action: LinkAction::_OpenOsc8,
                selection: sel,
            });
        }

        // Fall back to configured links
        self.link_at_pin(screen, mouse_pin, Some(mouse_mods))
    }

    /// Detects if a link is present at the given pin.
    ///
    /// If mouse mods is `None` then mouse mod requirements are ignored (all
    /// configured links are checked).
    fn link_at_pin(
        &self,
        screen: &Screen,
        mouse_pin: Pin,
        mouse_mods: Option<Mods>,
    ) -> Option<Link> {
        let cfg = self.cfg();
        // Check eligibility before walking scrollback or allocating a string
        // map. Ordinary mouse movement needs no URL lookup without Ctrl/Super.
        let mut links = cfg
            .links
            .iter()
            .filter(|link| {
                mouse_mods.is_none_or(|mods| match link.highlight {
                    Highlight::Always | Highlight::Hover => true,
                    Highlight::AlwaysMods(v) | Highlight::HoverMods(v) => v.equal(mods),
                })
            })
            .peekable();
        if links.peek().is_none() {
            return None;
        }

        let line = screen.select_line_bounded(
            SelectLine {
                pin: mouse_pin,
                whitespace: None,
                semantic_prompt_boundary: false,
            },
            MAX_LINK_CELLS / usize::from(screen.pages.cols),
        )?;

        let strmap = screen.selection_string_map(SelectionString {
            sel: line,
            trim: false,
        });

        for link in links {
            let mut it = strmap.search_iterator(&link.regex);
            while let Some(m) = it.next_match() {
                let sel = m.selection();
                if !sel.contains(&screen.pages, mouse_pin) {
                    continue;
                }
                return Some(Link {
                    action: link.action,
                    selection: sel,
                });
            }
        }

        None
    }

    /// Return the URI for an OSC8 hyperlink at the given position or `None`
    /// if there is no hyperlink.
    fn osc8_uri(screen: &Screen, pin: Pin) -> Option<Vec<u8>> {
        let node = screen.pages.node(pin.node)?;
        let page = &node.data;
        let idx = page
            .cell_index(usize::from(pin.x), usize::from(pin.y))
            .ok()?;
        let id = page.lookup_hyperlink(idx)?;
        let entry = page.hyperlink_set.get(id)?;
        Some(entry.uri.clone())
    }

    /// The URL of the link (`open` regex link or OSC8) under the current
    /// cursor position, if the mouse is over a link (Zig
    /// `copy_url_to_clipboard`). Takes the renderer lock.
    pub(crate) fn link_url_at_cursor(&self) -> Option<Vec<u8>> {
        // If the mouse isn't over a link, nothing we can do.
        if !self.mouse.borrow().over_link {
            return None;
        }
        let pos = self.rt_surface.get_cursor_pos();
        let vp = self.pos_to_viewport(f64::from(pos.x), f64::from(pos.y));
        let trim = self.cfg().clipboard_trim_trailing_spaces;

        let t = self.lock_terminal();
        let mods = self.mouse.borrow().mods;
        let mods = self.mouse_mods_with_capture_locked(&t, mods);
        let link = self.link_at_pos(&t, vp, mods)?;
        let screen = t.screen();
        match link.action {
            LinkAction::Open => Some(
                screen
                    .selection_string(SelectionString {
                        sel: link.selection,
                        trim,
                    })
                    .into_bytes(),
            ),
            LinkAction::_OpenOsc8 => {
                let uri = Self::osc8_uri(screen, link.selection.start(&screen.pages));
                if uri.is_none() {
                    log::warn!("failed to get URI for OSC8 hyperlink");
                }
                uri
            }
        }
    }

    /// Attempt to invoke the action of any link that is under the given
    /// position. Takes the renderer lock.
    fn process_links(&self, pos: CursorPos) -> Result<bool, CoreError> {
        let vp = self.pos_to_viewport(f64::from(pos.x), f64::from(pos.y));
        let url = {
            let t = self.lock_terminal();
            let mods = self.mouse.borrow().mods;
            let mods = self.mouse_mods_with_capture_locked(&t, mods);
            let Some(link) = self.link_at_pos(&t, vp, mods) else {
                return Ok(false);
            };
            let screen = t.screen();
            match link.action {
                LinkAction::Open => {
                    let str = screen.selection_string(SelectionString {
                        sel: link.selection,
                        trim: false,
                    });
                    // The matched text is plain terminal text (a URL or a
                    // path), never an escaped OS string.
                    open_link_target(t.get_pwd(), &str)
                }

                LinkAction::_OpenOsc8 => {
                    let start = link.selection.start(&screen.pages);
                    let Some(uri) = Self::osc8_uri(screen, start) else {
                        log::warn!("failed to get URI for OSC8 hyperlink");
                        return Ok(false);
                    };
                    ghostty_os::osbytes::EncodedOs::from_bytes(&uri)
                }
            }
        };

        self.open_url(OpenUrl {
            kind: OpenUrlKind::Unknown,
            url,
        })?;
        Ok(true)
    }

    pub(crate) fn open_url(&self, action: OpenUrl) -> Result<(), CoreError> {
        // If the apprt handles it then we're done.
        let kind = action.kind;
        let url = action.url.clone();
        if self.perform(RtAction::OpenUrl(action)) {
            return Ok(());
        }

        // apprt didn't handle it, fallback to our simple cross-platform URL
        // opener. We log a warning because we want well-behaved apprts to
        // handle this themselves.
        log::warn!("apprt did not handle open URL action, falling back to default opener");
        let kind = match kind {
            OpenUrlKind::Unknown => ghostty_os::open::OpenUrlKind::Unknown,
            OpenUrlKind::Text => ghostty_os::open::OpenUrlKind::Text,
            OpenUrlKind::Html => ghostty_os::open::OpenUrlKind::Html,
        };
        ghostty_os::open::open(kind, &url.to_os())?;
        Ok(())
    }

    /// Call this whenever the mouse moves or mods changed. Takes the
    /// renderer lock; must not be called with it held.
    pub(crate) fn mouse_refresh_links(
        &self,
        pos: CursorPos,
        pos_vp: PointCoordinate,
        over_link: bool,
    ) -> Result<(), CoreError> {
        let mut deferred = Deferred::default();
        {
            let mut st = self.lock_state();
            self.mouse_refresh_links_locked(&mut st, pos, pos_vp, over_link, &mut deferred);
        }
        deferred.run(self);
        Ok(())
    }

    fn mouse_refresh_links_locked(
        &self,
        st: &mut RenderStateData,
        pos: CursorPos,
        pos_vp: PointCoordinate,
        over_link: bool,
        out: &mut Deferred,
    ) {
        // If the position is outside our viewport, do nothing
        if pos.x < 0.0 || pos.y < 0.0 {
            return;
        }

        let cfg = self.cfg();

        // Update the last point that we checked for links so we don't
        // recheck if the mouse moves some pixels to the same point.
        self.mouse.borrow_mut().link_point = Some(pos_vp);

        // Get our link at the current position. This returns None if there
        // isn't a link OR if we shouldn't be showing links for some reason
        // (see further comments for cases).
        let (link, preview): (Option<MouseOverLink>, bool) = 'link: {
            // If we clicked and our mouse moved cells then we never
            // highlight links until the mouse is unclicked. This follows
            // standard macOS and Linux behavior where a click and drag
            // cancels mouse actions.
            {
                let m = self.mouse.borrow();
                if m.click_state[MouseButton::Left as usize] == MouseButtonState::Press
                    && let Some(h) = m.left_click_pin
                {
                    let screen = st.terminal.screen();
                    if let Some(pin) = screen.pages.tracked_pin(h)
                        && let Some(pt) = screen
                            .pages
                            .point_from_pin(ghostty_terminal::point::Tag::Viewport, pin)
                        && pt.coord() != pos_vp
                    {
                        log::debug!("mouse moved while left click held, ignoring link hover");
                        break 'link (None, false);
                    }
                }
            }

            let mods = self.mouse.borrow().mods;
            let mods = self.mouse_mods_with_capture_locked(&st.terminal, mods);
            let Some(link) = self.link_at_pos(&st.terminal, pos_vp, mods) else {
                break 'link (None, false);
            };
            let screen = st.terminal.screen();
            match link.action {
                LinkAction::Open => {
                    let str = screen.selection_string(SelectionString {
                        sel: link.selection,
                        trim: false,
                    });
                    (
                        Some(MouseOverLink {
                            url: ghostty_os::osbytes::EncodedOs::from_plain_text(&str),
                        }),
                        cfg.link_previews == LinkPreviews::True,
                    )
                }

                LinkAction::_OpenOsc8 => {
                    // Show the URL in the status bar
                    let pin = link.selection.start(&screen.pages);
                    let Some(uri) = Self::osc8_uri(screen, pin) else {
                        log::warn!("failed to get URI for OSC8 hyperlink");
                        break 'link (None, false);
                    };
                    (
                        Some(MouseOverLink {
                            url: ghostty_os::osbytes::EncodedOs::from_bytes(&uri),
                        }),
                        cfg.link_previews != LinkPreviews::False,
                    )
                }
            }
        };

        // If we found a link, setup our internal state and notify the apprt
        // so it can highlight it.
        if let Some(link) = link {
            st.mouse.point = Some(pos_vp);
            self.mouse.borrow_mut().over_link = true;
            st.terminal.screen_mut().dirty.hyperlink_hover = true;
            out.actions.push(RtAction::MouseShape(
                ghostty_vt::mouse_shape::MouseShape::Pointer,
            ));

            if preview {
                out.actions.push(RtAction::MouseOverLink(link));
            }

            out.render = true;
            return;
        }

        // No link, if we're previously over a link then we need to clear the
        // over-link apprt state.
        if over_link {
            out.actions
                .push(RtAction::MouseShape(st.terminal.mouse_shape));
            out.actions.push(RtAction::MouseOverLink(MouseOverLink {
                url: ghostty_os::osbytes::EncodedOs::new(),
            }));
            out.render = true;
        }
    }

    // ------------------------------------------------------------------
    // Pressure

    /// Mouse pressure (force click) callback.
    pub fn mouse_pressure_callback(&self, stage: MousePressureStage, _pressure: f64) {
        // We don't currently use the pressure value for anything. In the
        // future, we could report this to applications using new mouse
        // events or utilize it for some custom UI.

        // If the pressure stage is the same as what we already have do
        // nothing
        let left_pressed = {
            let mut m = self.mouse.borrow_mut();
            if m.pressure_stage == stage {
                return;
            }

            // Update our pressure stage.
            m.pressure_stage = stage;
            m.click_state[MouseButton::Left as usize] == MouseButtonState::Press
        };

        // If our left mouse button is pressed and we're entering a deep
        // click then we want to start a selection. We treat this as a word
        // selection since that is typical macOS behavior.
        if left_pressed && stage == MousePressureStage::Deep {
            let handle = self.mouse.borrow().left_click_pin;
            let word_chars = self.cfg().selection_word_chars.clone();
            let mut t = self.lock_terminal();

            // This should always be set in this state but we don't want to
            // handle state inconsistency here.
            let Some(h) = handle else { return };
            let Some(pin) = t.screen().pages.tracked_pin(h) else {
                return;
            };
            let Some(sel) = t.screen().select_word(pin, &word_chars) else {
                return;
            };
            if t.screen_mut().select(Some(sel)).is_ok() {
                self.queue_render();
            }
        }
    }

    // ------------------------------------------------------------------
    // Cursor position

    /// Cursor position callback.
    ///
    /// Send negative x or y values to indicate the cursor is outside the
    /// viewport. The magnitude of the negative values are meaningless; they
    /// are only used to indicate the cursor is outside the viewport. It's
    /// important to do this to ensure hover states are cleared.
    ///
    /// The mods parameter is optional because some apprts do not provide
    /// modifier information on cursor position events. If mods is `None`
    /// then we'll use the last known mods.
    pub fn cursor_pos_callback(&self, pos: CursorPos, mods: Option<Mods>) -> Result<(), CoreError> {
        let mut deferred = Deferred::default();

        // If the position is negative, it is outside our viewport and we
        // need to clear any hover states.
        if pos.x < 0.0 || pos.y < 0.0 {
            // Reset our hyperlink state
            let was_over_link = {
                let mut m = self.mouse.borrow_mut();
                m.link_point = None;
                let was = m.over_link;
                m.over_link = false;
                was
            };
            if was_over_link {
                let shape = self.lock_terminal().mouse_shape;
                self.perform(RtAction::MouseShape(shape));
                self.perform(RtAction::MouseOverLink(MouseOverLink {
                    url: ghostty_os::osbytes::EncodedOs::new(),
                }));
                self.queue_render();
            }

            let mut st = self.lock_state();

            // No mouse point so we don't highlight links
            st.mouse.point = None;

            // Mark the link's row as dirty, but continue with updating the
            // mouse state below so we can scroll when our position is
            // negative.
            st.terminal.screen_mut().dirty.hyperlink_hover = true;
        }

        // Always show the mouse again if it is hidden
        let hidden = self.mouse.borrow().hidden;
        if hidden {
            self.show_mouse();
        }

        // Update our modifiers if they changed
        if let Some(v) = mods {
            self.mods_changed(v);
        }

        // The mouse position in the viewport
        let pos_vp = self.pos_to_viewport(f64::from(pos.x), f64::from(pos.y));

        // If we have an inspector, we need to always record position
        // information
        self.inspector_record_cursor_pos(f64::from(pos.x), f64::from(pos.y));

        // We always reset the over link status because it will be
        // reprocessed below. But we need the old value to know if we need to
        // undo mouse shape changes.
        let over_link = {
            let mut m = self.mouse.borrow_mut();
            let o = m.over_link;
            m.over_link = false;
            o
        };

        let selection_scroll_active = self.selection_scroll_active.get();
        let size = self.size.get();

        {
            // We are reading/writing state for the remainder
            let mut st = self.lock_state();

            // Stop selection scrolling when inside the viewport within a 1px
            // buffer for fullscreen windows, but only when selection
            // scrolling is active.
            if pos.y >= 1.0 && selection_scroll_active {
                deferred.io.push(IoMessage::SelectionScroll(false));
            }

            // Update our mouse state. We set this to null initially because
            // we only want to set it when we're not selecting or doing any
            // other mouse event.
            st.mouse.point = None;

            // Handle link hovering. We refresh links when
            // 1. we were previously over a link
            // OR
            // 2. the cursor position has changed (either we have no
            //    previous state, or the state has changed)
            // AND
            // 1. mouse reporting is off
            // OR
            // 2. mouse reporting is on and we are not reporting shift to the
            //    terminal
            let (link_point, mouse_mods) = {
                let m = self.mouse.borrow();
                (m.link_point, m.mods)
            };
            if (over_link || link_point.is_none_or(|p| p != pos_vp))
                && (st.terminal.flags.mouse_event == MouseEvents::None
                    || (mouse_mods.shift && !self.mouse_shift_capture(&st.terminal)))
            {
                // If we were previously over a link, we always update. We do
                // this so that if the text changed underneath us, even if the
                // mouse didn't move, we update the URL hints and state
                self.mouse_refresh_links_locked(&mut st, pos, pos_vp, over_link, &mut deferred);
            }

            // Do a mouse report
            if self.is_mouse_reporting(&st.terminal) {
                // Shift overrides mouse "grabbing" in the window, taken from
                // Kitty. This only applies if there is a mouse button
                // pressed so that movement reports are not affected.
                let skip = mouse_mods.shift
                    && !self.mouse_shift_capture(&st.terminal)
                    && self
                        .mouse
                        .borrow()
                        .click_state
                        .iter()
                        .any(|s| *s != MouseButtonState::Release);

                if !skip {
                    let mut m = self.mouse.borrow_mut();

                    // We use the first mouse button we find pressed in order
                    // to report since the spec (afaict) does not say...
                    let button = m
                        .click_state
                        .iter()
                        .position(|s| *s == MouseButtonState::Press)
                        .and_then(button_from_index);

                    let mods = m.mods;
                    if let Some(b) = self.mouse_report(
                        &st.terminal,
                        &mut m,
                        button,
                        MouseReportAction::Motion,
                        mods,
                        pos,
                    ) {
                        deferred.io.push(IoMessage::write_req(&b));
                    }

                    // If we're doing mouse motion tracking, we do not
                    // support text selection.
                    drop(m);
                    drop(st);
                    deferred.run(self);
                    return Ok(());
                }
            }

            // Handle cursor position for text selection
            let (left_pressed, count, click_screen, handle, click_xpos, sel_mods) = {
                let m = self.mouse.borrow();
                (
                    m.click_state[MouseButton::Left as usize] == MouseButtonState::Press,
                    m.left_click_count,
                    m.left_click_screen,
                    m.left_click_pin,
                    m.left_click_xpos,
                    m.mods,
                )
            };

            // Left click pressed but count zero can happen if mouse
            // reporting is on. In this scenario, we mark the click state
            // because we need that to properly make some mouse reports, but
            // we don't keep track of the count because we don't want to
            // handle selection.
            //
            // If our terminal screen changed then we don't process this. We
            // don't invalidate our pin or mouse state because if the screen
            // switches back then we can continue our selection.
            if left_pressed && count != 0 && click_screen == st.terminal.screens.active_key {
                // All roads lead to requiring a re-render at this point.
                deferred.render = true;

                // If our y is negative, we're above the window. In this
                // case, we scroll up. The amount we scroll up is dependent
                // on how negative we are. We allow for a 1 pixel buffer at
                // the top and bottom to detect scroll even in full screen
                // windows.
                let max_y = size.screen.height as f32;

                // If the mouse is outside the viewport and we have the left
                // mouse button pressed then we need to start the scroll
                // timer.
                if (pos.y <= 1.0 || pos.y > max_y - 1.0) && !selection_scroll_active {
                    deferred.io.push(IoMessage::SelectionScroll(true));
                }

                // Convert to points
                let t = &mut st.terminal;
                let Some(pin) = t.screen().pages.pin(Point::Viewport(pos_vp)) else {
                    debug_assert!(false, "viewport pin not found");
                    drop(st);
                    deferred.run(self);
                    return Ok(());
                };

                // Handle dragging depending on click count
                if let Some(click_pin) = handle.and_then(|h| t.screen().pages.tracked_pin(h)) {
                    let word_chars = self.cfg().selection_word_chars.clone();
                    match count {
                        1 => Self::drag_left_click_single(
                            t,
                            click_pin,
                            pin,
                            click_xpos,
                            f64::from(pos.x),
                            sel_mods,
                            size,
                        ),
                        2 => {
                            let sel = drag_left_click_double_selection(
                                t.screen(),
                                click_pin,
                                pin,
                                &word_chars,
                            );
                            match sel {
                                Some(sel) => {
                                    let _ = t.screen_mut().select(Some(sel));
                                }
                                None => {
                                    let _ = self.set_selection(t, None);
                                }
                            }
                        }
                        3 => {
                            if let Some(sel) =
                                drag_left_click_triple_selection(t.screen(), click_pin, pin)
                            {
                                let _ = t.screen_mut().select(Some(sel));
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }

        deferred.run(self);
        Ok(())
    }

    fn drag_left_click_single(
        t: &mut Terminal,
        click_pin: Pin,
        drag_pin: Pin,
        click_xpos: f64,
        drag_x: f64,
        mods: Mods,
        size: Size,
    ) {
        // This logic is in a separate function so that it can be unit
        // tested.
        let sel = mouse_selection(
            &t.screen().pages,
            click_pin,
            drag_pin,
            click_xpos.max(0.0) as u32,
            drag_x.max(0.0) as u32,
            mods,
            size,
        );
        let _ = t.screen_mut().select(sel);
    }

    /// Selection scrolling tick (Zig `selectionScrollTick`).
    pub(crate) fn selection_scroll_tick(&self) -> Result<(), CoreError> {
        // If we're no longer active then we don't do anything.
        if !self.selection_scroll_active.get() {
            return Ok(());
        }

        // If we don't have a left mouse button down then we don't do
        // anything.
        let (count, screen_key, handle, xpos, mods) = {
            let m = self.mouse.borrow();
            (
                m.left_click_count,
                m.left_click_screen,
                m.left_click_pin,
                m.left_click_xpos,
                m.mods,
            )
        };
        if count == 0 {
            return Ok(());
        }

        let pos = self.rt_surface.get_cursor_pos();
        let pos_vp = self.pos_to_viewport(f64::from(pos.x), f64::from(pos.y));
        let delta: isize = if pos.y < 0.0 { -1 } else { 1 };
        let size = self.size.get();

        let mut t = self.lock_terminal();

        // If our screen changed while this is happening, we stop our
        // selection scroll.
        if screen_key != t.screens.active_key {
            drop(t);
            self.queue_io(IoMessage::SelectionScroll(false));
            return Ok(());
        }

        // Scroll the viewport as required
        t.scroll_viewport(ScrollViewport::Delta(delta));

        // Next, trigger our drag behavior
        let Some(pin) = t.screen().pages.pin(Point::Viewport(pos_vp)) else {
            debug_assert!(false, "viewport pin not found");
            return Ok(());
        };
        if let Some(click_pin) = handle.and_then(|h| t.screen().pages.tracked_pin(h)) {
            Self::drag_left_click_single(
                &mut t,
                click_pin,
                pin,
                xpos,
                f64::from(pos.x),
                mods,
                size,
            );
        }

        // We modified our viewport and selection so we need to queue a
        // render.
        drop(t);
        self.queue_render();
        Ok(())
    }
}

/// `@enumFromInt(i)` for the click state index.
fn button_from_index(i: usize) -> Option<MouseButton> {
    Some(match i {
        0 => MouseButton::Unknown,
        1 => MouseButton::Left,
        2 => MouseButton::Right,
        3 => MouseButton::Middle,
        4 => MouseButton::Four,
        5 => MouseButton::Five,
        6 => MouseButton::Six,
        7 => MouseButton::Seven,
        8 => MouseButton::Eight,
        9 => MouseButton::Nine,
        10 => MouseButton::Ten,
        11 => MouseButton::Eleven,
        _ => return None,
    })
}

/// Queues a render when dropped (Zig: `defer self.queueRender()`).
struct RenderOnDrop<'a>(&'a Surface);

impl Drop for RenderOnDrop<'_> {
    fn drop(&mut self) {
        self.0.queue_render();
    }
}

#[cfg(test)]
mod tests {
    use ghostty_renderer::size::{CellSize, Padding, ScreenSize};
    use ghostty_terminal::screen::Options as ScreenOptions;

    use super::*;

    /// Our screen size is 10x5 cells that are 10x20 px, with 5px padding on
    /// all sides.
    fn test_size() -> Size {
        Size {
            cell: CellSize {
                width: 10,
                height: 20,
            },
            padding: Padding {
                left: 5,
                top: 5,
                right: 5,
                bottom: 5,
            },
            screen: ScreenSize {
                width: 110,
                height: 110,
            },
        }
    }

    struct Setup {
        screen: Screen,
        mods: Mods,
        click_pin: Pin,
        drag_pin: Pin,
        click_x_pos: u32,
        drag_x_pos: u32,
    }

    fn setup(click_x: f64, click_y: u32, drag_x: f64, drag_y: u32, rect: bool) -> Setup {
        let size = test_size();
        let screen = Screen::new(ScreenOptions::new(10, 5, 0)).unwrap();

        // We hold both ctrl and alt for rectangular select so that this
        // test is platform agnostic.
        let mods = Mods {
            ctrl: rect,
            alt: rect,
            ..Mods::default()
        };
        assert_eq!(rect, surface_mouse::is_rectangle_select_state(mods));

        let vp = |x: f64, y: u32| {
            Point::Viewport(PointCoordinate {
                x: x.floor() as u16,
                y,
            })
        };
        let click_pin = screen.pages.pin(vp(click_x, click_y)).unwrap();
        let drag_pin = screen.pages.pin(vp(drag_x, drag_y)).unwrap();

        let cell_width = f64::from(size.cell.width);
        let click_x_pos = (click_x * cell_width).floor() as u32 + size.padding.left;
        let drag_x_pos = (drag_x * cell_width).floor() as u32 + size.padding.left;

        Setup {
            screen,
            mods,
            click_pin,
            drag_pin,
            click_x_pos,
            drag_x_pos,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn test_mouse_selection(
        click_x: f64,
        click_y: u32,
        drag_x: f64,
        drag_y: u32,
        start_x: u16,
        start_y: u32,
        end_x: u16,
        end_y: u32,
        rect: bool,
    ) {
        let s = setup(click_x, click_y, drag_x, drag_y, rect);
        let start_pin = s
            .screen
            .pages
            .pin(Point::Viewport(PointCoordinate {
                x: start_x,
                y: start_y,
            }))
            .unwrap();
        let end_pin = s
            .screen
            .pages
            .pin(Point::Viewport(PointCoordinate { x: end_x, y: end_y }))
            .unwrap();

        assert_eq!(
            Some(Selection::new(start_pin, end_pin, rect)),
            mouse_selection(
                &s.screen.pages,
                s.click_pin,
                s.drag_pin,
                s.click_x_pos,
                s.drag_x_pos,
                s.mods,
                test_size(),
            )
        );
    }

    /// Like `test_mouse_selection` but checks that the resulting selection
    /// is null.
    fn test_mouse_selection_is_null(
        click_x: f64,
        click_y: u32,
        drag_x: f64,
        drag_y: u32,
        rect: bool,
    ) {
        let s = setup(click_x, click_y, drag_x, drag_y, rect);
        assert_eq!(
            None,
            mouse_selection(
                &s.screen.pages,
                s.click_pin,
                s.drag_pin,
                s.click_x_pos,
                s.drag_x_pos,
                s.mods,
                test_size(),
            )
        );
    }

    #[test]
    fn surface_selection_logic() {
        // -- LTR
        // single cell selection
        test_mouse_selection(3.0, 3, 3.9, 3, 3, 3, 3, 3, false);
        // including click and drag pin cells
        test_mouse_selection(3.0, 3, 5.9, 3, 3, 3, 5, 3, false);
        // including click pin cell but not drag pin cell
        test_mouse_selection(3.0, 3, 5.0, 3, 3, 3, 4, 3, false);
        // including drag pin cell but not click pin cell
        test_mouse_selection(3.9, 3, 5.9, 3, 4, 3, 5, 3, false);
        // including neither click nor drag pin cells
        test_mouse_selection(3.9, 3, 5.0, 3, 4, 3, 4, 3, false);
        // empty selection (single cell on only left half)
        test_mouse_selection_is_null(3.0, 3, 3.1, 3, false);
        // empty selection (single cell on only right half)
        test_mouse_selection_is_null(3.8, 3, 3.9, 3, false);
        // empty selection (between two cells, not crossing threshold)
        test_mouse_selection_is_null(3.9, 3, 4.0, 3, false);

        // -- RTL
        // single cell selection
        test_mouse_selection(3.9, 3, 3.0, 3, 3, 3, 3, 3, false);
        // including click and drag pin cells
        test_mouse_selection(5.9, 3, 3.0, 3, 5, 3, 3, 3, false);
        // including click pin cell but not drag pin cell
        test_mouse_selection(5.9, 3, 3.9, 3, 5, 3, 4, 3, false);
        // including drag pin cell but not click pin cell
        test_mouse_selection(5.0, 3, 3.0, 3, 4, 3, 3, 3, false);
        // including neither click nor drag pin cells
        test_mouse_selection(5.0, 3, 3.9, 3, 4, 3, 4, 3, false);
        // empty selection (single cell on only left half)
        test_mouse_selection_is_null(3.1, 3, 3.0, 3, false);
        // empty selection (single cell on only right half)
        test_mouse_selection_is_null(3.9, 3, 3.8, 3, false);
        // empty selection (between two cells, not crossing threshold)
        test_mouse_selection_is_null(4.0, 3, 3.9, 3, false);

        // -- Wrapping
        // LTR, wrap excluded cells
        test_mouse_selection(9.9, 2, 0.0, 4, 0, 3, 9, 3, false);
        // RTL, wrap excluded cells
        test_mouse_selection(0.0, 4, 9.9, 2, 9, 3, 0, 3, false);
    }

    #[test]
    fn surface_rectangle_selection_logic() {
        // -- LTR
        // single column selection
        test_mouse_selection(3.0, 2, 3.9, 4, 3, 2, 3, 4, true);
        // including click and drag pin columns
        test_mouse_selection(3.0, 2, 5.9, 4, 3, 2, 5, 4, true);
        // including click pin column but not drag pin column
        test_mouse_selection(3.0, 2, 5.0, 4, 3, 2, 4, 4, true);
        // including drag pin column but not click pin column
        test_mouse_selection(3.9, 2, 5.9, 4, 4, 2, 5, 4, true);
        // including neither click nor drag pin columns
        test_mouse_selection(3.9, 2, 5.0, 4, 4, 2, 4, 4, true);
        // empty selection (single column on only left half)
        test_mouse_selection_is_null(3.0, 2, 3.1, 4, true);
        // empty selection (single column on only right half)
        test_mouse_selection_is_null(3.8, 2, 3.9, 4, true);
        // empty selection (between two columns, not crossing threshold)
        test_mouse_selection_is_null(3.9, 2, 4.0, 4, true);

        // -- RTL
        // single column selection
        test_mouse_selection(3.9, 2, 3.0, 4, 3, 2, 3, 4, true);
        // including click and drag pin columns
        test_mouse_selection(5.9, 2, 3.0, 4, 5, 2, 3, 4, true);
        // including click pin column but not drag pin column
        test_mouse_selection(5.9, 2, 3.9, 4, 5, 2, 4, 4, true);
        // including drag pin column but not click pin column
        test_mouse_selection(5.0, 2, 3.0, 4, 4, 2, 3, 4, true);
        // including neither click nor drag pin columns
        test_mouse_selection(5.0, 2, 3.9, 4, 4, 2, 4, 4, true);
        // empty selection (single column on only left half)
        test_mouse_selection_is_null(3.1, 2, 3.0, 4, true);
        // empty selection (single column on only right half)
        test_mouse_selection_is_null(3.9, 2, 3.8, 4, true);
        // empty selection (between two columns, not crossing threshold)
        test_mouse_selection_is_null(4.0, 2, 3.9, 4, true);

        // -- Wrapping
        // LTR, do not wrap
        test_mouse_selection(9.9, 2, 0.0, 4, 9, 2, 0, 4, true);
        // RTL, do not wrap
        test_mouse_selection(0.0, 4, 9.9, 2, 0, 4, 9, 2, true);
    }

    #[test]
    fn resolve_path_for_opening_semantics() {
        // Absolute paths are used as-is.
        assert_eq!(resolve_path_for_opening(Some(b"/tmp"), "/etc/passwd"), None);
        // No pwd, no resolution.
        assert_eq!(resolve_path_for_opening(None, "foo"), None);
        // Non-existent relative paths are not resolved.
        assert_eq!(
            resolve_path_for_opening(Some(b"/tmp"), "does-not-exist-xyz"),
            None
        );
        // Existing relative path resolves, including `..`.
        assert_eq!(
            resolve_path_for_opening(Some(b"/tmp"), "../etc"),
            Some(ghostty_os::osbytes::EncodedOs::from_plain_text("/etc"))
        );
    }

    #[test]
    fn open_link_plain_text_is_never_decoded() {
        // Regex-matched text is ordinary text: the escape character followed
        // by U+00FF must not turn into the byte 0xFF.
        let text = "https://example.com/\u{fdd0}\u{ff}";
        let t = open_link_target(Some(b"/tmp"), text);
        assert_eq!(t.to_bytes(), text.as_bytes());
        let abs = open_link_target(None, "/etc/\u{fdd0}\u{ff}");
        assert_eq!(abs.to_bytes(), "/etc/\u{fdd0}\u{ff}".as_bytes());
    }

    #[cfg(unix)]
    #[test]
    fn open_link_resolves_against_non_utf8_pwd() {
        use std::os::unix::ffi::OsStrExt;
        let td = ghostty_os::TempDir::init().unwrap();
        let dir = td.path().join(std::ffi::OsStr::from_bytes(b"d\xffir"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("file.txt"), "x").unwrap();
        // The OSC 7 pwd is raw bytes.
        let pwd = dir.as_os_str().as_bytes().to_vec();
        let t = open_link_target(Some(&pwd), "file.txt");
        assert_eq!(t.to_path().unwrap(), dir.join("file.txt"));
        assert!(t.to_path().unwrap().exists());
    }

    #[test]
    fn osc8_uri_bytes_are_carried_unchanged() {
        let uri = b"https://example.com/a\xffb".to_vec();
        let t = ghostty_os::osbytes::EncodedOs::from_bytes(&uri);
        assert_eq!(t.to_bytes(), uri);
        assert_ne!(t.as_encoded().as_bytes(), uri.as_slice());
    }

    #[test]
    fn scroll_amount_direction() {
        let a = ScrollAmount { delta: -3 };
        assert!(!a.up_right());
        assert_eq!(a.magnitude(), 3);
        assert!(ScrollAmount { delta: 2 }.up_right());
    }
}
