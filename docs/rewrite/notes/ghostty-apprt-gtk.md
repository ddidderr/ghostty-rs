# ghostty-apprt-gtk notes

## Overview / status
Port of ZIG/src/apprt/gtk (GTK4 + libadwaita, gtk4-rs 0.11 / libadwaita 0.9, baseline GTK 4.14 / adw 1.5).
build.rs compiles resources/ui/**/*.blp with blueprint-compiler and bundles UI+CSS+icons into a gresource
(`register_resources()`, called in `Application::new` before startup so AdwApplication
auto-loads style.css from `resource-base-path`). Inspector/imgui deferred (menu action is a no-op with TODO). Dropped the ghostty-core
dependency (cycle): the core boundary is the trait set in `src/core.rs`.

## Split model and deferred widget lifetime

`class/split_tree.rs` replaces the model synchronously and rebuilds widgets
on an idle callback. Each model installation owns a fresh `Rc<()>` identity;
`build_tree` shares it with each `SplitTreeSplit`. Before replacing the model,
`set_tree` rotates the identity so still-attached old widgets cannot address
new nodes with stale handles. This happens independently of `notify::tree`,
which callers can defer by freezing notifications. The split's idle callback
checks identity before reading, and `resize_in_place` checks again before
writing. Tokens hold no model or widget references and cannot wrap or match
another tree's generation. In-place zoom/ratio changes preserve node identity.

Four display-dependent regression tests use real GTK widgets and GLib idle
dispatch without realizing terminals. They cover shrinking from 15 to 13
nodes with stale handle 14, a reused in-bounds handle, frozen notifications
and stale writeback, and normal divider initialization/resize. The first two
were verified to fail before the fix (abort and wrong ratio, respectively).
See entry 42 in `../DEVIANT_FIXES.md` and the README's test command.

## Temporary GTK teardown stress mode

`GHOSTTY_GTK_STRESS=1` drives the real runtime after normal config and startup.
`class/application.rs` isolates the test process, creates its initial window,
starts the timer from `gtk_stress.rs`, and removes it when the run loop ends.
The timer selects bounded random actions from current tab/pane counts and
calls existing `SplitTree`, `Surface`, `Window`, and `TabView` handlers.
`class/{split_tree,window}.rs` accept close prompts in this mode while retaining
their normal teardown paths. The driver uses weak window references and logs
each operation before execution. `WaitState` gives asynchronous startup a
one-second grace period, then logs each readiness wait at most once per second.
`recover_target` focuses a mapped, initialized pane from the current model,
or selects the next tab if none is ready; a single tab requests GTK layout.
The next timer tick rechecks readiness before any random operation. This
recovers the observed wait on a newly split, unmapped surface with no core
without manufacturing terminal initialization or GL events. Waits, target
state, recovery attempts, and resumed progress are logged; a blocked main
thread still needs external diagnosis. See the root README for controls and
limits.

`gtk_stress/zoom.rs` owns the alternating phase controller. Each zoom session
creates a tab, splits right then down, and waits for all three panes to be
ready before selecting a smallest pane. Temporary size requests on the
split container are calibrated against `GtkGLArea` width/height times GTK's
scale factor (the texture allocation rule in GTK). The original core's
texture was 629×245 BGRA8 with stride 2516, as recorded in the sibling
`gtkbug_reproducer/OBSERVATIONS.md`; the textual core backtrace alone does
not include dimensions. The full zoomed area remains larger, so toggling
still performs the real widget teardown/rebuild and GL resize path.

The session holds weak tab/surface references and restores saved layout
requests/alignment when it ends. It waits 500 ms after calibration, zooms
the same small pane for 10 seconds with 80–300 ms pauses and sporadic
700–1,899 ms pauses, then releases control to normal stress for 5–10 seconds.
Both setup failures and size mismatches are logged; setup has a 10-second
deadline. Exact sizing is verified rather than assumed, and scale factors
that cannot represent odd physical dimensions are reported. Ordinary runs
without the stress environment variable do not use this controller.

## Core trait boundary (src/core.rs, authoritative spec = the file)
- `CoreApp` (impl by ghostty-core): tick, update_config, needs_confirm_quit, color_scheme_event, perform_all_action,
  push_message(AppMessage::{OpenConfig,PresentSurface(id),NewWindow}), new_surface(id, config, rt_app, rt_surface)
  -> Rc<dyn CoreSurface>, delete_surface, surface_ids, cell_size.
