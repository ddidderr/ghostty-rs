# ghostty-inspector (agent I1): terminal inspector

Ports `ZIG/src/inspector/*` (Inspector, widgets/{key,page,pagelist,renderer,screen,style,surface,terminal,termio}, units),
the GTK classes `imgui_widget`, `inspector_widget`, `inspector_window` (+ their `.blp`), and the core Surface hooks.

## Stack
`dear-imgui-rs` 0.19 (Dear ImGui 1.92.9b docking branch, builds fine with rustc 1.99) + `dear-imgui-glow` 0.19 (glow 0.18, the
same glow as the renderer). imgui-rs 0.12 was not used: it has no safe docking/DockBuilder and an old ImGui. The safe API of
`dear-imgui-rs` is strict (it panics on API misuse, e.g. table phases, dockspace ordering), which the headless test below exercises.

## Layout
* `crates/ghostty-inspector` (`unsafe_code = "deny"`)
  * `inspector.rs`: `Inspector` (`mouse`, `gui`), `setup(ctx, scale)`, `Shared` (= `Mutex<Inspector>` newtype implementing
    `ghostty_renderer::state::InspectorHook`).
  * `widgets/*.rs`: 1:1 with the Zig widget files (`surface.rs` has the dockspace + Surface info + `SurfaceInfo` input struct).
  * `keys.rs`: `tag_name` (Zig `@tagName` from `Debug`), `imgui_key` (Zig `Key.imguiKey`).
  * `dock.rs`: the ONLY `unsafe` module (`#![allow(unsafe_code)]`, all SAFETY-commented): `DockBuilder` (node create/split/dock),
    `SetNextWindowSize`, `SetNextItemAllowOverlap`, and a raw `Begin`/`End` scope (`begin_window`) so that widgets still run when
    `Begin` returned false, like Zig's `Begin` + `defer End`.
* `ghostty-renderer::state`: `State.overlay_features` replaced by `State.inspector: Option<Arc<dyn InspectorHook>>` (Zig
  `inspector: ?*Inspector`); `generic.rs` reads `overlay_features()` from it each frame. The trait breaks the crate cycle.
* `ghostty-termio`: `Output::process_output` goes into Zig's "slow mode" while an inspector is set (`record_pty_read` per byte, then
  `process_byte`).
* `ghostty-core` (`surface/inspector.rs`): `activate_inspector` / `deactivate_inspector` (render-state pointer + renderer and IO
  `Inspector(bool)` messages), `render_inspector(ui)` (locks terminal then inspector, fills `SurfaceInfo`, wakes the renderer),
  key event recording in `keyboard.rs` (event created after key remapping, `binding` set in the consumed branch, `pty` copied in
  `encode_key`, recorded + `queue_render` when done, only if the inspector is still active), mouse recording in
  `mouse.rs` (cursor pos, `queue_render` on buttons). The `inspector` binding action already forwarded `RtAction::Inspector`.
* `ghostty-apprt` `CoreSurface`: new `render_inspector(&Ui)` next to `activate/deactivate_inspector`.
* `ghostty-apprt-gtk`: `class/{imgui_widget,inspector_widget,inspector_window}.rs`, `resources/ui/1.5/{imgui-widget,inspector-widget,
  inspector-window}.blp` (verbatim from Zig), `Surface::control_inspector` (toggle/show/hide, weak ref to the window) and
  `redraw_inspector`, `Window::toggle_inspector` (`win.toggle-inspector`, ctrl+shift+i). Unsafe only for `glow::Context::from_loader_function`
  (epoxy loader) and the GL read-back of the debug screenshot, both `#[allow(unsafe_code)]` + SAFETY.

## GTK input/rendering (imgui_widget)
GLArea + `IMMulticontext` as in Zig: focus/key/click/motion/scroll/IM-commit controllers feed `Io` (modifier keys as LeftShift/Ctrl/Alt/Super
events, keyval -> `imgui_key`), 30 FPS tick callback, two ImGui frames per GL render (first one only `end_frame`), then clear + draw.
Zig clears with `0x28/0xFF` which is integer 0, so the background is black; kept.

