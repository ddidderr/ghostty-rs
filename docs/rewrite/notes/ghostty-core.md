# ghostty-core notes

## Keyboard (fork K) - crates/ghostty-core/src/surface/keyboard.rs
Ported: Keyboard state, keyEventIsBinding, keyCallback, maybeHandleBinding (sequences/leader, key tables incl. `once`,
catch_all ignore, chained actions, performable, consumed/global/all, last_trigger release suppression), endKeySequence,
encodeKey (+ `encode_options_from_terminal` = `key_encode.Options.fromTerminal`), key-table helpers
(`activate_key_table`, `deactivate_key_table`, `deactivate_all_key_tables`), `release_pressed_key_on_unfocus`,
`SurfaceMouse.keyToMouseShape` (private copy, because it is only used here at the moment).
Deviations: sequences/tables hold paths/names resolved against the current config (no raw pointers);
global/all chained actions are re-implemented locally (`perform_all_chained_action`) as the surface has no `App` handle;
no inspector; macOS option-as-alt skipped; `once` table deactivation calls `deactivate_key_table` directly.
Needs from the mouse module: `Mouse.{mods: Mods, over_link: bool, hidden: bool}`, `mods_changed(Mods)`,
`mouse_shift_capture(bool) -> bool`, `mouse_refresh_links(CursorPos, Coordinate, bool) -> Result<(), CoreError>`.
Tests: 8 (Zig has none for this code): OwnedKeyEvent round trip, closing actions, sequence path resolution, table lookup,
leader path, encode options from terminal modes, mouse shape for keys.

## Binding actions (fork B) - surface/binding.rs

- Ported: `Search` (+ search-thread event callback), `Text`/`dump_text`/`dump_text_locked` (viewport math in free fn `text_viewport`), `perform_binding_action` for every action, `closing_action`, `writeScreenFile` (free fns `screen_file_contents` / `write_screen_temp_file`; the temp dir is deliberately left on disk like Zig), `adjust_selection` (free fn on `&mut Terminal`).
- App-scoped actions are mapped locally (`perform_app_action`) because `App::perform_action` needs an `App`, not the shared `AppState`; `new_window` from a surface targets the surface.
- `toggle_mouse_reporting` rebuilds the `DerivedConfig` with a cloned `Config` (Zig mutated in place).
- Inspector action only forwards `RtAction::Inspector` (inspector is a no-op).
- Cross-crate change: `ghostty_terminal::search::thread::Options.terminal` is now `Arc<dyn TerminalLock>` (trait implemented for `Mutex<Terminal>`; core implements it for the shared render state) because the search thread must lock the terminal inside `ghostty_renderer::state::State`.
- Requires from the mouse module: `link_url_at_cursor(&self) -> Option<String>` (returns the link text under the cursor incl. clipboard trimming and OSC8 URI; None when not over a link); also `mods_changed`, `mouse_shift_capture`, `mouse_refresh_links` used by keyboard.rs.
- Tests: 7 (no Zig tests in this range).

## Mouse (fork M)
Files: `crates/ghostty-core/src/surface/mouse.rs` (Mouse state, mouse reporting x10/utf8/sgr/urxvt/sgr-pixels, button/cursor/scroll/pressure callbacks, click-to-move, multi-click select, right-click actions, middle-click paste, links incl. OSC8, selection scrolling) and `src/surface_mouse.rs`.
- Tests: `key_to_mouse_shape`, `surface_selection_logic`, `surface_rectangle_selection_logic` ported 1:1 (selection math is the pure `mouse_selection(&PageList, ..)`); 3 extra.
- Deviations: mouse reports/IO messages/runtime actions are collected and run after the renderer lock is released (Zig held the lock and used `.locked` sends). `process_links` takes the lock itself (Zig called it unlocked). Left-release copy-on-select copies via `copy_on_select` directly: Zig's `setSelection(prev-equal)` early-returns on `eql`, which would never copy.
- Extra API for binding fork: `link_url_at_cursor()`.

