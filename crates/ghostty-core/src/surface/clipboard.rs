//! Clipboard handling for a surface (Zig `Surface.zig`: `clipboardWrite`,
//! `copySelectionToClipboards`, clipboard requests, paste, OSC 52 and
//! desktop notifications).

use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ghostty_apprt::action::{Action as RtAction, DesktopNotification};
use ghostty_apprt::runtime_api::ClipboardError;
use ghostty_apprt::structs::{Clipboard, ClipboardRequest};
use ghostty_config::types_b::ClipboardAccess;
use ghostty_input::binding::CopyToClipboard;
use ghostty_input::paste;
use ghostty_terminal::formatter::{self, Format, ScreenFormatter};
use ghostty_terminal::selection::Selection;
use ghostty_terminal::terminal::Terminal;
use ghostty_termio::message::Message as IoMessage;
use ghostty_vt::modes::Mode;

use super::Surface;
use super::config::DerivedConfig;

/// `input.paste.Options.fromTerminal`.
#[must_use]
pub fn paste_options_from_terminal(t: &Terminal) -> paste::Options {
    paste::Options {
        bracketed: t.modes.get(Mode::BracketedPaste),
    }
}

impl Surface {
    /// Handle an OSC 52 write: `data` is base64 (Zig `clipboardWrite`).
    pub(crate) fn clipboard_write(&self, data: &[u8], loc: Clipboard) {
        let cfg = self.cfg();
        if cfg.clipboard_write == ClipboardAccess::Deny {
            log::info!(
                "application attempted to write clipboard, but 'clipboard-write' is set to deny"
            );
            return;
        }

        // Zig validates the size first (`calcSizeForSlice` on the padded
        // standard alphabet rejects lengths that aren't a multiple of 4),
        // then decodes ignoring only the `InvalidPadding` error (e.g. nonzero
        // trailing bits as in "YR==", which still decodes to "a").
        if !data.len().is_multiple_of(4) {
            log::info!("application sent invalid base64 data for OSC 52");
            return;
        }
        let lenient = base64::engine::GeneralPurpose::new(
            &base64::alphabet::STANDARD,
            base64::engine::GeneralPurposeConfig::new()
                .with_decode_padding_mode(base64::engine::DecodePaddingMode::RequireCanonical)
                .with_decode_allow_trailing_bits(true),
        );
        let Ok(decoded) = lenient.decode(data) else {
            log::info!("application sent invalid base64 data for OSC 52");
            return;
        };
        let text = String::from_utf8_lossy(&decoded).into_owned();

        // When clipboard-write is "ask" a prompt is displayed to the user
        // asking them to confirm the clipboard access. Each app runtime
        // handles this differently.
        let confirm = cfg.clipboard_write == ClipboardAccess::Ask;
        self.rt_surface
            .set_clipboard(loc, &[("text/plain".to_owned(), text)], confirm);
    }

    /// Format the selection and put it on the given clipboards (Zig
    /// `copySelectionToClipboards`). The caller holds the terminal lock.
    pub(crate) fn copy_selection_to_clipboards(
        &self,
        t: &Terminal,
        sel: Selection,
        clipboards: &[Clipboard],
        format: CopyToClipboard,
    ) {
        let cfg = self.cfg();

        // The options we'll use for all formatting. We'll just override the
        // emit format.
        let codepoint_map: Vec<formatter::CodepointMap> =
            cfg.clipboard_codepoint_map.map.list.clone();
        let opts = formatter::Options {
            emit: Format::Plain, // We'll override this below
            unwrap: true,
            trim: cfg.clipboard_trim_trailing_spaces,
            codepoint_map: Some(codepoint_map),
            background: t.colors.background.get(),
            foreground: t.colors.foreground.get(),
            palette: Some(t.colors.palette.current),
        };

        let run = |opts: formatter::Options| -> Option<String> {
            let mut f = ScreenFormatter::new(t.screen(), opts);
            f.content = formatter::Content::Selection(Some(sel));
            match f.format_string() {
                Ok(s) => Some(s),
                Err(err) => {
                    log::error!("error formatting selection for clipboard err={err:?}");
                    None
                }
            }
        };

        let mut contents: Vec<(String, String)> = Vec::new();
        match format {
            CopyToClipboard::Plain => {
                let Some(s) = run(opts) else { return };
                contents.push(("text/plain".to_owned(), s));
            }

            CopyToClipboard::Vt => {
                let mut o = opts;
                o.emit = Format::Vt;
                // Note: We don't apply codepoint mappings to VT format since
                // it contains escape sequences that should be preserved
                // as-is
                let Some(s) = run(o) else { return };
                contents.push(("text/plain".to_owned(), s));
            }

            CopyToClipboard::Html => {
                let mut o = opts;
                o.emit = Format::Html;
                // Note: We don't apply codepoint mappings to HTML format
                // since HTML has its own character encoding and entity
                // system
                let Some(s) = run(o) else { return };
                contents.push(("text/html".to_owned(), s));
            }

            CopyToClipboard::Mixed => {
                // First, generate plain text with codepoint mappings applied
                let Some(plain) = run(opts.clone()) else {
                    return;
                };
                contents.push(("text/plain".to_owned(), plain));

                // Second, generate HTML without codepoint mappings
                let mut o = opts;
                o.emit = Format::Html;
                // We purposely don't emit background/foreground for mixed
                // mode because the HTML contents is often used for rich text
                // input and with trimmed spaces it looks pretty bad.
                o.background = None;
                o.foreground = None;
                let Some(html) = run(o) else { return };
                contents.push(("text/html".to_owned(), html));
            }
        }

        for clipboard in clipboards {
            self.rt_surface.set_clipboard(*clipboard, &contents, false);
        }
    }

