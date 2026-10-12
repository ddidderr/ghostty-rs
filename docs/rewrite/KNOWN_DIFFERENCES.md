# Known differences (left by the stopping criterion)

All previously listed differences (rows 5-10: theme-search word navigation,
grapheme backspace, ASCII-only case folding, initial preview colors, load_file
error propagation, subprocess cleanup wait) have been fixed; see
`notes/final-P3.md` (row 10 is registered as a deviant fix in
`DEVIANT_FIXES.md`). No known differences are currently recorded here.

## Not verifiable on this machine

- Native Apple execution: Metal, IOSurface, CoreText, Apple input APIs, Mach handling and Apple packaging (compile-checked only, `just check-macos`).
- Native Windows (real Windows console host, Windows-only APIs Wine stubs: `ResizePseudoConsole` is a Wine stub, POSIX-semantics rename over an existing file is unsupported by Wine). The Windows code paths are otherwise run under Wine (`just test-windows-wine`, see `notes/windows-wine.md`): ConPTY, `CreateProcessW`, pipes, the IO thread and `cmd.exe` children are runtime-tested; `just check-windows` stays the clippy gate.
- Native FreeBSD execution (cross `cargo check` only, if the target could be added).
- Live portal / D-Bus integration.
- Real-browser rendering (wasm is checked with Node only).
- Native io_uring execution; Swift code was excluded.
