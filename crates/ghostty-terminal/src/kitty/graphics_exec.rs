//! Kitty graphics command execution (`ZIG/src/terminal/kitty/graphics_exec.zig`).
//!
//! # Integration
//!
//! Zig's `execute` takes a `*Terminal` and reaches
//! `terminal.screens.active.kitty_images` (the [`ImageStorage`]),
//! `terminal.screens.active.pages`, the cursor, `terminal.index()` and
//! `terminal.setCursorPos()`. In Rust the storage lives inside the `Screen`
//! next to the `PageList`, so everything `execute` needs is exposed through
//! the small [`ExecTerminal`] trait (notably a split borrow of storage +
//! page list). `Terminal` implements it by delegating to its active screen;
//! see `docs/rewrite/notes/ghostty-terminal-kitty.md` for the exact impl.

use ghostty_vt::kitty::graphics_command::{Command, Control, Display, Quiet, Response};

use super::graphics_image::{Error as ImageError, LoadingImage};
use super::graphics_storage::{
    Error as StorageError, ImageStorage, Location, Placement, TerminalInfo,
};
use crate::page_list::{PageList, Pin};

/// The terminal operations the Kitty graphics executor needs.
pub trait ExecTerminal {
    /// The active screen's image storage and page list (a split borrow of
    /// `screens.active().kitty_images` and `screens.active().pages`).
    fn kitty_parts(&mut self) -> (&mut ImageStorage, &mut PageList);

    /// Snapshot of terminal geometry and cursor position.
    fn exec_info(&self) -> TerminalInfo;

    /// The active screen cursor's current pin (`cursor.page_pin.*`).
    fn exec_cursor_pin(&self) -> Pin;

    /// `Terminal.index`. Errors are logged and stop cursor movement.
    fn exec_index(&mut self) -> Result<(), crate::page::Error>;

    /// `Terminal.setCursorPos` (1-based row/col).
    fn exec_set_cursor_pos(&mut self, row: usize, col: usize);
}

/// Execute a Kitty graphics command against the given terminal. This
/// will never fail, but the response may indicate an error and the
/// terminal state may not be updated to reflect the command. This will
/// never put the terminal in an unrecoverable state, however.
pub fn execute<T: ExecTerminal>(terminal: &mut T, cmd: &Command) -> Option<Response> {
    // If storage is disabled then we disable the full protocol. This means
    // we don't even respond to queries so the terminal completely acts as
    // if this feature is not supported.
    if !terminal.kitty_parts().0.enabled() {
        log::debug!("kitty graphics requested but disabled");
        return None;
    }

    log::debug!(
        "executing kitty graphics command: quiet={:?} control={:?}",
        cmd.quiet,
        cmd.control
    );

    // The quiet settings used to control the response. We have to make this
    // a var because in certain special cases (namely chunked transmissions)
    // this can change.
    let mut quiet = cmd.quiet;

    let resp_: Option<Response> = match &cmd.control {
        Control::Query(_) => Some(query(cmd)),
        Control::Display(_) => Some(display(terminal, cmd)),
        Control::Delete(_) => Some(delete(terminal, cmd)),

        Control::Transmit(_) | Control::TransmitAndDisplay { .. } => {
            // If we're transmitting, then our `q` setting value is complicated.
            // The `q` setting inherits the value from the starting command
            // unless `q` is set >= 1 on this command. If it is, then we save
            // that as the new `q` setting.
            let storage = terminal.kitty_parts().0;
            if let Some(loading) = storage.loading.as_mut() {
                match cmd.quiet {
                    // q=0 we use whatever the start command value is
                    Quiet::No => quiet = loading.quiet,

                    // q>=1 we use the new value, but we should already be set to it
                    Quiet::Ok | Quiet::Failures => {
                        quiet = cmd.quiet;
                        loading.quiet = cmd.quiet;
                    }
                }
            }

            Some(transmit(terminal, cmd))
        }

        Control::TransmitAnimationFrame(_)
        | Control::ControlAnimation(_)
        | Control::ComposeAnimation(_) => Some(Response {
            message: "ERROR: unimplemented action".to_owned(),
            ..Response::default()
        }),
    };

    // Handle the quiet settings
    if let Some(resp) = resp_ {
        if !resp.ok() {
            log::warn!("erroneous kitty graphics response: {}", resp.message);
        }

        return match quiet {
            Quiet::No => {
                if resp.empty() {
                    None
                } else {
                    Some(resp)
                }
            }
            Quiet::Ok => {
                if resp.ok() {
                    None
                } else {
                    Some(resp)
                }
            }
            Quiet::Failures => None,
        };
    }

    None
}

