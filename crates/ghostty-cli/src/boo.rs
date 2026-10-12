//! `+boo` (ZIG/src/cli/boo.zig): the ghost animation from the Ghostty
//! website, played in the terminal.
//!
//! The frames are `src/build/framegen/frames/frame_*.txt` from the Zig tree,
//! joined with `\x01` and raw-deflate compressed into `boo_frames.deflate`
//! (the same format `framegen` produces for Zig's `framedata`).

use std::{
    io::Read,
    time::{Duration, Instant},
};

use flate2::read::DeflateDecoder;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::Rect,
    style::{Color, Style},
};

use crate::{error::Error, options::NoOptions as Options};

static FRAME_DATA: &[u8] = include_bytes!("boo_frames.deflate");

/// Width of a single frame
pub const FRAME_WIDTH: u16 = 100;
/// Height of a single frame
pub const FRAME_HEIGHT: u16 = 41;

/// One cell of an animation frame: the character and whether it is part of
/// the outline (drawn with the outline style).
pub type FrameCell = (char, bool);

/// Decompress the frames into a list of individual frames
/// (Zig: `decompressFrames`).
pub fn decompress_frames() -> Result<Vec<String>, Error> {
    let mut data = Vec::new();
    DeflateDecoder::new(FRAME_DATA).read_to_end(&mut data)?;
    let text = String::from_utf8(data).map_err(|e| Error::Other(e.to_string()))?;
    Ok(text.split('\u{1}').map(str::to_string).collect())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Span,
    InTag,
    InClosingTag,
}

/// Convert a frame to its cells (Zig: the body of `updateFrame`).
///
/// A frame is characters with html spans. When we encounter a span, we use
/// the outline style until the span ends. That is, when we find a '<', we
/// parse until '>'. Then we use the outline style until the next '<', and
/// skip until the next '>'.
#[must_use]
pub fn parse_frame(frame: &str) -> Vec<FrameCell> {
    let mut cells = Vec::with_capacity(usize::from(FRAME_WIDTH) * usize::from(FRAME_HEIGHT));
    for line in frame.split('\n') {
        let mut state = State::Normal;
        let mut outline = false;
        for ch in line.chars() {
            match state {
                State::Normal => {
                    if ch == '<' {
                        state = State::InTag;
                        // We will be entering a span
                        outline = true;
                        continue;
                    }
                }
                State::Span => {
                    if ch == '<' {
                        state = State::InTag;
                        outline = false;
                        continue;
                    }
                }
                State::InTag => {
                    // If we encounter a '/', we are a closing tag
                    // If we parse all the way to a '>' we are an opening tag: we are now in a span
                    if ch == '/' {
                        state = State::InClosingTag;
                    } else if ch == '>' {
                        state = State::Span;
                    }
                    continue;
                }
                State::InClosingTag => {
                    // If we are closing a tag, we will enter the normal state
                    if ch == '>' {
                        state = State::Normal;
                    }
                    continue;
                }
            }
            cells.push((ch, outline));
        }
    }
    cells
}

/// The animation state (Zig: `Boo`).
pub struct Boo {
    frames: Vec<String>,
    frame: usize,
    /// 30 fps
    pub framerate: Duration,
    buffer: Vec<FrameCell>,
}

impl Boo {
    pub fn new() -> Result<Boo, Error> {
        Ok(Boo {
            frames: decompress_frames()?,
            frame: 0,
            framerate: Duration::from_millis(1000 / 30),
            buffer: vec![(' ', false); usize::from(FRAME_WIDTH) * usize::from(FRAME_HEIGHT)],
        })
    }