    /// Start a clipboard request, with some basic validation. For example,
    /// an OSC 52 request is not actually requested if OSC 52 is disabled.
    ///
    /// Returns true if the request was started, false if it was not (e.g.,
    /// clipboard doesn't contain text for paste requests). This allows
    /// performable keybinds to pass through when the action cannot be
    /// performed.
    pub(crate) fn start_clipboard_request(&self, loc: Clipboard, req: ClipboardRequest) -> bool {
        match req {
            ClipboardRequest::Paste => {} // always allowed
            ClipboardRequest::Osc52Read(_) => {
                if self.cfg().clipboard_read == ClipboardAccess::Deny {
                    log::info!(
                        "application attempted to read clipboard, but 'clipboard-read' is set to deny"
                    );
                    return false;
                }
            }

            // No clipboard write code paths travel through this function
            ClipboardRequest::Osc52Write(_) => unreachable!(),
        }

        self.rt_surface.clipboard_request(loc, req)
    }

    /// The runtime answers a clipboard request (Zig:
    /// `completeClipboardRequest`).
    pub fn complete_clipboard_request(
        &self,
        req: ClipboardRequest,
        data: &str,
        confirmed: bool,
    ) -> Result<(), ClipboardError> {
        match req {
            ClipboardRequest::Paste => self.complete_clipboard_paste(data.as_bytes(), confirmed),

            ClipboardRequest::Osc52Read(clipboard) => {
                self.complete_clipboard_read_osc52(data.as_bytes(), clipboard, confirmed)
            }

            ClipboardRequest::Osc52Write(clipboard) => {
                self.rt_surface.set_clipboard(
                    clipboard,
                    &[("text/plain".to_owned(), data.to_owned())],
                    !confirmed,
                );
                Ok(())
            }
        }
    }

    /// Paste text (also the target of `text_callback`).
    pub(crate) fn complete_clipboard_paste(
        &self,
        data: &[u8],
        allow_unsafe: bool,
    ) -> Result<(), ClipboardError> {
        if data.is_empty() {
            return Ok(());
        }

        let cfg = self.cfg();
        let encode_opts = {
            let mut t = self.lock_terminal();
            let opts = paste_options_from_terminal(&t);

            // If we have paste protection enabled, we detect unsafe pastes
            // and return an error. The error approach allows apprt to
            // attempt to complete the paste before falling back to
            // requesting confirmation.
            //
            // We do not do this for bracketed pastes because bracketed
            // pastes are by definition safe since they're framed.
            let unsafe_paste = 'unsafe_paste: {
                // If we've disabled paste protection then we always allow
                // the paste.
                if !cfg.clipboard_paste_protection {
                    break 'unsafe_paste false;
                }

                // If we're allowed to paste unsafe data then we always allow
                // the paste. This is set during confirmation usually.
                if allow_unsafe {
                    break 'unsafe_paste false;
                }

                if opts.bracketed {
                    // If we're bracketed and the paste contains and ending
                    // bracket then something naughty might be going on and
                    // we never trust it.
                    if data.windows(6).any(|w| w == b"\x1b[201~") {
                        break 'unsafe_paste true;
                    }

                    // If we are bracketed and configured to trust that then
                    // the paste is not unsafe.
                    if cfg.clipboard_paste_bracketed_safe {
                        break 'unsafe_paste false;
                    }
                }

                !paste::is_safe(data)
            };

            if unsafe_paste {
                log::info!("potentially unsafe paste detected, rejecting until confirmation");
                return Err(ClipboardError::UnsafePaste);
            }

            // With the lock held, we must scroll to the bottom. We always
            // scroll to the bottom for these inputs.
            self.scroll_to_bottom(&mut t);

            opts
        };