fn err_response(message: &str) -> Response {
    Response {
        message: message.to_owned(),
        ..Response::default()
    }
}

/// Execute a "query" command.
///
/// This command is used to attempt to load an image and respond with
/// success/error but does not persist any of the command to the terminal
/// state.
fn query(cmd: &Command) -> Response {
    let Control::Query(t) = &cmd.control else {
        return err_response("EINVAL: internal error");
    };

    // Query requires image ID. We can't actually send a response without
    // an image ID either but we return an error and this will be logged
    // downstream.
    if t.image_id == 0 {
        return err_response("EINVAL: image ID required");
    }

    // Build a partial response to start
    let mut result = Response {
        id: t.image_id,
        image_number: t.image_number,
        placement_id: t.placement_id,
        ..Response::default()
    };

    // Attempt to load the image. If we cannot, then set an appropriate error.
    if let Err(err) = LoadingImage::init(cmd) {
        encode_error(&mut result, err);
    }
    result
}

/// Transmit image data.
///
/// This loads the image, validates it, and puts it into the terminal
/// screen storage. It does not display the image.
fn transmit<T: ExecTerminal>(terminal: &mut T, cmd: &Command) -> Response {
    let Some(t) = cmd.transmission() else {
        return err_response("EINVAL: internal error");
    };
    let mut result = Response {
        id: t.image_id,
        image_number: t.image_number,
        placement_id: t.placement_id,
        ..Response::default()
    };
    if t.image_id > 0 && t.image_number > 0 {
        return err_response("EINVAL: image ID and number are mutually exclusive");
    }

    let load = match load_and_add_image(terminal, cmd) {
        Ok(l) => l,
        Err(err) => {
            encode_error(&mut result, err);
            return result;
        }
    };

    // If we're also displaying, then do that now. This function does
    // both transmit and transmit and display. The display might also be
    // deferred if it is multi-chunk.
    if let Some(d) = load.display {
        debug_assert!(!load.more);
        let mut d_copy = d;
        d_copy.image_id = load.image_id;
        result = display(
            terminal,
            &Command {
                control: Control::Display(d_copy),
                quiet: cmd.quiet,
                data: Vec::new(),
            },
        );
    }

    // If there are more chunks expected we do not respond.
    if load.more {
        return Response::default();
    }

    // If the loaded image was assigned its ID automatically, not based
    // on a number or explicitly specified ID, then we don't respond.
    if load.implicit_id {
        return Response::default();
    }

    // After the image is added, set the ID in case it changed.
    // The resulting image number and placement ID never change.
    result.id = load.image_id;

    result
}

