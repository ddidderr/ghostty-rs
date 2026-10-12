# Z15: RV12 P3 closures

## 1. Platform error codes (`ghostty-os/src/zig_error.rs`)
`errno_name` is now per target: unix uses `libc::E*` constants (Linux and Darwin/BSD numbering both
correct, e.g. Darwin `ELOOP=62`, `ENAMETOOLONG=63`, `EAGAIN=35`); Windows maps Win32 codes the way
Zig std's `windowsCreateProcess` / file-open paths do (FILE_NOT_FOUND, PATH_NOT_FOUND, DIRECTORY ->
FileNotFound; ACCESS_DENIED -> AccessDenied; INVALID_NAME, FILENAME_EXCED_RANGE -> NameTooLong;
SHARING_VIOLATION -> FileBusy; COMMITMENT_LIMIT -> SystemResources; DISK_FULL -> NoSpaceLeft; the
ENOEXEC family -> InvalidExe), using `windows-sys` constants. Other targets (wasm): `Unexpected`.
Tests are cfg'd per platform (libc constants everywhere, literal tables for Linux/Apple/Windows) and
compile-checked with `just check-macos` / `just check-windows`.

## 2. Dot theme names (`ghostty-config/src/theme.rs`)
`zig_basename` ports `std.fs.path.basename` (ComponentIterator.last: trailing separators stripped,
`.`/`..` are ordinary names; `X:` drive root on Windows) instead of `Path::file_name`. `theme=.` and
`theme=..` now pass the basename check and report Zig's `not reading theme from ".": it is a
directory`. Differentially checked against /usr/bin/ghostty (`.`, `..`, `a/`, `a/b`, `./`: identical).

## 3. Sprite stroking (`ghostty-font/src/sprite/{stroke,canvas}.rs`)
tiny-skia is gone (dependency removed). New `stroke.rs` ports z2d 0.10 `stroke_plotter`, `Face`, `Pen`,
`Slope`, `Spline` (curve flattening with round joins), `PointBuffer` and Contour/Polygon edge building
(identity CTM, no dashes). `canvas.rs` now builds z2d-style nodes (`close` appends the implicit
`move_to`), fills via direct node flattening in f64 (no f32 skia round trip), ports z2d's Cairo-style
`arc` (segment count from the tolerance table), and uses z2d's truncating integer `src_over`
(`s + d - s*d/255`, `dst_in` `cov*a/255`) instead of rounding.
Sprite-sheet reference test: every one of the 36 sheets is now bit-exact (was 1.2% differing, max 86);
the test now requires 0 differing pixels.
`just visual-parity` (iteration build of the production profile): page, page-bar, tabs and chrome all
0 differing pixels (was 196 px / 0.020% for page).