        // Encode the data. In most cases this doesn't require any copies,
        // so we optimize for that case.
        let mut data_duped: Vec<u8>;
        let vecs = match paste::encode(data, encode_opts) {
            Ok(v) => v,
            Err(paste::Error::MutableRequired) => {
                data_duped = data.to_vec();
                paste::encode_mut(&mut data_duped, encode_opts)
            }
        };

        for vec in vecs {
            if !vec.is_empty() {
                self.queue_io(IoMessage::write_req(vec));
            }
        }
        Ok(())
    }

    fn complete_clipboard_read_osc52(
        &self,
        data: &[u8],
        clipboard_type: Clipboard,
        confirmed: bool,
    ) -> Result<(), ClipboardError> {
        let cfg = self.cfg();

        // We should never get here if clipboard-read is set to deny
        debug_assert_ne!(cfg.clipboard_read, ClipboardAccess::Deny);

        // If clipboard-read is set to ask and we haven't confirmed with the
        // user, do that now
        if cfg.clipboard_read == ClipboardAccess::Ask && !confirmed {
            return Err(ClipboardError::UnauthorizedPaste);
        }

        // Even if the clipboard data is empty we reply, since presumably the
        // client app is expecting a reply.
        let kind = match clipboard_type {
            Clipboard::Standard => 'c',
            Clipboard::Selection => 's',
            Clipboard::Primary => 'p',
        };

        // Wrap our data with the OSC code
        let mut buf = format!("\x1b]52;{kind};").into_bytes();
        buf.extend_from_slice(STANDARD.encode(data).as_bytes());
        buf.extend_from_slice(b"\x1b\\");

        self.queue_io(IoMessage::write_req(&buf));
        Ok(())
    }

    /// Set `clipboard-read` / `clipboard-write` to a fixed value for this
    /// surface (the runtime calls this after the user chose "remember").
    pub(crate) fn set_config_clipboard(&self, read: bool, access: ClipboardAccess) {
        let mut config = (*self.cfg().config).clone();
        if read {
            config.clipboard_read = access;
        } else {
            config.clipboard_write = access;
        }
        match DerivedConfig::new(Arc::new(config)) {
            Ok(d) => *self.config.borrow_mut() = Rc::new(d),
            Err(err) => log::warn!("failed to update clipboard access err={err}"),
        }
    }

    /// Show a desktop notification, rate limited (Zig
    /// `showDesktopNotification`).
    pub(crate) fn show_desktop_notification(&self, title: &str, body: &str) {
        let now = Instant::now();

        // Set a limit of one desktop notification per second so that the OS
        // doesn't kill us when we run out of resources.
        let last = self.app.last_notification_time.get();
        if let Some(last) = last
            && now.duration_since(last) < Duration::from_secs(1)
        {
            log::warn!("rate limiting desktop notifications");
            return;
        }

        let new_digest = {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            title.hash(&mut hash);
            body.hash(&mut hash);
            hash.finish()
        };

        // Set a limit of one notification per five seconds for desktop
        // notifications with identical content.
        if let Some(last) = last
            && self.app.last_notification_digest.get() == new_digest
            && now.duration_since(last) < Duration::from_secs(5)
        {
            log::warn!("suppressing identical desktop notification");
            return;
        }

        self.app.last_notification_time.set(Some(now));
        self.app.last_notification_digest.set(new_digest);
        self.perform(RtAction::DesktopNotification(DesktopNotification {
            title: title.to_owned(),
            body: body.to_owned(),
        }));
    }

    /// `textCallback`: text input that is pasted verbatim.
    pub fn text_callback(&self, text: &str) -> Result<(), ClipboardError> {
        self.complete_clipboard_paste(text.as_bytes(), true)
    }
}