/// Display a previously transmitted image.
fn display<T: ExecTerminal>(terminal: &mut T, cmd: &Command) -> Response {
    let Some(d) = cmd.display() else {
        return err_response("EINVAL: internal error");
    };

    // Display requires image ID or number.
    if d.image_id == 0 && d.image_number == 0 {
        return err_response("EINVAL: image ID or number required");
    }

    // Build up our response
    let mut result = Response {
        id: d.image_id,
        image_number: d.image_number,
        placement_id: d.placement_id,
        ..Response::default()
    };

    let info = terminal.exec_info();
    let cursor_pin = terminal.exec_cursor_pin();

    // Verify the requested image exists if we have an ID
    let (storage, pages) = terminal.kitty_parts();
    let img_id = {
        let img = if d.image_id != 0 {
            storage.image_by_id(d.image_id)
        } else {
            storage.image_by_number(d.image_number)
        };
        let Some(img) = img else {
            "ENOENT: image not found".clone_into(&mut result.message);
            return result;
        };
        img.id
    };

    // Make sure our response has the image id in case we looked up by number
    result.id = img_id;

    // Location where the placement will go.
    let location = if d.virtual_placement {
        // Virtual placements are not tracked
        if d.parent_id > 0 {
            "EINVAL: virtual placement cannot refer to a parent".clone_into(&mut result.message);
            return result;
        }

        Location::Virtual
    } else {
        // Track a new pin for our cursor. The cursor is always tracked but we
        // don't want this one to move with the cursor.
        match pages.track_pin(cursor_pin) {
            Ok(pin) => Location::Pin(pin),
            Err(err) => {
                log::warn!("failed to create pin for Kitty graphics err={err:?}");
                "EINVAL: failed to prepare terminal state".clone_into(&mut result.message);
                return result;
            }
        }
    };

    // Add the placement
    let p = Placement {
        location,
        x_offset: d.x_offset,
        y_offset: d.y_offset,
        source_x: d.x,
        source_y: d.y,
        source_width: d.width,
        source_height: d.height,
        columns: d.columns,
        rows: d.rows,
        z: d.z,
    };
    if let Err(err) = storage.add_placement(pages, img_id, result.placement_id, p) {
        p.deinit(pages);
        encode_error(&mut result, err.into());
        return result;
    }

    // Apply cursor movement setting. This only applies to pin placements.
    match p.location {
        Location::Virtual => {}
        Location::Pin(_) => match d.cursor_movement {
            ghostty_vt::kitty::graphics_command::CursorMovement::None => {}
            ghostty_vt::kitty::graphics_command::CursorMovement::After => {
                // We use terminal.index to properly handle scroll regions.
                let size = {
                    let (storage, _) = terminal.kitty_parts();
                    match storage.image_by_id(img_id) {
                        Some(img) => p.grid_size(img, &info),
                        None => return result,
                    }
                };
                for _ in 0..size.rows {
                    if let Err(err) = terminal.exec_index() {
                        log::warn!("failed to move cursor: {err:?}");
                        break;
                    }
                }

                let y = terminal.exec_info().cursor_y;
                terminal.exec_set_cursor_pos(
                    usize::from(y),
                    usize::from(cursor_pin.x) + size.cols as usize + 1,
                );
            }
        },
    }

    result
}

/// Delete placements/images.
fn delete<T: ExecTerminal>(terminal: &mut T, cmd: &Command) -> Response {
    let Control::Delete(d) = &cmd.control else {
        return err_response("EINVAL: internal error");
    };
    let info = terminal.exec_info();
    let (storage, pages) = terminal.kitty_parts();
    storage.delete(pages, &info, *d);

    // Delete never responds on success
    Response::default()
}

struct Loaded {
    image_id: u32,
    implicit_id: bool,
    more: bool,
    display: Option<Display>,
}

fn load_and_add_image<T: ExecTerminal>(
    terminal: &mut T,
    cmd: &Command,
) -> Result<Loaded, ImageError> {
    let t = cmd.transmission().ok_or(ImageError::InternalError)?;
    let (storage, pages) = terminal.kitty_parts();

    // Determine our image. This also handles chunking and early exit.
    let mut loading: LoadingImage = if let Some(loading) = storage.loading.as_mut() {
        // Note: we do NOT want to take the command's data here because
        // we're _copying_ the data. We want the command data to be freed.
        loading.add_data(&cmd.data)?;

        // If we have more then we're done
        if t.more_chunks {
            return Ok(Loaded {
                image_id: loading.image.id,
                implicit_id: loading.image.implicit_id,
                more: true,
                display: None,
            });
        }

        // We have no more chunks. We're going to be completing the
        // image so we want to destroy the pointer to the loading
        // image and copy it out.
        match storage.loading.take() {
            Some(l) => *l,
            None => return Err(ImageError::InternalError),
        }
    } else {
        LoadingImage::init(cmd)?
    };

    // If the image has no ID, we assign one
    if loading.image.id == 0 {
        loading.image.id = storage.next_image_id;
        storage.next_image_id = storage.next_image_id.wrapping_add(1);

        // If the image also has no number then its auto-ID is "implicit".
        // See the doc comment on the Image.implicit_id field for more detail.
        if loading.image.number == 0 {
            loading.image.implicit_id = true;
        }
    }

    // If this is chunked, this is the beginning of a new chunked transmission.
    // (We checked for an in-progress chunk above.)
    if t.more_chunks {
        // We allocate the pointer on the heap because its rare and we
        // don't want to always pay the memory cost to keep it around.
        let result = Loaded {
            image_id: loading.image.id,
            implicit_id: loading.image.implicit_id,
            more: true,
            display: None,
        };
        storage.loading = Some(Box::new(loading));
        return Ok(result);
    }

    // Validate and store our image
    let img = loading.complete()?;
    let image_id = img.id;
    let implicit_id = img.implicit_id;
    storage.add_image(pages, img)?;

    // Get our display settings
    let display_ = loading.display;

    Ok(Loaded {
        image_id,
        implicit_id,
        more: false,
        display: display_,
    })
}

