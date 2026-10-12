# Known differences

All behavioural differences found by the audits (RV1–RV18) have been fixed;
intentional deviations (upstream bugs fixed rather than replicated) are listed
in DEVIANT_FIXES.md. No known unfixed behavioural differences remain.

What remains is verification coverage that this Linux machine cannot provide:

## Not verifiable on this machine

- Native Apple execution: Metal, IOSurface, CoreText, Apple input APIs, Mach handling and Apple packaging (compile-checked only, `just check-macos`).
- Native Windows (real Windows console host, Windows-only APIs Wine stubs: `ResizePseudoConsole` is a Wine stub, POSIX-semantics rename over an existing file is unsupported by Wine). The Windows code paths are otherwise run under Wine (`just test-windows-wine`, see `notes/windows-wine.md`): ConPTY, `CreateProcessW`, pipes, the IO thread and `cmd.exe` children are runtime-tested; `just check-windows` stays the clippy gate.
- Native FreeBSD execution (cross `cargo check` only, if the target could be added).
- Live portal / D-Bus integration.
- Real-browser rendering (wasm is checked with Node only).
- Native io_uring execution; Swift code was excluded.