## Debug / verification
`GHOSTTY_DEBUG_INPUT_SCRIPT` got `inspector-screenshot <png>`, `inspector-click <x> <y>`, `inspector-scroll <x> <y> <dy>`
(`debug_input.rs`). Inspector frames are rendered synchronously for these steps because the compositor does not send frame
callbacks to an off-screen window (without this the screenshot lags one event behind). Run:
`GSETTINGS_BACKEND=memory GHOSTTY_DEBUG_INPUT_SCRIPT=script.txt timeout 25 ./target/debug/ghostty -e sh` with
`key ctrl+shift+i`, `wait 2500`, `inspector-click 126 11`, `inspector-screenshot /abs/x.png`, ...
Verified in the real app on Wayland (screenshots viewed): all tabs (Terminal with screens/Misc/Layout/Mouse/Color/Modes tables, Surface,
Keyboard event table with pty encoding, Terminal IO VT log, Renderer overlays, ImGui demo), screen pop-out window with Info/Cell/PageList
docking (cell picking resolves the cell at (0,0)), detach of a header into a docked window, toggle open/close/reopen, closing a surface
and a tab while its inspector is open (no warnings, core deactivates in the surface's `dispose`).

## Tests
* ghostty-inspector: 15 unit tests (Zig has none for this package): label/pty/utf8 formatting, ring bound, hsv/key colour, text filter
  (ImGuiTextFilter semantics), OSC/CSI/ESC/Execute encodings and event recording through a real `Stream`, features toggle, snake_case,
  imgui key map, mouse button labels, plus `draws_frames_headless`: draws 8 frames of the whole inspector headlessly (all windows, pop-out
  screen window, palette window, detached headers, selected VT event) to catch misuse of the strict imgui API.
* ghostty-core `tests/e2e_inspector.rs`: 4 tests (key events with pty data + binding, VT events from a real child, overlay features
  reach the render state, activate/deactivate semantics).
* apprt-gtk `debug_input` parse test extended.
Skipped: none (Zig package has no tests).

## Deviations
* Font size is `16 * scale_factor` (Zig hardcodes `16 * 2` with a TODO). The imgui style is scaled by the change of the factor
  (a `Style` cannot be re-created from Rust like Zig's `ImGuiStyle_ImGuiStyle`).
* `GhosttyImguiWidget` virtual methods (`setup`/`render`) are hooks installed with `ImguiWidget::set_hooks` (gtk-rs class-struct
  boilerplate avoided); the widget hierarchy `InspectorWidget -> ImguiWidget -> AdwBin` is real.
* The inspector widget keeps an `Rc<dyn CoreSurface>` and the GTK Surface destroys its inspector window in `dispose` (Zig used a
  GObject weak notify to call `deactivateInspector` while the core surface is alive).
* DockBuilder splits requested by the pop-out headers (`DetachableHeader`, `detachable_header`) are queued and applied after all windows
  of the frame ended (`dock::flush_splits`): doing the split while the parent window is being built re-parents its dock node and trips
  Dear ImGui's "Must call EndChild() and not End()" assertion. The popped-out window therefore docks one frame later.
* Popups: the VT filter popup uses a Rust port of `ImGuiTextFilter` (`TextFilter`, same Build/PassFilter semantics) because the safe wrapper's
  type holds a raw pointer (not `Send`) and the inspector is shared with the IO thread.
* VT events: `Parser.Action.format` for DCS/APC is approximated (`parser.Parser.Action{ .dcs_hook = ... }`); the OSC metadata for
  `color_operation.requests` / `kitty_color_protocol.list` print the Zig container type name; metadata keeps insertion order (Zig hash map order).
  `vt_raw` only sees parser-driven actions (CSI/ESC/OSC/DCS/APC), exactly like Zig: printable text and C0 controls in the ground state
  are dispatched straight to the handler, so they never appear in the log.
* The recorded cursor is a `CursorSnapshot` (displayed fields only) instead of a copy of the whole cursor struct.
* Added accessors in ghostty-terminal: `Page::string_used_bytes` / `string_capacity_bytes`, `PageList::viewport_pin_handle`.
* ImGui demo window is shown when `debug_assertions` (Zig: `builtin.mode == .Debug`); the workspace release profile keeps debug assertions,
  so it is also shown there.

## Known gaps
* No unit tests can drive real GTK events; clicks/scroll are only covered by the debug replay.
* Fractional scale factors are rounded by GTK's integer `scale_factor`.