impl From<StorageError> for ImageError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::OutOfMemory => ImageError::OutOfMemory,
        }
    }
}

/// Encode an error code into a message for a response.
fn encode_error(r: &mut Response, err: ImageError) {
    let msg = match err {
        ImageError::OutOfMemory => "ENOMEM: out of memory",
        ImageError::InternalError => "EINVAL: internal error",
        ImageError::InvalidData => "EINVAL: invalid data",
        ImageError::DecompressionFailed => "EINVAL: decompression failed",
        ImageError::FilePathTooLong => "EINVAL: file path too long",
        ImageError::TemporaryFileNotInTempDir => "EINVAL: temporary file not in temp dir",
        ImageError::TemporaryFileNotNamedCorrectly => "EINVAL: temporary file not named correctly",
        ImageError::UnsupportedFormat => "EINVAL: unsupported format",
        ImageError::UnsupportedMedium => "EINVAL: unsupported medium",
        ImageError::UnsupportedDepth => "EINVAL: unsupported pixel depth",
        ImageError::DimensionsRequired => "EINVAL: dimensions required",
        ImageError::DimensionsTooLarge => "EINVAL: dimensions too large",
    };
    msg.clone_into(&mut r.message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{Options, Terminal};
    use ghostty_vt::kitty::graphics_command::{Parser, TransmissionFormat};

    /// A `Terminal` plus the image storage that, once integrated, lives in
    /// `Screen.kitty_images`.
    // TODO(integration): replace with `Terminal` once `Terminal` implements
    // `ExecTerminal` via `screen.kitty_images: ImageStorage`.
    struct Harness {
        t: Terminal,
        storage: ImageStorage,
    }

    impl Harness {
        fn new(cols: u16, rows: u16) -> Self {
            Harness {
                t: Terminal::new(Options::new(cols, rows)).unwrap(),
                storage: ImageStorage::default(),
            }
        }
    }

    impl ExecTerminal for Harness {
        fn kitty_parts(&mut self) -> (&mut ImageStorage, &mut PageList) {
            (&mut self.storage, &mut self.t.screens.active_mut().pages)
        }
        fn exec_info(&self) -> TerminalInfo {
            TerminalInfo::from_terminal(&self.t)
        }
        fn exec_cursor_pin(&self) -> Pin {
            self.t.screens.active().cursor_pin()
        }
        fn exec_index(&mut self) -> Result<(), crate::page::Error> {
            self.t.index()
        }
        fn exec_set_cursor_pos(&mut self, row: usize, col: usize) {
            self.t.set_cursor_pos(row, col);
        }
    }

    fn run(h: &mut Harness, s: &str) -> Option<Response> {
        let cmd = Parser::parse_string(s.as_bytes()).unwrap();
        execute(h, &cmd)
    }

    #[test]
    fn kittygfx_more_chunks_with_q_1() {
        let mut t = Harness::new(5, 5);

        // Initial chunk has q=1
        let resp = run(&mut t, "a=T,f=24,t=d,i=1,s=1,v=2,c=10,r=1,m=1,q=1;////");
        assert!(resp.is_none());

        // Subsequent chunk has no q but should respect initial
        let resp = run(&mut t, "m=0;////");
        assert!(resp.is_none());
    }

    #[test]
    fn kittygfx_more_chunks_with_q_0() {
        let mut t = Harness::new(5, 5);

        // Initial chunk has q=0
        let resp = run(&mut t, "a=t,f=24,t=d,s=1,v=2,c=10,r=1,m=1,i=1,q=0;////");
        assert!(resp.is_none());

        // Subsequent chunk has no q so should respond OK
        let resp = run(&mut t, "m=0;////").unwrap();
        assert!(resp.ok());
    }

    #[test]
    fn kittygfx_more_chunks_with_chunk_increasing_q() {
        let mut t = Harness::new(5, 5);

        // Initial chunk has q=0
        let resp = run(&mut t, "a=t,f=24,t=d,s=1,v=2,c=10,r=1,m=1,i=1,q=0;////");
        assert!(resp.is_none());

        // Subsequent chunk sets q=1 so should not respond
        let resp = run(&mut t, "m=0,q=1;////");
        assert!(resp.is_none());
    }

    #[test]
    fn kittygfx_default_format_is_rgba() {
        let mut t = Harness::new(5, 5);

        let resp = run(&mut t, "a=t,t=d,i=1,s=1,v=2,c=10,r=1;///////////").unwrap();
        assert!(resp.ok());

        let img = t.storage.image_by_id(1).unwrap();
        assert_eq!(img.format, TransmissionFormat::Rgba);
    }

    #[test]
    fn kittygfx_test_valid_u32_expect_invalid_image_id() {
        let mut t = Harness::new(5, 5);

        let resp = run(&mut t, "a=p,i=4294967295").unwrap();
        assert!(!resp.ok());
        assert_eq!(resp.message, "ENOENT: image not found");
    }

    #[test]
    fn kittygfx_test_valid_i32_expect_invalid_image_id() {
        let mut t = Harness::new(5, 5);

        let resp = run(&mut t, "a=p,i=1,z=-2147483648").unwrap();
        assert!(!resp.ok());
        assert_eq!(resp.message, "ENOENT: image not found");
    }

    #[test]
    fn kittygfx_no_response_with_no_image_id_or_number() {
        let mut t = Harness::new(5, 5);

        let resp = run(&mut t, "a=t,f=24,t=d,s=1,v=2,c=10,r=1,i=0,I=0;////////");
        assert!(resp.is_none());
    }

    #[test]
    fn kittygfx_no_response_with_no_image_id_or_number_load_and_display() {
        let mut t = Harness::new(5, 5);

        let resp = run(&mut t, "a=T,f=24,t=d,s=1,v=2,c=10,r=1,i=0,I=0;////////");
        assert!(resp.is_none());
    }

    #[test]
    fn kittygfx_disabled_storage_never_responds() {
        let mut t = Harness::new(5, 5);
        t.storage.total_limit = 0;
        assert!(run(&mut t, "a=q,i=1,s=1,v=1,f=24;AAAA").is_none());
    }

    #[test]
    fn kittygfx_query_ok_and_error() {
        let mut t = Harness::new(5, 5);
        let resp = run(&mut t, "a=q,i=31,s=1,v=1,f=24;AAAA").unwrap();
        assert!(resp.ok());
        assert_eq!(resp.id, 31);
        // Nothing is stored by a query.
        assert!(t.storage.images.is_empty());

        // Without an ID there is nothing to respond to (error is only logged).
        assert!(run(&mut t, "a=q,s=1,v=1,f=24;AAAA").is_none());
    }

    #[test]
    fn kittygfx_transmit_and_display_moves_cursor() {
        let mut t = Harness::new(10, 10);
        t.t.width_px = 100;
        t.t.height_px = 100;
        // 1x2 pixel image shown in 3 columns x 2 rows.
        let resp = run(&mut t, "a=T,f=24,t=d,i=5,s=1,v=2,c=3,r=2;////////").unwrap();
        assert!(resp.ok());
        assert_eq!(resp.id, 5);
        assert_eq!(t.storage.placements.len(), 1);

        // The cursor moves to the last row of the image, right of its last column.
        let cursor = &t.t.screens.active().cursor;
        assert_eq!((cursor.x, cursor.y), (3, 1));
    }

    #[test]
    fn kittygfx_delete_all() {
        let mut t = Harness::new(10, 10);
        run(&mut t, "a=T,f=24,t=d,i=5,s=1,v=2,C=1;////////").unwrap();
        assert_eq!(t.storage.placements.len(), 1);
        let resp = run(&mut t, "a=d,d=A");
        assert!(resp.is_none());
        assert!(t.storage.placements.is_empty());
        assert!(t.storage.images.is_empty());
    }
}