- `CoreSurface`: config, key/preedit/focus/mouse_button/cursor_pos/scroll/size/content_scale callbacks, ime_point,
  has_selection, perform_binding_action, complete_clipboard_request (+ClipboardError::{UnsafePaste,UnauthorizedPaste}),
  set_clipboard_read/write_allowed, needs_confirm_quit, child_exited, renderer_realized/unrealized,
  draw_frame(fbo) (core must query GL_FRAMEBUFFER_BINDING; GL loader: `gl::proc_address`), color_scheme_callback,
  update_config, readonly/grid_size/font_size, inspector hooks.
- `RtApp::perform_action(Target<SurfaceId>, Action) -> bool`, `RtSurface` (close, cgroup, get_title/size/scale/cursor_pos,
  clipboard_request/set_clipboard, default_termio_env), `Waker` (Send+Sync, `app::waker()`).
- All main-thread only except Waker. Core must not call RtApp from other threads.

## Integration TODOs / gaps
- libgtk4-layer-shell must be linked/preloaded before libwayland-client for quick terminal.
- ghostty-termio has its own cgroup wait loop; choose one (cgroup.rs here).
- Stubs (as upstream): toggle_visibility, float_window, secure_input, close_all_windows; inspector; URLs via xdg-open.
- Runtime coverage is light: demo launches cleanly (no GTK criticals); tabs/splits/dialogs/palette not driven by hand.
- Cargo: workspace root lacks `ghostty-apprt` in workspace.dependencies; this crate uses a path dep.
- Demo: `DEMO_QUIT_AFTER_MS=4000 cargo run -p ghostty-apprt-gtk --example demo`.

# Per-component notes (by sub-agent)
## F3: dialogs + config object (class/{dialog,config,config_errors_dialog,close_confirmation_dialog,clipboard_confirmation_dialog,title_dialog,debug_warning}.rs)

Update (GC): the Zig compat fallbacks are now ported, see compat-GC.md. By default (adw_1_5) `Dialog` is an `adw::AlertDialog` subclass, DebugWarning uses the Banner (1.3) template and the clipboard dialog the 1.4 template. GType names, property names, signals and template children match the Zig classes.

Public API (all `&self` methods on `glib::wrapper!` types; signals via `connect_*`):
- `config::Config` (GhosttyConfig; props `diagnostics-buffer`, `has-diagnostics` -> generated getters `diagnostics_buffer() -> Option<gtk::TextBuffer>`, `has_diagnostics() -> bool`): `new(&CoreConfig)` (clones), `from_arc(Arc<CoreConfig>)`, `new_default()`, `get() -> Arc<CoreConfig>`, `get_mut(|&mut CoreConfig| ..)` (Arc::make_mut), `clone_config() -> CoreConfig`. Test `ghostty_config` ported.
- `dialog::Dialog` (GhosttyDialog, parent adw::AlertDialog): `present(Option<&impl IsA<Widget>>)`, `close()` (force_close); subclass via `DialogImpl: AdwAlertDialogImpl` (unsafe `IsSubclassable` impl, SAFETY-commented). Override `response` through `AdwAlertDialogImpl`.
- `config_errors_dialog::ConfigErrorsDialog`: `new(&Config)`, prop `config`, `present(Option<&W>)`, `close()`, `connect_reload_config(Fn(&Self))`.
- `close_confirmation_dialog::{CloseConfirmationDialog, Target{App,Tab,Window,Surface}}`: `new(Target)`, prop `target`, `present(Option<&W>)` (sets heading/body), `close()`, `connect_close_request(Fn(&Self))`, `connect_cancel(Fn(&Self))`. `Target::title()/body()` use ghostty_os::i18n::tr.
- `clipboard_confirmation_dialog::{ClipboardConfirmationDialog, ClipboardRequestBoxed(pub ClipboardRequest)}`: `new()`, props `request` (boxed, nullable), `clipboard-contents` (TextBuffer), `blur`, `can-remember` (generated getters/setters: `set_request(Option<ClipboardRequestBoxed>)`, `set_clipboard_contents(Option<TextBuffer>)`, `set_blur(bool)`, `set_can_remember(bool)`), helpers `set_request_value(ClipboardRequest)`, `get_request() -> Option<ClipboardRequest>`, `get_clipboard_contents()`, `present(Option<&W>)`, `connect_confirm(Fn(&Self, remember: bool))`, `connect_deny(Fn(&Self, bool))`.
- `title_dialog::{TitleDialog, Target{Surface,Tab}}` (parent adw::AlertDialog directly, as in Zig): `new(Target, Option<&str>)`, props `target`, `initial-value`, `present(&W)` (anchors to ancestor adw::ApplicationWindow/Window, async `choose`), `connect_set(Fn(&Self, &str))`.
- `debug_warning::DebugWarning`: `new()`.

