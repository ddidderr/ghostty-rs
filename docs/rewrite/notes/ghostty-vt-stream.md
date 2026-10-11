# ghostty-vt stream notes

## Ported
- terminal/stream.zig -> `crates/ghostty-vt/src/stream.rs` (Action, Stream, CSI/ESC/OSC/C0/C1 dispatch, 37 of 38 Zig tests as `#[test]`, plus 8 extra).
- simd/index_of.zig, simd/vt.zig -> `src/simd.rs` (scalar; `memchr` dependency). 1 + 6 Zig tests ported.
  Not ported: simd/codepoint_width (belongs to ghostty-unicode), simd/base64 (graphics_command already uses the `base64` crate).
- Small edits to W1 files: `parser.rs` (`Esc::new`, parser param fields `pub(crate)` for the CSI fast path), `utf8_decoder.rs` (`in_sequence()`), `lib.rs`.
- Skipped test: `test Action` (C ABI reification). `Action.C`/`cval`/`lib.TaggedUnion` not ported (C ABI).

## API overview (for handler implementors)
```rust
use ghostty_vt::stream::{Action, ActionKey, Handler, Stream};

impl Handler for MyHandler {
    type Error = MyError;                       // Zig `!void` error set (Debug)
    fn vt(&mut self, action: Action<'_>) -> Result<(), Self::Error> {
        match action {
            Action::Print(p) => ...,            // p.cp: u32
            Action::CursorUp(m) => ...,         // m.value: u16
            Action::SetMode(m) => ...,          // m.mode: modes::Mode
            _ => {}
        }
        Ok(())
    }
    // optional: fn vt_raw(&mut self, &parser::Action) -> Result<bool, Error>  (Zig vtRaw; true = skip)
}
let mut s = Stream::new_alloc(handler);   // sets osc_parser.alloc = true (Zig initAlloc)
s.next_slice(bytes)?;                     // fast path; s.next(byte)? is scalar
s.handler / s.parser / s.utf8decoder      // public fields
```
- `Stream::new` = Zig `init` (alloc flag false: OSC 4/5/10-19/104/110-119/133/21/52-style commands needing alloc are rejected). Real terminals should use `new_alloc`.
- Variant names are CamelCase of the Zig tag (`.cursor_up` -> `Action::CursorUp`, `.erase_display_below` -> `EraseDisplayBelow(bool protected)`, `.kitty_keyboard_set_or` -> `KittyKeyboardSetOr`). `Action::key() -> ActionKey` is Zig `Action.Key`.
- Payload struct names/fields match Zig (`CursorMovement{value}`, `CursorPos{row,col}`, `Margin{top_left,bottom_right}`, `Mode{mode}`, `RawMode{mode,ansi}`, `InvokeCharset{bank,charset,locking}`, `ConfigureCharset{slot,charset}`, ...).
  Count-style payloads that are `usize` in Zig (`PrintRepeat`, `InsertBlanks`, `DeleteChars`, `EraseChars`, `InsertLines`, `DeleteLines`, `ScrollUp/Down`) are `usize`; `HorizontalTab(u16)`, `HorizontalTabBack(u16)`, `TitlePush/Pop(u16)`, `KittyKeyboardPop(u16)`.
- Payloads borrow from the decoded OSC command (`Action<'a>`): `WindowTitle{title:&str}` (validated UTF-8), `ReportPwd{url:&[u8]}`, `ShowDesktopNotification{title,body}`, `StartHyperlink{uri, id:Option<&[u8]>}`, `ClipboardContents{kind,data}`, `ColorOperation{op, requests:&List, terminator}`, `KittyColorReport(&kitty::color::OSC)`, `SemanticPrompt(&osc::SemanticPrompt)`, `SetAttribute(sgr::Attribute<'a>)`. Copy what you need to keep. `DcsHook(parser::Dcs)` is owned/Copy; `DcsPut/ApcPut` get single bytes.
- `ProgressReport(osc::ProgressReport)` and `MouseShape(MouseShape)` by value.
- Errors returned from `vt` abort the current `next`/`next_slice` call and propagate (Zig `try`). Errors from `vt_raw` are logged and the action skipped (as in Zig).
- `impl Handler for &mut H` exists, so `Stream::new(&mut handler)` works (Zig pointer handlers). Zig `deinit` = `Drop`.
- Mirror `terminal/stream_readonly.zig` and `termio/stream_handler.zig` with a single `match action { ... }`; unhandled variants fall into `_ => {}`.

## Behavior notes / deviations
- Logging uses the `log` crate with the same messages/levels as Zig.
- `next_slice` processes input in 4096-byte chunks like Zig; ESC is located with `memchr`; ground-state text is decoded in bulk (`simd::utf8_decode_until_control_seq`); a stray/partial byte is handed to the scalar DFA decoder and decoding of the rest of the chunk then resumes on the fast path (Zig fed the entire remainder through the scalar path, which would also run `next_utf8` outside the ground state if an ESC followed; same output otherwise).
- Codepoints 0x00-0x0F are executed, all other codepoints (including 0x10-0x1F, 0x7F, C1 from UTF-8) are printed, exactly like Zig's SIMD/scalar paths. Raw C1 bytes reach `execute` only via the parser and are mapped to `ESC c-0x40`.
- CSI W: Zig logs a spurious "invalid cursor tabulation control" after successfully handling `CSI 2 W` / `CSI 5 W`; the Rust port does not log in that case (log-only difference).
- CSI K accepts params 0..2 only, so `EraseLineRightUnlessPendingWrap` (4) is unreachable from CSI, as in Zig. DECRQM only handles `CSI ? Ps $ p` (ANSI `CSI Ps $ p` is "unimplemented", as in Zig).
- Test handlers use a small closure-based `F<T, C>` helper instead of per-test handler structs. Zig's `Stream(H).init(.{})` tests use `Stream::new`.
