# ghostty-apprt notes

## Mapping (ZIG/src/apprt -> crates/ghostty-apprt/src)
- action.zig -> action.rs: `Target<S>` (+`TargetKey`), `Action` enum (all 65 variants, docs ported), `Key` (C-enum order, `Action::key()`), helper enums/structs (SplitDirection, GotoSplit, GotoWindow, GotoTab, ResizeSplit, MoveTab, Fullscreen, FloatWindow, SecureInput, Inspector, QuitTimer, Readonly, MouseVisibility, PromptTitle, SizeLimit, InitialSize, CellSize, SetTitle, Pwd, DesktopNotification, KeySequence, KeyTable, ColorChange/ColorKind, ReloadConfig, ConfigChange, OpenUrl/OpenUrlKind, CloseTabMode, CommandFinished, StartSearch, SearchTotal, SearchSelected, MouseOverLink).
- structs.zig -> structs.rs: ContentScale, SurfaceSize, CursorPos, ImePos (Zig `IMEPos`), Clipboard, ClipboardContent, ClipboardRequest(+Type), ColorScheme, Selection.
- surface.zig -> surface.rs: `Message`, `Mailbox<S, A>`, `AppMailbox<S>` trait, `NewSurfaceContext`, `should_inherit_working_directory`, `new_config`.
- ipc.zig -> ipc.rs: `Target` (Class/Detect), `Action::NewWindow(NewWindow{arguments})`, `ActionKey`, `Error::IpcFailed`.
- runtime.zig/none.zig -> runtime.rs: `Runtime{None,Gtk}`, `none_perform_ipc` (always false).
- Skipped: embedded.zig, browser.zig, gtk.zig (apprt-gtk), all C ABI (`C`, `CValue`, `cval`, `getGObjectType`).

## Reused types
ghostty_input::binding::Trigger, ghostty_vt::mouse_shape::MouseShape, ghostty_vt::osc::{ProgressReport, color::ColoredTarget},
ghostty_config::{config::Config, types_b::Duration, types_a::WorkingDirectory}, ghostty_datastruct::{MessageData, blocking_queue::Timeout}.

## Deviations
- Strings are `String` (not NUL-terminated); `surface::Message::SetTitle` and `DesktopNotification` keep fixed arrays (256 / 64 / 256 bytes, NUL-terminated) with `title()`/`body()` accessors.
- `ConfigChange.config` and `Message::ChangeConfig` hold `Arc<Config>` instead of a borrowed pointer.
- `GotoTab`/`ColorKind` (non-exhaustive Zig enums) are Rust enums with an `Index(i32)`/`Palette(i32)` catch-all and `from_int`/`int`.
- `Mailbox::push` forwards through the `AppMailbox<S>::push_surface_message(surface, msg, timeout)` trait (Zig wraps in `App.Message.surface_message`).
- `new_config` takes the previous focused surface's pwd as `Option<String>` rather than an `App`.
- Tests of kind `ghostty.h ...` (checkGhosttyHEnum) skipped (C ABI); `refAllDecls` skipped. Replaced by 16 Rust tests (discriminants, key order, roundtrips, mailbox, config inheritance).

## TODO(integration)
- `surface::Scrollbar` stands in for ghostty_terminal `page_list::Scrollbar`; `surface::RendererHealth` for ghostty_renderer `backend::Health`. Core should convert or (if dep order permits) re-export the real ones.
- ghostty-core implements `AppMailbox<S>` for its app mailbox (S = core surface handle) and instantiates `Target<S>`.
- Selection (structs) is the pixel-based apprt struct; terminal::Selection is not used by any payload here.