## Core overview (integration, agent CORE)
Done: trait boundary moved to `ghostty_apprt::runtime_api` (apprt now depends on terminal+renderer and re-exports the real `Scrollbar`/`RendererHealth`);
termio uses `ghostty_renderer::state::{State,SharedState}` (`SharedStateExt::lock_terminal`); renderer `update_kitty` calls `image::State::kitty_update`;
termio applies `image-storage-limit` (init + config change) and deletes kitty images on clear-screen; `app.rs` (App.zig, `CoreApp` impl, app mailbox),
`global.rs` (logging in Zig format, GHOSTTY_LOG, rlimits, resources dir, i18n), `surface/mod.rs` (init/teardown with renderer+IO threads, message handler,
config updates, sizing, fonts, focus, preedit, selection, `CoreSurface` impl), `surface/{clipboard,keyboard,mouse,binding,config}.rs`, `surface_mouse.rs`; root `src/main.rs`.
Wiring details: renderer->app via `Host` (scrollbar/health/redraw as app mailbox messages); termio->renderer via `RendererLink` + `Wakeup::with_hook`;
apprt-gtk got `RtSurface::gl_proc_address` (libepoxy `epoxy_*` pointer variables) and `create_scope_fn` (cgroup scope over D-Bus from the IO thread).
Deviations/stubs: inspector no-op; locale only via GTK's setlocale (no unsafe in core/root); crash reporting (sentry) not ported; `Rust version` row of `+version` empty;
default DPI 96 (Zig linux).
How to run (resources are found automatically via `<workspace>/resources`, see notes/cleanup-V2.md): `GSETTINGS_BACKEND=memory cargo run -- -e sh -c '...'`.
Dev screenshot: `GHOSTTY_DEBUG_SCREENSHOT=/abs/out.png [GHOSTTY_DEBUG_SCREENSHOT_MS=2500]` dumps the renderer's last frame (compositor screenshots under niri come out black / are blocked).
Verified manually: window opens on Wayland and X11, shell prompt and SGR colored output render (target/screenshots/frame.png, shell.png), child exit closes window, clean shutdown.
NOT verified: keyboard input end-to-end (no way to inject keys into the window here; xdotool/niri did not deliver), mouse, clipboard, IME, splits/tabs.
Known warnings at runtime: `unimplemented action=CellSize` (same as Zig GTK), cgroup scope creation fails with UnixProcessIdUnknown in this sandbox (non-fatal).

## Surface shutdown and bounded message delivery

`Surface::drop` first cancels the surface's `app::MailboxScope`, then closes
its IO and renderer queues, before joining search, renderer, and IO threads.
The IO thread joins its reader after signaling the reader quit pipe. This
ordering breaks the dependency cycle in `ghostty-hang.269303`: UI joins IO,
IO joins reader, reader waits to enqueue `SetTitle` into the full UI mailbox.
The quit pipe alone cannot interrupt a reader inside a mailbox send.

- App layer: every worker-facing UI sender (renderer host, IO, reader, search)
  carries the same surface cancellation receiver. Dropping the scope's sole
  sender broadcasts cancellation to all waiting sends. The global UI queue
  remains bounded at 64 and other surfaces' scopes stay active.
- IO layer: `Mailbox::close` disconnects a shared cancellation channel;
  `send`, `send_locked`, and `try_send` respect it. The locked variant still
  reacquires its terminal guard after cancellation. The IO consumer also closes
  its queue on loop exit before backend teardown joins the reader.
- Renderer layer: `BlockingQueue::close` sets a mutex-protected terminal state
  and broadcasts its condvar; both indefinite and timed pushes return zero.
  Renderer `Handle::stop` closes its queue as well.

No queue enlargement, timeout polling, or event-loop reentry is needed.
Already-enqueued messages remain drainable. Crossbeam sends already selected
concurrently with close may finish enqueueing; they cannot remain blocked.
Messages for a closing surface can be discarded; normal live-surface ordering
and backpressure remain unchanged. This addresses full-mailbox **surface
teardown**, not every possible runtime backpressure cycle or driver hang.

Regression: `tests/e2e_shutdown.rs` fills the UI queue, has a real child emit
an OSC title, observes that the reader parsed it, and drops the real surface
without pumping the UI, both with and without an active search. A subprocess
watchdog detects a stuck join. Removing the pre-join cancellation reproduces
the deadlock at teardown; restoring it completes both cases in about 0.2 s. Run with
`LIBGL_ALWAYS_SOFTWARE=1 cargo test -p ghostty-core --test e2e_shutdown` on a
machine with headless Mesa EGL. Queue unit tests cover broadcast cancellation,
timed/indefinite sends, future rejection, retained messages, and scope isolation.
See `DEVIANT_FIXES.md` row 43; this is separate from the original NVIDIA crash.
