# Live D-Bus / portal integration (agent LIVE)

Harness: `scripts/live-dbus-test.sh` (`just live-dbus [scenario...]`), helpers in `scripts/live-dbus/`
(`scenario.sh`, `scenarios.sh`, `mock.py`, `normalize.py`, child scripts, `golden/`, `build-rust.sh`).
Automated gate: `tests/live_dbus.rs` (golden mode, see below).

## Method
- Every scenario runs once per binary (original `/usr/bin/ghostty` = oracle, Rust release-profile build) on a
  **private session bus**: `dbus-run-session --config-file=...` with its own `servicedir` (only
  `com.mitchellh.ghostty.service` pointing at the binary under test, so D-Bus activation starts the build under test, not
  `/usr/bin/ghostty`), inside a private headless weston (as `visual-parity.sh`), private HOME/XDG dirs,
  `GSETTINGS_BACKEND=memory`. The user's bus, portal and desktop are never touched. All spawned processes carry an
  `LDB_TAG` env marker and are swept at the end.
- Mock services (`mock.py`, Gio via PyGObject) on the private bus: `org.freedesktop.Notifications`,
  `org.freedesktop.systemd1` (Manager.StartTransientUnit, switchable to failing), `org.freedesktop.portal.Desktop`
  (GlobalShortcuts incl. Request/Session objects, Settings, OpenURI). Control interface `test.ghostty.Mock`
  (`Activate`, `SetColorScheme`, `Invoke`, `SetFail`). Every received call is logged as JSON.
- `busctl --user monitor --json=short` records all traffic; `normalize.py` keeps `org.gtk.*`, Notifications, systemd1 and
  portal messages and normalizes pids, tokens, unique names, paths. Observable effects ("facts") come from the exported
  GtkApplication window objects (`busctl tree`), process counts, child cwd/argv, and the pty (colour-scheme reports).
- `golden/<scenario>.txt` = the original's facts + received mock calls (recorded with `UPDATE_GOLDEN=1`). With the
  original installed the harness compares original vs Rust directly **and** Rust vs golden; without it (CI,
  `tests/live_dbus.rs`, `LABELS=rust`) only Rust vs golden. Debug builds use the app id `com.mitchellh.ghostty-debug`
  (as Zig), so the Rust build must be the release profile (`scripts/live-dbus/build-rust.sh`).
- Real `xdg-desktop-portal` + `xdg-desktop-portal-gtk` do run on the private bus (scenario `real-portal`, keyfile
  gsettings backend so `color-scheme` can be changed live). They cannot give Ghostty an app id without systemd/flatpak
  ("An app id is required"), so GlobalShortcuts against the real frontend only yields `NotAllowed`; the BindShortcuts
  payload is therefore checked against the mock.

## Scenarios: original vs Rust
| Scenario | Covers | Result |
|---|---|---|
| single-instance | first launch owns `com.mitchellh.ghostty`; `+new-window` (plain, `-e`, `--working-directory`, both, wrong/bad `--class`), second GUI launch forwarded, `-e` launch is never single-instance, `--class=com.example.custom-app` own bus name/path and `+new-window --class`; exact `org.gtk.Actions.Activate new-window-command` payloads (the CLI prepends `--working-directory=<cwd>`), exported actions, window counts, child cwd/argv | identical |
| single-instance-detect | `gtk-single-instance=detect`: plain launch = single instance, `TERM_PROGRAM` set or any CLI argument = dedicated instance | identical |
| notifications | OSC 9, OSC 777 (title/body/empty parts/`;` in body/markup/non-ASCII), repeated body (replaces id), `ActionInvoked default` incl. unknown id, `desktop-notifications=false` after reload; exact `Notify(app, replaces_id, icon, summary, body, actions, hints, timeout)` | identical (app name `""`, summary `Ghostty` for OSC 9, actions `[default,""]`, hints `desktop-entry`, `image-path`, `urgency=1`) |
| shortcuts | `global:` keybinds -> `CreateSession`/`BindShortcuts` payload (ids, descriptions, order), `Activated` -> action, unknown id ignored, session `Close` + re-create on reload, none without global binds | **diverged, fixed** (see below) |
| settings | portal `org.freedesktop.appearance color-scheme` (Read/ReadAll/SettingChanged) -> CSI ? 997 reports (mode 2031), `theme = light:..,dark:..` OSC 11 colour, `window-theme=dark` | identical |
| cgroup | `linux-cgroup` default/always/never, memory/processes limits (`MemoryHigh`,`TasksMax`,`ManagedOOMMemoryPressure`,`PIDs`), failing StartTransientUnit (app keeps running) | identical |
| openuri | `open-config` -> `xdg-open <config>`; **no** portal OpenURI/OpenFile call (upstream TODO in `Application.openUrl`; the Rust port has the same TODO) | identical |
| openurl-click | ctrl+click on plain URL / OSC 8 link / `file://` / path -> `xdg-open` argv | Rust only (the original cannot be driven: no input injection, Xwayland pointer is not reachable in headless weston); golden derived from `Surface.zig`/`os/open.zig` |
| real-portal | real xdg-desktop-portal + gtk backend: Settings color-scheme live changes, GlobalShortcuts error path, Inhibit monitor | identical |

## Divergences found and fixed (Rust port bugs, not upstream bugs: normal commits)
1. **GlobalShortcuts payloads had the wrong D-Bus signature** (`crates/ghostty-apprt-gtk/src/class/global_shortcuts.rs`):
   `CreateSession` was sent as `(v)` instead of `(a{sv})`, option values were double-boxed (`v` in `v`), and
   `BindShortcuts` was `(oa(sv)sv)` instead of `(oa(sa{sv})sa{sv})`. A real portal rejects these, so global shortcuts could
   never work. Now built with `tuple_from_iter`/typed arrays by `create_session_payload`/`bind_shortcuts_payload`;
   unit tests assert the signatures.
2. **CreateSession was sent twice at startup** (`class/application.rs`): the app's `config` notify forwarded the config to
   `GlobalShortcuts` before `startup_global_shortcuts`; Zig binds the property only after `setDbusConnection`
   (`bindProperty(.., .sync_create)`), so the session is created once. New `global_shortcuts_bound` flag.

No upstream (deviant) bugs were found in this area; `DEVIANT_FIXES.md` is unchanged by this work.

## Not covered
- Input method (ibus/fcitx not installed) stays in "Not verifiable on this machine".
- The original cannot receive pointer input here, so link clicks are checked for the Rust build only.
- `notify-on-command-finished` never produced a notification for either binary in the headless setup (identical, no
  focus change events), so only OSC 9/777 are compared.
- Real GlobalShortcuts backends (GNOME/KDE) are not installed; the real frontend answers `NotAllowed` for non-sandboxed
  apps without systemd-assigned app ids.
