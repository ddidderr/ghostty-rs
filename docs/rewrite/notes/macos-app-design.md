# macOS app (Rust/AppKit port of `macos/Sources`): design

Crate `crates/ghostty-macos` (lib `ghostty_macos` + bin `ghostty-macos-app`). Written by MA1 (foundation); the
feature areas are ported by MA2..MA5 on top of it (split at the end). Nothing here has ever run on a Mac: it is
compile-checked with `scripts/macos-crosscheck.sh` (`just check-macos`, both Apple targets, `-D warnings`).

## Principles

* **Same API surface as the Swift app.** The Swift app is a client of `libghostty` through `ghostty.h`. This app calls the
  *same entry points* in-process: `ghostty-lib` is a normal dependency (renamed `libghostty`) and `ghostty::capi`
  re-exports its `extern "C"` functions (`ghostty_app_new`, `ghostty_surface_key`, ...), `#[repr(C)]` types and the header's
  enumerators (`capi::consts`, generated from `ghostty.h` by `scripts/gen-macos-capi-consts.py`). The runtime config
  (`wakeup`, `action`, `read/confirm_read/write_clipboard`, `close_surface` callbacks, userdata) is built exactly like
  `Ghostty.App.init`. libghostty is not embedded or linked as a separate library: no XCFramework needed.
* **Programmatic AppKit.** No `ibtool`/Xcode on Linux, so xibs/SwiftUI become code (below). The crate compiles to an empty
  stub off Apple; platform independent logic is *not* gated, so it is unit-tested on Linux.
* **Faithful port.** Swift logic is ported 1:1 (same names in snake_case, doc comments kept). Deviations: `DEVIANT_FIXES.md`
  ("Other deliberate deviations") and `macos-app-MA1.md`.

## Module tree (mirrors `Sources/`)

| Swift | Rust (`crates/ghostty-macos/src`) |
|---|---|
| `App/macOS/main.swift` | `app/entry.rs` (`run()`, called by `main.rs`) |
| `App/macOS/AppDelegate.swift`, `AppDelegate+Ghostty.swift` | `app/app_delegate.rs`, `app/app_delegate_ghostty.rs` |
| `App/macOS/MainMenu.xib` | `app/menu_spec/` (generated tables + pure logic) and `app/main_menu.rs` (builds the `NSMenu`s) |
| `Ghostty/Ghostty.App.swift` | `ghostty/app.rs` (`App`) |
| `Ghostty/Ghostty.Config*.swift`, `Ghostty.Command/Action/Error/Event/Shell`, `GhosttyPackage*.swift` | `ghostty/{config,config_types,command,action,error,event,shell,package,notification}.rs` |
| `Ghostty/Ghostty.Surface.swift`, `Surface View/*` | `ghostty/{surface,surface_view,surface_configuration}.rs` (+ MA2) |
| `Ghostty/Ghostty.Input.swift`, `NSEvent+Extension.swift` | `ghostty/input/`, `ghostty/ns_event.rs` |
| `Helpers/**` | `helpers/**` (extensions: `helpers/extensions/<ns_type>.rs`, one extension trait `XExt` per Swift extension) |
| `Features/<Area>/**` | `features/<area>/**` |

Platform independent modules (tested on Linux): `ghostty::{capi, config, config_types, command, action, shell, input,
notification, package, error, surface_configuration}`, `helpers::{keyboard_shortcut, color, notification_center, published,
...}`, `app::menu_spec`, `features::quick_terminal::{position,size,screen,space_behavior}`.

## Swift type -> Rust type