Status: all type-check and are clippy-clean; runtime (template instantiation, notify callbacks) NOT yet exercised because the crate did not fully compile when F3 finished. Also added `#![allow(clippy::unwrap_used)]` to build.rs.

## F4: overlays, child-exited banner, scrolled window, css (done)

Files: `src/css.rs`, `src/class/{resize_overlay,key_state_overlay,search_overlay,surface_child_exited,surface_scrolled_window}.rs`.
Verified in an isolated scratch crate (compiles, clippy -D warnings, instantiated on a live display with no GTK criticals).
All GType names match Zig (note `GhostttySurfaceScrolledWindow` keeps Zig's 3-t typo); templates from the bundled gresource.

- `ResizeOverlay` (`GhosttyResizeOverlay`): props `duration`(u32, 750), `first-delay`(250), `label`(Option<String>), `overlay-halign/valign`. API: `new()`, `set_label(Option<&str>)`, `label()`, `schedule()`.
- `KeyStateOverlay`: props `tables`/`sequence` are **GStrv (`Vec<String>`)** instead of `ext.StringList`; read-only `has-tables`/`has-sequence`; `valign-target`. API: `set_tables(Vec<String>)`, `set_sequence(Vec<String>)`.
- `SearchOverlay`: props `active` (rw), `search-total`/`search-selected` (u64 ro), `has-search-total/selected`, `halign-target`, `valign-target`. API: `set_search_total(Option<usize>)`, `set_search_selected(Option<usize>)`, `set_search_contents(&str)`, `grab_focus_entry()` (Zig `grabFocus`; renamed to avoid clashing with `Widget::grab_focus`), `set_active(bool)`, `connect_{stop_search,search_changed(Fn(&Self, Option<String>)),next_match,previous_match}`.
- `SurfaceChildExited`: prop `data` (boxed `ChildExitedBoxed`), signal `close-request`. API: `set_child_exited(Option<&ChildExited>)`, `connect_close_request`. The Zig Noop variant (adw < 1.3) is omitted (crate requires adw 1.5).
- `SurfaceScrolledWindow`: `SurfaceScrolledWindow::new(Option<&impl IsA<Widget>>)`, props `surface` (typed `GtkWidget`, accepts the `Surface` class), `config` (typed `GObject`; the scrollbar-policy closure downcasts to `class::config::Config` and calls `.get().scrollbar` - needs `Config::get() -> impl Deref<Target = ghostty_config::config::Config>`). Binds `surface.config` -> own `config`, so Surface must expose a `config` property holding a `class::config::Config`.
- `css::CssProviders`: `new(&Display)` (installs runtime provider at APPLICATION+3), `load_all(&Config)`, `load_runtime`, `load_custom` (gtk-custom-css, 5MB limit, USER priority), `set_color_scheme(dark)` (sets `prefers-color-scheme` on providers, GTK >= 4.20, dynamic property). `css::runtime_css(&Config, gtk_416) -> String` is pure and tested. Static style*.css come from the gresource: the Application must set `resource_base_path` = `/com/mitchellh/ghostty` so AdwApplication loads them automatically.
- Template signal handlers receive the emitter (not the template) in gtk-rs, so shared handlers locate the overlay with `ancestor()`.
- build.rs fix: blueprint `.ui` files are bundled with `alias="<ver>/<name>.ui"` so resource paths are `/com/mitchellh/ghostty/ui/<ver>/<name>.ui` (previously doubled `ui/ui/`).

## F1: utilities, key, ipc (files: key, gtk_version, adw_version, weak_ref, ext/*, gsettings, cgroup, flatpak, pre_exec, post_fork, ipc/*)

Ported (Zig -> Rust): key.zig -> key.rs (`accel_from_trigger`, `xdg_shortcut_from_trigger`, `label_from_trigger` return `Option<String>`; `translate_mods`, `keyval_unicode_unshifted(&Widget, &gdk::KeyEvent, keycode) -> u32`,
`event_mods(&gdk::Event, Key, ModifierType, Action, winproto_event_mods: impl FnOnce(Option<&gdk::Device>, ModifierType)->Mods) -> Mods`, `key_from_keyval(u32)`, `keyval_from_key(Key)`; 132-entry KEYMAP).
gtk_version/adw_version -> `get_runtime_version() -> (u32,u32,u32)`, `at_least/runtime_at_least/runtime_until`, `log_version`, adw `supports_*`. "Comptime" version is the cargo feature level (4.14 / 1.5).
weak_ref.zig -> thin `WeakRef<T>` over `glib::WeakRef`. ext -> `ext::actions::{Action, add, add_to_map, add_as_group}`, `ext::StringList` (glib Boxed `GhosttyStringList`), `get_ancestor`, `g_value_holds` (boxedCopy/boxedFree/listFind not needed).
gsettings -> `get_bool/get_int/get_string(Key)`. cgroup -> `fmt_scope(pid)`, `create_scope(&gio::DBusConnection, pid, Options)`. flatpak -> `resources_dir()`.
pre_exec/post_fork -> `PreExecInfo::new(&Config)`/`pre_exec(&info) -> Option<u8>`; `PostForkInfo::new(&Config)`/`post_fork(&info, Option<pid>) -> Result<(), PostForkError>` (uses `gio::Application::default()` dbus connection; ghostty-termio's Command also has its own copy of the cgroup wait loop).
ipc -> `ipc::perform_ipc(&Target, &Action) -> Result<bool, ipc::Error>` (org.gtk.Actions.Activate, `new-window` / `new-window-command`), `ipc::dbus::DBus`.

Tests: Zig ported 11 (key 3, gtk_version 2, adw_version 1, weak_ref 1, gsettings 4 (the comptime ones became 2), ext/actions 2, ext/slice 3); extra 8. The ext/actions GTK-display test and adw test are skipped at runtime when GTK cannot init (not main thread / no display).
Deviations: only unsafe is `FromGlib` for `gdk::Key` in key.rs (annotated). `many keycode` media keys remain a TODO as upstream.

## winproto (F2) — src/winproto/{mod,noop,x11,wayland}.rs

Ported winproto.zig, noop.zig, x11.zig, wayland.zig. Deps added: gdk4-x11 (x11, xlib; Xlib via its re-exported dlopen `x11` = x11-dl), gdk4-wayland (wayland_crate), wayland-client/-backend(client_system)/-protocols(client,staging)/-protocols-plasma(client).

### Public API (consumed by class/window.rs, class/application.rs)
```rust
pub trait WindowHost {            // implemented by window.rs using a *weak* ref to the Window gobject
    fn gtk_window(&self) -> Option<gtk::Window>;   // as(gtk.Window)/Native
    fn is_quick_terminal(&self) -> bool;
    fn window_decoration(&self) -> ghostty_config::types_b::WindowDecoration;
    fn config(&self) -> Option<Arc<Config>>;       // getConfig()?.get()
}
pub enum App { None(noop::App), Wayland(wayland::App), X11(x11::App) }
App::init(&gdk::Display, app_id:&str, &Config) -> Result<App>     // wayland, then x11, else None
App::protocol() -> Protocol; App::event_mods(Option<&gdk::Device>, gdk::ModifierType) -> Mods  // falls back to crate::key::translate_mods
App::supports_quick_terminal() -> bool
App::init_quick_terminal(&mut self, &gtk::Window) -> Result<()>   // call BEFORE the window is realized (layer-shell init)
pub enum Window { None, Wayland, X11 }
Window::init(&App, Rc<dyn WindowHost>) -> Result<Window>          // window must be realized (needs gdk::Surface)
Window::{resize_event, sync_appearance, set_urgent(bool)}(&mut self) -> Result<()>
Window::client_side_decoration_enabled(&self) -> bool
Window::add_subprocess_env(&mut self, &mut Vec<(String,String)>) -> Result<()>   // X11: WINDOWID
```
Zig `deinit` = `Drop` (Wayland releases blur/decoration/slide/token objects, disconnects enter-monitor).
`noop::Window::update_config_event` (unused in Zig union) was not ported.

### Behaviour notes / deviations
* Wayland uses gdk4-wayland's shared `wayland_client::Connection` (no unsafe). We keep a private event queue; blocking roundtrips at init; while an xdg-activation token is outstanding a 50 ms glib timeout dispatches the queue. KDE blur / server-decoration / slide / xdg-activation implemented; xdg_wm_dialog_v1 presence + layer-shell < 1.0.4 check ported. Registry changes after init (global_remove) are not tracked.
* Quick terminal via `gtk4-layer-shell` crate (`LayerShell` trait). Upstream requires libgtk4-layer-shell to be loaded before libwayland-client; Rust link order is not controllable, so the final binary should link it first or run with `LD_PRELOAD=libgtk4-layer-shell.so` (else `is_supported()` is false and the quick terminal is disabled).
* X11: Xlib (dlopen'd via x11-dl) for XGet/Change/DeleteWindowProperty, XEventsQueued/XPeekEvent; `XkbQueryExtension`/`XkbSelectEventDetails` declared in a small `extern "C"` block (`#[link(name="X11")]`). `unsafe` is confined to x11.rs (`#![allow(unsafe_code)]`, SAFETY comments).
* Tests: Zig has none for winproto; 2 added (noop defaults, X11 property struct layouts). Not runtime-tested on a live X11/Wayland compositor in this fork.
* Depends on `crate::key::translate_mods(gdk::ModifierType) -> Mods` (F1).

## Application / App / GlobalShortcuts / demo (F8)

Files: `src/app.rs` (App.zig), `src/class/application.rs` (application.zig),
`src/class/global_shortcuts.rs` (global_shortcuts.zig), `src/gl.rs` (GL proc loader),
`examples/demo.rs`.

### Public API
* `app::App { pub app: Application }`: `App::new(Rc<dyn CoreApp>)`, `run()` (-> `Result<(), RunError>`),
  `terminate()`, `wakeup()`, `perform_action(Target<SurfaceId>, Action) -> bool`,
  `App::perform_ipc(&ipc::Target, &ipc::Action)` (static). `app::waker() -> Arc<dyn Waker>`
  (thread-safe, `MainContext::wakeup`) - create it before the core app and give it to core.
  `app::MUST_DRAW_FROM_APP_THREAD = true`.
* `class::application::Application` (GType `GhosttyApplication`, extends `AdwApplication`, property `config`
  (read-only `Config` object, notify on change)):
  `Application::default()`, `new(core_app)`, `core() -> Rc<dyn CoreApp>`, `rt() -> Rc<dyn RtApp>`,
  `get_config() -> class::config::Config` (object), `core_config() -> Arc<ghostty_config::Config>`,
  `winproto() -> Ref<winproto::App>` / `winproto_mut()`, `saved_language()`, `surfaces() -> Vec<Surface>`,
  `perform_action(...)`, `new_window(parent: Option<&Surface>, WindowOverrides)`, `quit()`, `wakeup()`,
  `run_loop()`, `deinit()`. `AppRt` is the `RtApp` impl (forwards to the default app; main thread only).
* The giant action switch is `Application::perform_action`. Surface targets are resolved with
  `Surface::from_id(id)` (F5); actions use the Zig mechanisms (`activate_action("tab.close"/"win.close"/
  "split-tree.*")`, `ancestor::<SplitTree/Window/Tab>`). `inspector` actions go to `Surface::control_inspector`
  (inspector deferred; no-op). Unimplemented actions (`secure_input`, `close_all_windows`, ...) log + return false.
* Startup mirrors Zig: log writer funnel (`glib::log_set_writer_func` -> `log`), version logging, `Config::load`
  (errors become a diagnostic), LANG/`language`, i18n, `GDK_DEBUG`/`GDK_DISABLE` env (`set_gtk_env`), `adw::init`,
  single-instance flag, app-id (`class`), winproto init (noop fallback), CSS via `css::CssProviders`,
  `setlocale(LC_ALL,"")` (`init_locale`, closes the INTEGRATION_TODO item for locale), style-manager <-> core color scheme
  sync (`color_scheme_event` + each `CoreSurface::color_scheme_callback`), `app.*` GActions
  (`new-window`, `new-window-command` (`as`: `-e`, `--command=`, `--working-directory=`, `--title=`), `open-config`,
  `present-surface` (u64 surface id), `quit`, `reload-config`), accelerator sync, config errors dialog, quit timer,
  `run_loop` (own `MainContext` iteration + `CoreApp::tick` + quit conditions), `quit()` with close confirmation dialog.
* `SIGUSR2` reload: glib-rs has no `g_unix_signal_add`; a helper thread (`signal-hook`) hops to the main context.
* Global shortcuts: portal `CreateSession`/`BindShortcuts` over gio D-Bus (deprecated `signal_subscribe` allowed);
  `GlobalShortcuts` signal `trigger(String)` carries the action in keybind syntax; `connect_trigger` parses it.
* `gl::proc_address(name)`: dlsym on libepoxy (works for EGL and GLX); use as the loader for glow /
  `ghostty-renderer::prepare_context`.

### Changes to `core.rs` (trait spec)
Added `AppMessage::NewWindow`, `CoreSurface::color_scheme_callback`, `CoreSurface::update_config`;
`CoreApp::perform_all_action` takes a `ghostty_input::binding::Action` (as in Zig).

### Demo
`cargo run -p ghostty-apprt-gtk --example demo` (env: `DEMO_QUIT_AFTER_MS`, `DEMO_CONFIRM=1`). Dummy core:
keybindings from the user config drive window/tab/split/palette actions; `draw_frame` clears with `background`.

### Known gaps
* `toggle_visibility`/`float_window`/`secure_input`/`close_all_windows` etc.: unimplemented like upstream.
* Open URL uses `xdg-open` (OpenURI portal is a TODO upstream too).
* `async-backend` (libxev) is meaningless in Rust and ignored.

## F7: window + command palette (class/{window,command_palette}.rs)

`window::Window` (GhosttyWindow, `adw::ApplicationWindow` subclass; template `ui/1.5/window.ui`). Props: `active-surface` (ro), `config` (rw), `debug` (ro), `titlebar-style` (ro, enum `window::TitlebarStyle` = GEnum `GhosttyWindowTitlebarStyle`), `headerbar-visible`, `tabs-autohide`, `tabs-wide`, `tabs-visible`, `toolbar-style` (`adw::ToolbarStyle`), `quick-terminal` (rw). Template callbacks follow GtkBuilder's `[sender, args.., template_instance]` calling convention (instance is the LAST parameter); the two blueprint closures use `#[template_callback(function)]`.
Actions registered on the window (`win.*`, same names/param types as Zig): about, close, close-tab (s), new-tab, new-window, prompt-surface-title, prompt-tab-title, prompt-context-tab-title, ring-bell, split-{right,left,up,down}, copy, paste, reset, clear, toggle-command-palette, toggle-inspector (no-op, inspector deferred).

Public API: `Window::new(&Application, Option<&str> title)`, `Window::new_quick_terminal(&Application)`, `winproto() -> RefMut<winproto::Window>`, `is_quick_terminal()`, `get_active_surface() -> Option<Surface>`, `get_config() -> Option<Config>`, `tab_view()/get_tab_view()`, `get_window_decoration()`, `toggle_window_decorations()`, `get_needs_confirm_quit()`, `new_tab(Option<&Surface>)`, `new_tab_for_window(Option<&Surface>, TabOverrides{command,working_directory,title})`, `select_tab(SelectTab::{Previous,Next,Last,N(usize)}) -> bool`, `move_tab(&Surface, isize) -> bool`, `toggle_tab_overview()`, `toggle_visibility()`.
Everything else (sync_appearance, sync_actions, tab/surface signal wiring: present-request, clipboard-write (toast for standard clipboard only), menu, toggle-fullscreen, toggle-maximize, init (default/min size once), tab close-request, split-tree `changed`) is private and ported 1:1, incl. tab-overview focus timer (500ms), close confirmations for window/tab, new-window-from-dragged-tab, quick-terminal autohide, urgency reset, winproto resize/scale hooks.

Deviations: parent surfaces are GTK `Surface` objects (not core surfaces); `adw::AboutDialog` always (adw >= 1.5 assumed); `GBindingGroup` replaced by a single `glib::Binding` (tab title -> window title, no sync_create, like Zig); signal handler "disconnect by data" is a registry of `SignalHandlerId`s; command palette `clipboard-write` enum compared by value 0 / nick `standard`.
TODO: toggle-inspector is a stub (log only).

`command_palette::CommandPalette` (GhosttyCommandPalette, `adw::Bin`, template `ui/1.5/command-palette.ui`, root object is the `AdwDialog`): prop `config`; signal `trigger(BoxedAction)` with `connect_trigger(|palette, &binding::Action|)`; `new()` (keeps a self-reference until the dialog's `closed`, like Zig's refSink), `toggle(&Window)`. Private `GhosttyCommand` list item (props `config`, `action-key`, `action`, `title`, `description`; regular + jump commands, comparator `compare_commands` with colon normalisation) sourced from `config.command_palette_entry` filtered by `is_action_supported_on_gtk` plus one jump command per `Application::surfaces()`.
Tests: 3 unit tests (title ordering, unsupported actions, titlebar conversion); GObject runtime paths need a display (exercised by the demo).
Shared edit: added `"gnome_44"` to the gtk4 features in Cargo.toml (glib >= 2.74/2.76 APIs such as SignalGroup/BindingGroup).

## F5: surface (class/surface.rs = class/surface.zig + Surface.zig)

`GhosttySurface` (`adw::Bin` + `gtk::Scrollable`, template `ui/1.2/surface.ui`, all template children/callbacks/closures bound).
Properties (same names as Zig): `bell-ringing`, `config` (class::config::Config), `child-exited`, `default-size`/`min-size` (boxed `Size{width,height}`),
`error`, `font-size-request` (boxed `FontSizeRequest(DesiredFontSize)`), `focused`, `mouse-hidden`, `mouse-shape` (**i32 = `MouseShape` discriminant**, Zig used a GEnum),
`mouse-hover-url`, `pwd`, `title`, `title-override`, `zoom`, `is-split`, `readonly` (read-only, from `CoreSurface::readonly`), `key-sequence`/`key-table` (**GStrv/`Vec<String>`**, matching F4's KeyStateOverlay),
and the 4 `Gtk.Scrollable` overrides. Signals: `bell`, `close-request`, `clipboard-write(u32 clipboard, str)`, `clipboard-read`, `init`, `menu`, `present-request`, `toggle-fullscreen`, `toggle-maximize`
(typed `connect_*` helpers exist for all). glib's derived accessors live on a private alias wrapper (`SurfaceProps`) so the hand-written Zig-shaped API below owns the names.

Public API on `Surface` (Zig name snake_cased; `?T` -> `Option`):
`new(SurfaceOverrides{command,working_directory,title})`, `id()`, `Surface::from_id(id)`, `Surface::all()`, `core() -> Option<Rc<dyn CoreSurface>>`, `rt() -> Rc<RtSurfaceHandle>`,
`set_parent(&Surface, NewSurfaceContext)`, `redraw`, `toggle_fullscreen`, `toggle_maximize`, `toggle_command_palette -> bool`, `control_inspector(Inspector) -> bool` (TODO inspector: no-op, returns true),
`redraw_inspector` (no-op), `key_sequence_action(&KeySequence)`, `key_table_action(&KeyTable)`, `show_on_screen_keyboard()`, `show_on_screen_keyboard_event(Option<&gdk::Event>)`, `set_scrollbar(Scrollbar)`,
`set_progress_report(ProgressReport)`, `present`, `command_finished(&CommandFinished) -> bool`, `get_readonly`, `set_readonly(Readonly) -> bool`, `key_event(..)`, `prompt_title`, `close`, `child_exited(&ChildExited) -> bool`,
`get_content_scale`, `get_size`, `get_cursor_pos`, `default_termio_env() -> Vec<(String,String)>`, `clipboard_request`, `set_clipboard`, `paste(&str)`, `grab_focus()` (inherent; grabs the GLArea),
`send_desktop_notification(&str,&str)`, `get_title/set_title(Option<&str>)`, `get_effective_title`, `get_title_override/set_title_override(Option<&str>)`, `copy_title_to_clipboard -> bool`, `get_pwd/set_pwd(Option<&str>)`,
`get_focused`, `get_config/set_config(&Config)`, `get_default_size -> Option<Size>`/`set_default_size(Size)`, `get_min_size/set_min_size(Size)`, `get_font_size_request`, `estimate_initial_size`,
`get/set_mouse_shape(MouseShape)`, `get/set_mouse_hidden`, `set_mouse_hover_url(Option<&str>)`, `get/set_bell_ringing`, `set_error`, `get_zoom`, `get_is_split`, `get_child_exited`,
`set_search_active(bool,&str)`, `set_search_total(Option<usize>)`, `set_search_selected(Option<usize>)`, `get/set_hadjustment`, `get/set_vadjustment`.
`RtSurfaceHandle` (Zig `Surface.zig`) implements `core::RtSurface` over a weak ref; `cgroup()` returns None (the Zig forwarder points at a method that doesn't exist).

Deviations / decisions:
- The surface registers itself in a thread-local `id -> WeakRef` map (`from_id`); core's `Target::Surface(id)` is resolved with it. The ids are given to `CoreApp::new_surface(id, ..)`; desktop-notification action target is the u64 id.
- `set_parent` takes the parent `&Surface` widget (font size via `parent.core().font_size()`, pwd via `get_pwd()`).
- `core.rs` additions (all with defaults): `CoreSurface::{readonly, grid_size, font_size, id}` (`id` currently unused). `draw_frame(0)`: GTK's GLArea FBO id is not exposed by gtk4-rs, the renderer must query `GL_FRAMEBUFFER_BINDING` itself.
- New-surface config: `apprt::surface::new_config(base, context, prev_pwd)` where prev_pwd is the pwd of another focused surface (Zig `App.focusedSurface`), looked up via the registry.
- Zig's `Clipboard.paste` for a dropped single `GFile` builds the escaped string but never pastes it; preserved (flagged in a comment).
- `gtk-xft-dpi`/primary-paste come from `crate::gsettings` (portal aware in Flatpak). Snap env filtering (`build_config.snap`) not ported. `child exited` always uses the banner overlay (adw >= 1.3).
- vadjustment value-changed uses a manual handler id (+ block/unblock) instead of `GSignalGroup` (needs glib v2_74 feature).
- Inspector (`control_inspector`, `redraw_inspector`, inspector weak ref) deferred.
- Tests: Zig `computeFraction` ported; extra tests for mouse button/shape translation. GTK-requiring behaviour is exercised via the demo.

## class/split_tree.rs and class/tab.rs (F6)

Ported: `GhosttySplitTree`, internal `GhosttySplitTreeSplit` (split_tree.zig) and `GhosttyTab` (tab.zig), templates ui/1.5/{split-tree,split-tree-split,tab}.

Tree model: `split_tree::Tree = ghostty_datastruct::SplitTree<Surface>`; `impl SplitTreeView for Surface` lives in split_tree.rs;
`SurfaceTree(pub Tree)` is the `glib::Boxed` (`GhosttySurfaceTree`, nullable) used for the `tree`/`surface-tree` properties and the `changed(old,new)` signal.
Deviation: Zig `getTree()` returns a mutable pointer; Rust `get_tree()` returns a clone (views are ref-counted GObjects). In-place mutation (zoom, live split resize) is done internally (RefCell); external changes use `set_tree(Some(&new_tree))`.

SplitTree public API: `new()`, `new_split(direction, parent: Option<&Surface>, SurfaceOverrides) -> Result<(), SplitError>`, `resize(direction, amount: u16) -> Result<bool, SplitError>`, `goto(Goto) -> bool`, `get_needs_confirm_quit()`, `get_active_surface()`, `get_last_focused_surface()`, `get_has_surfaces()`, `get_is_zoomed()`, `get_is_split()`, `get_tree()`, `set_tree(Option<&Tree>)`.
Properties: `active-surface`(ro), `has-surfaces`(ro), `is-zoomed`(ro), `tree`(rw boxed), `is-split`(ro). Signal: `changed`. Actions (group `split-tree`): `new-split(s)`, `equalize`, `zoom`.
Surface close: connects each surface's `close-request` / `notify::focused` via generic signal API; shows `CloseConfirmationDialog` when `core.needs_confirm_quit()`.

Tab public API: `Tab::new(Option<&Config>, SurfaceOverrides)`, `set_parent(&Surface)`, `set_parent_with_context(&Surface, ctx)`, `set_title_override(Option<&str>)`, `get_title_override()`, `get_title()`, `prompt_tab_title()`, `get_active_surface()`, `get_surface_tree()`, `get_split_tree()`, `get_config()`, `get_needs_confirm_quit()`, `connect_close_request(f)`.
Properties: `active-surface`(ro), `config`, `split-tree`(ro), `surface-tree`(ro), `tooltip`, `title`, `title-override`. Signal `close-request`. Actions (group `tab`): `close(s: this|other|right)`, `ring-bell`, `next-page`, `previous-page`, `prompt-tab-title`.
Differences: Tab::set_parent takes the parent `&Surface` (not a core surface) to match `Surface::set_parent`. Template callbacks that Zig bound with the tab as user-data (`notify_active_surface`, `notify_tree`) find the tab through `ancestor()` of the emitting SplitTree. The computed-title closure is a `#[template_callback(function)]` (not yet runtime-verified in a live window).
No Zig tests exist for these files.
