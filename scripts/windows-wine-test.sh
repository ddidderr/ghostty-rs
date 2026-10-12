#!/usr/bin/env bash
# Run the Windows test suites under Wine (x86_64-pc-windows-gnu + mingw-w64).
#
# Usage: scripts/windows-wine-test.sh [cargo test args, default: the core crates]
#
# First run builds expat + FreeType + fontconfig for mingw into
# target/wine-deps/prefix (like Ghostty, from source; needs network for the
# expat tarball unless EXPAT_TARBALL points at one, and the fontconfig source
# from the zig package cache or FONTCONFIG_SRC). A dedicated WINEPREFIX lives in
# target/wine-prefix. Headless: no display is needed for console tests.
# See docs/rewrite/notes/windows-wine.md.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
D="$ROOT/target/wine-deps"
P="$D/prefix"
export RUSTC_WRAPPER= CARGO_BUILD_RUSTC_WRAPPER=   # kache breaks cc-rs
export WINEPREFIX="$ROOT/target/wine-prefix" WINEDEBUG=-all WINEARCH=win64
unset DISPLAY WAYLAND_DISPLAY

build_deps() {
  [ -f "$P/lib/libfontconfig.a" ] && return 0
  mkdir -p "$D"
  local ft expat fc
  ft=$(echo "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/freetype-sys-0.23.0/freetype2)
  [ -d "$ft" ] || { cargo fetch --target x86_64-pc-windows-gnu >/dev/null; ft=$(echo "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/freetype-sys-0.23.0/freetype2); }
  fc="${FONTCONFIG_SRC:-}"
  if [ -z "$fc" ]; then
    for d in "$HOME"/.cache/zig/p/*/; do
      grep -q "project('fontconfig'" "$d/meson.build" 2>/dev/null && fc="$d" && break
    done
  fi
  if [ -z "$fc" ]; then
    curl -sSLf -o "$D/fontconfig.tar.xz" https://www.freedesktop.org/software/fontconfig/release/fontconfig-2.14.2.tar.xz
    tar -C "$D" -xf "$D/fontconfig.tar.xz"; fc="$D/fontconfig-2.14.2"
  fi
  if [ ! -d "$D/expat-2.6.4" ]; then
    if [ -n "${EXPAT_TARBALL:-}" ]; then cp "$EXPAT_TARBALL" "$D/expat.tar.bz2"
    else curl -sSLf -o "$D/expat.tar.bz2" https://github.com/libexpat/libexpat/releases/download/R_2_6_4/expat-2.6.4.tar.bz2; fi
    tar -C "$D" -xjf "$D/expat.tar.bz2"
  fi
  x86_64-w64-mingw32-cmake -S "$D/expat-2.6.4" -B "$D/b-expat" -G Ninja -DCMAKE_INSTALL_PREFIX="$P" \
    -DBUILD_SHARED_LIBS=OFF -DEXPAT_BUILD_TOOLS=OFF -DEXPAT_BUILD_EXAMPLES=OFF -DEXPAT_BUILD_TESTS=OFF \
    -DEXPAT_BUILD_DOCS=OFF -DCMAKE_BUILD_TYPE=Release >/dev/null
  cmake --build "$D/b-expat" --target install >/dev/null
  x86_64-w64-mingw32-cmake -S "$ft" -B "$D/b-ft" -G Ninja -DCMAKE_INSTALL_PREFIX="$P" -DBUILD_SHARED_LIBS=OFF \
    -DFT_DISABLE_BROTLI=ON -DFT_DISABLE_BZIP2=ON -DFT_DISABLE_PNG=ON -DFT_DISABLE_HARFBUZZ=ON \
    -DCMAKE_BUILD_TYPE=Release >/dev/null
  cmake --build "$D/b-ft" --target install >/dev/null
  cat > "$D/cross.ini" <<EOS
[binaries]
c = 'x86_64-w64-mingw32-gcc'
cpp = 'x86_64-w64-mingw32-g++'
ar = 'x86_64-w64-mingw32-ar'
strip = 'x86_64-w64-mingw32-strip'
pkg-config = 'pkg-config'
[host_machine]
system = 'windows'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
EOS
  PKG_CONFIG_PATH="$P/lib/pkgconfig:/usr/x86_64-w64-mingw32/lib/pkgconfig" \
  PKG_CONFIG_LIBDIR="$P/lib/pkgconfig:/usr/x86_64-w64-mingw32/lib/pkgconfig" \
    meson setup "$D/b-fc" "$fc" --cross-file "$D/cross.ini" --prefix "$P" --default-library static \
      -Dtools=disabled -Dtests=disabled -Ddoc=disabled -Dnls=disabled -Dcache-build=disabled \
      --wrap-mode nodownload >/dev/null
  ninja -C "$D/b-fc" install >/dev/null
}
build_deps

export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_ALL_STATIC=1
export PKG_CONFIG_PATH="$P/lib/pkgconfig" PKG_CONFIG_LIBDIR="$P/lib/pkgconfig:/usr/x86_64-w64-mingw32/lib/pkgconfig"
export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++ \
  AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine
# `cfg(wine)` lets tests mark documented Wine limitations as ignored.
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="--cfg wine"
# libstdc++ (bundled HarfBuzz) / libgcc / winpthreads DLLs for the test exes.
export WINEPATH='Z:\usr\x86_64-w64-mingw32\bin'

CRATES=(-p ghostty-vt -p ghostty-unicode -p ghostty-datastruct -p ghostty-input -p ghostty-os
  -p ghostty-config -p ghostty-terminal -p ghostty-crash -p ghostty-termio -p ghostty-cli)
if [ $# -gt 0 ]; then CRATES=("$@"); fi
wine wineboot -i >/dev/null 2>&1 || true
exec timeout "${WINE_TEST_TIMEOUT:-2400}" cargo test --target x86_64-pc-windows-gnu --no-fail-fast "${CRATES[@]}"