| Swift | Rust |
|---|---|
| `class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate` | `define_class!` `AppDelegate` (`#[thread_kind = MainThreadOnly]`); Swift stored properties live in the `Ivars` struct (`RefCell`/`Cell`); `@IBAction`/delegate methods are `#[unsafe(method(sel:))]`s with the same selectors |
| `@IBOutlet menuXxx` | `menu_spec::Outlet` enum + `HashMap<Outlet, Retained<NSMenuItem>>` filled by `main_menu::install` |
| `Ghostty.App` (`ObservableObject`) | `Rc<ghostty::app::App>`; `@Published` -> `helpers::published::Published<T>` |
| `Ghostty.Config` (owns `ghostty_config_t`) | `ghostty::config::Config` (`Drop` frees; shared as `Rc<Config>`) |
| `Ghostty.SurfaceView: NSView` | `define_class!` `SurfaceView` (`ghostty/surface_view.rs`; MA2) |
| `Ghostty.Surface` | `ghostty::surface::Surface` |
| `BaseTerminalController/TerminalController: NSWindowController` | `define_class!` classes in `features/terminal` (MA3) |
| `Weak<T>` | `objc2::rc::Weak<T>` (ObjC objects) / `std::rc::Weak` |
| Swift enums with raw strings (`init?(rawValue:)`) | Rust enums with `from_raw_value(&str)` |
| `OptionSet` | `bitflags!` |
| `SwiftUI.Color`, `KeyboardShortcut`, `KeyEquivalent`, `EventModifiers` | `helpers::{color::Color, keyboard_shortcut::*}` (same raw values, so menu key equivalents are identical) |
| `Duration` | `std::time::Duration` |
| `UserDefaults.ghostty` | `helpers::extensions::user_defaults::ghostty()` |
| `Logger` (OSLog) | `log` crate (`ghostty_init` installs the os_log logger like Zig's lib mode) |

## Threading

Everything that touches AppKit/Rust-side state runs on the **main thread** (the Swift code is `@MainActor`/main queue):
objects are `!Send` (`Rc`, `Retained<..>` of main-thread classes), `MainThreadMarker`s come from `helpers::main_thread()`.
The one cross-thread entry from libghostty is the `wakeup` callback: it only forwards the address of the `App` allocation to
`DispatchQueue.main.async`, where it is resolved through a main-thread registry of live apps and `ghostty_app_tick` runs
(`ghostty/app.rs`). UNUserNotificationCenter callbacks arrive on arbitrary queues: they only capture `Send` data and hop to
the main queue (`main_async_local` for closures with main-only captures; `AppDelegate::shared(mtm)` re-resolves the delegate).
`DispatchQueue.main.async` -> `dispatch2::DispatchQueue::main().exec_async`, `asyncAfter` -> `exec_after`, signal sources
(`SIGUSR2` config reload) -> `dispatch2::DispatchSource` with `_dispatch_source_type_signal`.

## Notifications, observers, Combine

* AppKit/Foundation notifications (`NSWindow.didBecomeKeyNotification`, ...): block based observers
  (`helpers::extensions::ns_notification::observe`) returning an RAII token (dropping it = `removeObserver`).
* Ghostty's own notifications (`Notification.Name.ghosttyConfigDidChange`, `Ghostty.Notification.ghosttyNewWindow`, ...):
  `helpers::notification_center::NotificationCenter` (names/keys with the same strings in `ghostty::notification`). Payloads stay
  typed: `UserInfo` is a string-keyed map of `Rc<dyn Any>`, the sender (`object`) is a `Sender` (identity + downcastable handle,
  e.g. `Retained<SurfaceView>`). Delivery is synchronous on the main thread, observer semantics follow Foundation (filter by
  sender, tokens, snapshot on post). `Ghostty.App` posts these exactly where the Swift code does.
* Combine `$property.sink` / `@Published` / `ObservableObject`: `Published<T>` (`get/set/update/sink`, `Subscription` = cancellable).
* KVO (`NSApplication.observe(\.effectiveAppearance)`): the delegate registers itself via `addObserver:forKeyPath:` and
  implements `observeValueForKeyPath:ofObject:change:context:`.
* Distributed notifications (icon change): `NSDistributedNotificationCenter` directly.

## SwiftUI views and xibs

* **xib** (`MainMenu.xib`, `About.xib`, `ConfigurationErrors.xib`, `ClipboardConfirmation.xib`, `QuickTerminal.xib`,
  `Terminal*.xib`): reproduced in code. For the main menu the xib is *parsed by a script*
  (`scripts/gen-macos-menu-spec.py` -> `app/menu_spec/generated.rs`: titles, key equivalents, modifier masks, selectors, targets,
  outlets, system menus), and `main_menu.rs` instantiates the objects; a unit test pins the structure. Window xibs are small
  (a window, style mask, content rect, connections): port them by hand as `NSWindowController::init` code with the same style
  mask/frame/flags (read the XML; `scripts/gen-macos-menu-spec.py` shows how to parse it).
* **SwiftUI views** (`AboutView`, `ConfigurationErrorsView`, `ClipboardConfirmationView`, update pill/popover, `TerminalView`,
  split views, command palette, `SurfaceView` overlays): rebuilt with AppKit views (`NSStackView`/Auto Layout, `NSTextField`,
  `NSVisualEffectView`, `NSScrollView`, `NSPopover`, `NSHostingView` is *not* available). `@State`/`@Published` become
  `Published<T>` subscriptions that update the views; `EnvironmentObject` becomes explicit references. Visual fidelity target:
  same layout, text, spacing and behaviour as far as AppKit allows; record gaps in the owner's notes.
* **Unportable SwiftUI-only helpers** (no AppKit equivalent, not ported): `Backport` (view modifiers), `HostingWindow`,
  `NonDraggableHostingView`, `MetalView`/`MTKView` wrapper, `View+Extension`, `Transferable+Extension`/drag-and-drop via
  `Transferable` (the AppKit `NSDraggingSource/Destination` equivalent is implemented by MA2/MA3 where needed).

## Menu bar

`menu_spec` holds `MAIN_MENU` (generated from the xib), `Outlet`, `SYNCED_SHORTCUTS` (the `syncMenuShortcut(config, action:,
menuItem:)` list), `MENU_IMAGES` (SF Symbols), `DOCK_MENU` and `MenuShortcutKey` (the shortcut lookup key). `AppDelegate` ports
`syncMenuShortcuts`, `performGhosttyBindingMenuKeyEquivalent`, `validateMenuItem`, the dock menu and float-on-top handling.
Upstream quirk kept: the xib connects "Use Selection for Find" to a non-existent outlet `menuSelectionForSearch` and "Jump to
Selection" to none, so those two items never get a config shortcut (see the `menu_spec` module docs).

## Bundle (`scripts/macos-bundle.sh`)

Builds `ghostty-macos-app`, creates `Ghostty.app` (executable `ghostty`), expands `Ghostty-Info.plist` plus the Xcode
`INFOPLIST_KEY_*` settings with PlistBuddy, copies `Ghostty.sdef`, builds `AppIcon.icns` from `Assets.xcassets` with
`sips`/`iconutil` and copies the alternate/custom icon PNGs and `ResetZoom.pdf`, installs shell-integration/themes/terminfo where
`ghostty_os::resources_dir()` looks on macOS (`Contents/Resources/ghostty`, `Contents/Resources/terminfo/78/xterm-ghostty`),
signs with `Ghostty.entitlements` (`GhosttyDebug.entitlements` with `--debug`). Runs for real only on macOS; elsewhere it is a dry
run that prints every step. `NSMainNibFile` is not set (menu built in code); the DockTilePlugin is a separate bundle (MA4,
`--dock-tile-plugin`); Sparkle.framework would be copied to `Contents/Frameworks` by MA5.

## Assessments

* **App Intents** (`Features/App/Intents`, `Ghostty.Input.*: AppEnum`): the AppIntents framework is Swift-only. Intents,
  entities and enums are Swift types conforming to generic Swift protocols (`AppIntent`, `AppEntity`, `AppEnum`,
  `EntityQuery`, `@Parameter` property wrappers); the system discovers them from `Metadata.appintents` that the *Swift compiler
  toolchain* extracts (`appintentsmetadataprocessor`) from the module. There is no Objective-C surface and no runtime registration
  API, so a Rust binary cannot declare intents. Verdict: **unportable from Rust** (would need a Swift shim, i.e. not a Rust port);
  MA5 documents it, ports the non-framework logic that is useful elsewhere (e.g. the `Ghostty.Input.Action/MouseButton/...`
  enums, already needed by AppleScript) and records the Shortcuts/Siri loss in DEVIANT_FIXES.
* **Sparkle** (`Features/Update`): `Sparkle.framework` is Objective-C; load it at runtime (`NSBundle(path:).load()` from
  `Contents/Frameworks`), look classes/protocols up with `AnyClass::get`/`AnyProtocol::get`, create `SPUStandardUpdaterController`-style
  objects with `msg_send!`, and declare the `SPUUserDriver` / `SPUUpdaterDelegate` implementations with
  `objc2::runtime::ClassBuilder` + `add_protocol` (a `define_class!` cannot name a protocol that is not linked at build time).
  Weak linking = the lookup returns `None` when the framework is absent. `UpdateController`'s surface used by `AppDelegate` is
  already in place (`start_updater`, `check_for_updates`, `is_installing`, the two automatic-update flags).
* **AppleScript** (`Features/AppleScript`): `Ghostty.sdef` + `OSAScriptingDefinition` in Info.plist (handled by the bundle
  script); Cocoa scripting needs `NSObject` subclasses whose KVC keys match the sdef (`define_class!` with property getters
  as methods named after the keys), `NSScriptCommand` subclasses (`performDefaultImplementation`), and the `NSApplication`
  scripting hooks (`application:delegateHandlesKey:` etc.) on the delegate.

## Work split

| Agent | Area (Swift) | Notes |
|---|---|---|
| **MA2** | `Ghostty/Surface View/*` (SurfaceView AppKit: rendering host, input, IME, mouse, drag source, scroll view, progress bar, grab handle, search overlay, inspector view), rest of `Ghostty.Input.swift` (KeyEvent, Mouse*, Mods, Key enum), `Ghostty.Inspector`, `GhosttyPackage` remaining pieces | fills the `TODO(MA2)` in `ghostty/surface_view.rs`; the model fields App.swift needs already exist |
| **MA3** | `Features/Terminal/*` (BaseTerminalController, TerminalController, TerminalView, windows + `Styles/*.xib` variants, restorable state, tab color), `Features/Splits/*` (SplitTree, split views), Helpers `TabTitleEditor`, `TabGroupCloseCoordinator` | replaces the stubs in `features/terminal/mod.rs` (API already used by `AppDelegate`/`Ghostty.App`) |
| **MA4** | `Features/QuickTerminal`, `Command Palette`, `Settings` (ConfigurationErrors window + SettingsView), `About`, `ClipboardConfirmation`, `Global Keybinds` (event tap, Helpers `PermissionRequest`), `Secure Input` (+overlay), `Services` (ServiceProvider), `Custom App Icon` (AppIcon, ColorizedGhosttyIcon, DockTilePlugin, UserDefaults+AppIcon) | replace the surface stubs in each `features/<area>/mod.rs`; pure value types of quick terminal are already ported |
| **MA5** | `Features/AppleScript` (sdef + scripting classes), `Features/Update` (Sparkle via the ObjC runtime, weak-linked), `Features/App` App Intents (assess: unportable, see above) | |

Each owner: port tests of `macos/Tests/*` for the area (pure logic portable and tested on Linux, AppKit logic compile-checked),
write `docs/rewrite/notes/macos-app-<MAn>.md`, keep `just check-macos` green, list deviations.
