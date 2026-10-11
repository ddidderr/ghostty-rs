# ghostty-terminal: search/* and tmux/* (agent T5)

## Ported
| Zig | Rust | tests (ported/Zig) |
|---|---|---|
| search.zig | `search/mod.rs` | - |
| search/sliding_window.zig | `search/sliding_window.rs` | 25/25 |
| search/active.zig | `search/active.rs` | 2/2 |
| search/pagelist.zig | `search/pagelist.rs` | 7/7 |
| search/viewport.zig | `search/viewport.rs` | 4/4 |
| search/screen.zig | `search/screen.rs` | 11/13 |
| search/Thread.zig | `search/thread.rs` | 1/1 (unnamed `test {}` -> `search_thread_basic`) + 1 extra (`search_thread_select_and_state`) |
| tmux.zig | `tmux/mod.rs` (re-exports `ghostty_vt::tmux::control`) | - |
| tmux/layout.zig | `tmux/layout.rs` | 43 (24 layout + 19 checksum) |
| tmux/output.zig | `tmux/output.rs` | 37 |
| tmux/viewer.zig | `tmux/viewer.rs` | 8/8 |

Skipped (Zig-only): the two `reloadActive partial history cleanup on ... error` tests in search/screen.zig
(tripwire allocator-failure injection). output.zig comptime-format variants are covered by the runtime `format()` tests.

## Search design / deviations
- Uses `crate::highlight::{Flattened, Untracked, Tracked, Chunk}` and `crate::formatter::{PageFormatter, Options}`
  (plain + `unwrap` with a point map) from T4. `NodeId` carries the node, `Chunk.serial` is the node serial.
- Nothing stores `*Screen` / `*PageList`. Everything touching tracked pins takes the screen explicitly:
  `ScreenSearch::new(&mut Screen, needle)`, `.deinit(&mut Screen)` (must be called to untrack pins),
  `.feed(&mut Screen)`, `.reload_active(&mut Screen)`, `.select(&mut Screen, Select)`, `.search_all(&mut Screen)`.
  `tick()`, `matches()`, `selected_match()`, `matches_len()` need no screen. `PageListSearch::{new,feed,deinit}` take `&mut PageList`.
  `ActiveSearch::update(&PageList) -> Option<NodeId>`, `ViewportSearch::update(&PageList) -> bool` (infallible).
  `SlidingWindow::append(&PageList, NodeId) -> usize`.
- `SlidingWindow` uses `VecDeque` (made contiguous for searching) instead of `CircBuf` + overlap buffer; results are identical.
  The Zig assertions about circular-buffer slice layout in the "across circular buffer boundary" tests are dropped (behaviour kept).
  Row counts are cached per page at `append` time. Needle must be non-empty (Zig has the same underflow).
  `next()` returns an owned `Flattened`, so Zig's `clone()` dance is unnecessary.
- `ScreenSearch` caches `HistorySearch.start_node` (Zig reads `start_pin.node` lock-free in `tick`); refreshed in `feed`/`reload_active`.
  Screen identity (`screen_id`, address) replaces Zig's `*Screen` pointer comparison for "screen was reinitialized" in the thread.
- `Thread`: libxev replaced by crossbeam-channel: bounded(64) mailbox + control channel (wakeup/stop) + 24 ms deadline refresh timer.
  API: `Thread::new(Options)`, `.handle() -> Handle {send,try_send,wakeup,stop}`, `.thread_main()` or `Thread::spawn(opts) -> (Handle, JoinHandle)`.
  `Options { terminal: Arc<Mutex<Terminal>>, event_cb: Option<Arc<dyn Fn(Event)+Send+Sync>> }`;
  `Message::{ChangeNeedle(Vec<u8>), Select(Select)}` (Zig `MessageData(u8,255)` -> `Vec<u8>`); `Event` carries owned data
  (`ViewportMatches(Vec<Flattened>)`). The search is torn down under the lock (pins untracked) when the thread exits or the needle changes.
  Dependency added: `crossbeam-channel`.
- Terminal hook already present: `Terminal.flags.search_viewport_dirty` (the thread reads/clears it).
- `cargo fmt -p ghostty-terminal` was run for the whole crate (not just my files).

## tmux design / deviations
- `ghostty_vt::tmux::control` is reused (`tmux::control`, plus `ControlParser`/`ControlNotification` aliases).
- output.rs: comptime `FormatStruct` -> `Variable` enum + `FormatValues` (`.bool()/.usize()/.str()`); `Variable::parse` keeps
  Zig's `InvalidCharacter`/`Overflow`/`FormatError` distinction; `parse_format_struct` folds into `ParseError::FormatError`.
- layout.rs: `Layout` owns children (`Vec`), `ParseError::{SyntaxError, ChecksumMismatch}`.
- viewer.rs: `Viewer::new()`, `next(Input) -> Vec<Action>` (owned; no arena); `Action::{Command(Vec<u8>), Windows(Vec<..> clone), Exit}`;
  public `session_id`, `tmux_version`, `windows`, `panes` (insertion-ordered `PanesMap`). `sync_layouts` creates terminals before
  committing; any command-processing failure goes defunct like Zig. `Notification::Enter` must NOT be fed to the viewer (unreachable, as Zig).
  Unknown extra `Notification` variants are ignored.

## TODO(integration) - tmux viewer wiring (nothing in terminal.rs is required)
1. In the DCS state kept by `ghostty_vt::dcs`/stream, on DCS `1000p` (`Notification::Enter`) create `Viewer::new()` (do not pass Enter to it).
2. Feed every later `ghostty_vt::tmux::control::Parser` notification via `viewer.next(Input::Tmux(n))`.
3. Execute returned actions in termio: `Command(bytes)` -> write to pty; `Windows(w)` -> diff in apprt; `Exit` -> drop viewer, leave control mode.
4. Pane contents are in `viewer.panes.get(id).terminal`; renderer/apprt render from there.

## TODO(integration) - search wiring
- Core owns the search thread: keep `Handle`; on search-start/changed text send `Message::ChangeNeedle`, navigation sends `Message::Select`.
  Events arrive on the search thread via `event_cb` (forward to the apprt main loop). The renderer must set
  `terminal.flags.search_viewport_dirty = true` when it finds viewport/active dirty (as Zig), so the thread re-searches.