    /// Updates our internal buffer with the current frame, then advances the
    /// frame index.
    pub fn update_frame(&mut self) {
        self.buffer = parse_frame(&self.frames[self.frame]);
        debug_assert_eq!(
            self.buffer.len(),
            usize::from(FRAME_WIDTH) * usize::from(FRAME_HEIGHT)
        );

        // Lastly, update the frame index
        self.frame += 1;
        if self.frame == self.frames.len() {
            self.frame = 0;
        }
    }

    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Draw the current frame centered in `buf`.
    pub fn draw(&self, buf: &mut Buffer) {
        let area = buf.area;
        // Warn for screen size
        if area.width < FRAME_WIDTH || area.height < FRAME_HEIGHT {
            let text = "Screen must be at least 100w x 41h";
            let w = u16::try_from(text.len()).unwrap_or(0);
            let x = area.x + area.width.saturating_sub(w) / 2;
            let y = area.y + area.height / 2;
            buf.set_stringn(x, y, text, usize::from(area.width), Style::default());
            return;
        }

        // Calculate x and y offsets to center the animation frame
        let offset_y = area.y + (area.height - FRAME_HEIGHT) / 2;
        let offset_x = area.x + (area.width - FRAME_WIDTH) / 2;

        let ghostty_style = Style::default();
        let outline_style = Style::default().fg(Color::Indexed(4));
        let frame_area = Rect::new(offset_x, offset_y, FRAME_WIDTH, FRAME_HEIGHT);
        for (i, (ch, outline)) in self.buffer.iter().enumerate() {
            let i = u16::try_from(i).unwrap_or(0);
            let x = frame_area.x + i % FRAME_WIDTH;
            let y = frame_area.y + i / FRAME_WIDTH;
            if let Some(cell) = buf.cell_mut((x, y)) {
                let mut tmp = [0u8; 4];
                cell.set_symbol(ch.encode_utf8(&mut tmp));
                cell.set_style(if *outline {
                    outline_style
                } else {
                    ghostty_style
                });
            }
        }
    }
}

/// The `boo` command is used to display the animation from the Ghostty website in the terminal
pub fn run(args: &[impl AsRef<[u8]>]) -> Result<u8, Error> {
    // Disable on non-desktop systems.
    if cfg!(not(any(
        windows,
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd"
    ))) {
        return Ok(1);
    }

    let mut opts = Options;
    crate::parse_options(&mut opts, args)?;

    let mut boo = Boo::new()?;
    let mut terminal = ratatui::try_init()?;
    let result = run_loop(&mut terminal, &mut boo);
    ratatui::restore();
    result?;
    Ok(0)
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, boo: &mut Boo) -> Result<(), Error> {
    let mut next_tick = Instant::now();
    loop {
        if Instant::now() >= next_tick {
            boo.update_frame();
            next_tick = Instant::now() + boo.framerate;
        }
        terminal.draw(|f| boo.draw(f.buffer_mut()))?;

        let timeout = next_tick.saturating_duration_since(Instant::now());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Release
                    && ((key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                        || key.code == KeyCode::Esc)
                {
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn all_frames_decode_to_full_screen() {
        let frames = decompress_frames().unwrap();
        assert_eq!(235, frames.len());
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(
                usize::from(FRAME_WIDTH) * usize::from(FRAME_HEIGHT),
                parse_frame(f).len(),
                "frame {i}"
            );
        }
    }

    #[test]
    fn spans_use_outline_style() {
        // normal, then a span (outline), then normal again.
        let cells = parse_frame("a<b>bc</b>d");
        assert_eq!(
            vec![('a', false), ('b', true), ('c', true), ('d', false)],
            cells
        );
        // Style resets on each line (a closing tag seen in the normal state keeps the outline, like Zig).
        let cells = parse_frame("<b>x\ny</b>z");
        assert_eq!(vec![('x', true), ('y', false), ('z', true)], cells);
    }

    #[test]
    fn frame_advances_and_wraps() {
        let mut boo = Boo::new().unwrap();
        for _ in 0..boo.frame_count() {
            boo.update_frame();
        }
        assert_eq!(0, boo.frame);
    }

    #[test]
    fn small_screen_warning() {
        let boo = Boo::new().unwrap();
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 10));
        boo.draw(&mut buf);
        let line: String = (0..60).map(|x| buf[(x, 5)].symbol().to_string()).collect();
        assert!(line.contains("Screen must be at least 100w x 41h"));
    }

    #[test]
    fn draw_centers_frame() {
        let mut boo = Boo::new().unwrap();
        boo.update_frame();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 50));
        boo.draw(&mut buf);
        // Anything outside the frame area stays blank.
        assert_eq!(" ", buf[(0, 0)].symbol());
        assert!((10..110).any(|x| (4..45).any(|y| buf[(x, y)].symbol() != " ")));
    }
}
