#!/usr/bin/env bash
# Compile-check (clippy -D warnings) the Windows code paths from Linux with
# mingw. No linking happens, so a stub fontconfig.pc is enough. See
# docs/rewrite/notes/windows-WIN.md.
set -euo pipefail
cd "$(dirname "$0")/.."
W="$PWD/target/windows-cross"
mkdir -p "$W/pkgconfig"
cat > "$W/pkgconfig/fontconfig.pc" <<'EOS'
Name: fontconfig
Description: stub for cross cargo check
Version: 2.15.0
Libs: -lfontconfig
Cflags:
EOS
export RUSTC_WRAPPER= CARGO_BUILD_RUSTC_WRAPPER= PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH="$W/pkgconfig"
export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++ \
  AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
cargo clippy --target x86_64-pc-windows-gnu --lib --tests \
  -p ghostty-vt -p ghostty-unicode -p ghostty-datastruct -p ghostty-input -p ghostty-os \
  -p ghostty-config -p ghostty-terminal -p ghostty-termio -p ghostty-crash -p ghostty-core \
  -p ghostty-cli -- -D warnings
