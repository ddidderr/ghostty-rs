#!/usr/bin/env bash
# Install libghostty and/or libghostty-vt below a prefix, mirroring
# build/GhosttyLib.zig (`install`, `installHeader`) and build/GhosttyLibVt.zig.
#
# Usage: scripts/install-lib.sh PREFIX PROFILE [all|vt|ghostty]
#   PREFIX   install prefix; DESTDIR (env) is prepended for the file copies but
#            not for the paths recorded in the pkg-config file.
#   PROFILE  cargo profile whose artifacts are installed (dev -> target/debug).
#   WHAT     `vt`: libghostty-vt only (what `zig build` always installs);
#            `ghostty`: libghostty only (Zig: `-Dapp-runtime=none`, non-Darwin);
#            `all` (default): both.
#
# libghostty-vt (version 0.1.0):
#   lib/libghostty-vt.so.0.1.0 (+ libghostty-vt.so.0, libghostty-vt.so symlinks;
#   macOS: libghostty-vt.0.1.0.dylib, libghostty-vt.0.dylib, libghostty-vt.dylib)
#   include/ghostty/vt.h and include/ghostty/vt/**.h
#   share/pkgconfig/libghostty-vt.pc
# libghostty:
#   lib/libghostty.so, lib/libghostty.a (macOS: .dylib), include/ghostty.h,
#   include/module.modulemap
set -euo pipefail

cd "$(dirname "$0")/.."
prefix=${1:?usage: install-lib.sh PREFIX PROFILE [all|vt|ghostty]}
profile=${2:-production}
what=${3:-all}
dest="${DESTDIR:-}$prefix"
dir=$profile
[ "$profile" = dev ] && dir=debug
tdir=${CARGO_TARGET_DIR:-target}/$dir
cargo_args=(--profile "$profile")
vt_version=0.1.0
vt_major=${vt_version%%.*}

case $(uname -s) in
Darwin) dylib=dylib ;;
*) dylib=so ;;
esac

install_vt() {
    cargo build "${cargo_args[@]}" -p ghostty-vt-capi
    local src="$tdir/libghostty_vt_capi.$dylib"
    install -d "$dest/lib" "$dest/share/pkgconfig"
    if [ "$dylib" = so ]; then
        local real="libghostty-vt.so.$vt_version"
        install -m755 "$src" "$dest/lib/$real"
        # The cargo-built soname is the unversioned name; Zig's is `.so.0`.
        if command -v patchelf >/dev/null 2>&1; then
            patchelf --set-soname "libghostty-vt.so.$vt_major" "$dest/lib/$real"
        else
            echo "patchelf not found: soname stays libghostty-vt.so" >&2
        fi
        ln -sf "$real" "$dest/lib/libghostty-vt.so.$vt_major"
        ln -sf "libghostty-vt.so.$vt_major" "$dest/lib/libghostty-vt.so"
    else
        local real="libghostty-vt.$vt_version.dylib"
        install -m755 "$src" "$dest/lib/$real"
        if command -v install_name_tool >/dev/null 2>&1; then
            install_name_tool -id "@rpath/libghostty-vt.$vt_major.dylib" "$dest/lib/$real"
        fi
        ln -sf "$real" "$dest/lib/libghostty-vt.$vt_major.dylib"
        ln -sf "libghostty-vt.$vt_major.dylib" "$dest/lib/libghostty-vt.dylib"
    fi

    # Headers: include/ghostty/**.h (lib.installHeadersDirectory, .h only).
    local inc=crates/ghostty-vt-capi/include/ghostty
    while IFS= read -r h; do
        install -Dm644 "$inc/$h" "$dest/include/ghostty/$h"
    done < <(cd "$inc" && find . -type f -name '*.h' | sed 's|^\./||' | LC_ALL=C sort)

    # pkg-config file (no trailing newline, like the Zig `b.fmt` literal).
    printf 'prefix=%s\nincludedir=${prefix}/include\nlibdir=${prefix}/lib\n\nName: libghostty-vt\nURL: https://github.com/ghostty-org/ghostty\nDescription: Ghostty VT library\nVersion: %s\nCflags: -I${includedir}\nLibs: -L${libdir} -lghostty-vt' \
        "$prefix" "$vt_version" >"$dest/share/pkgconfig/libghostty-vt.pc"
    chmod 644 "$dest/share/pkgconfig/libghostty-vt.pc"
}

install_ghostty() {
    cargo build "${cargo_args[@]}" -p ghostty-lib
    install -d "$dest/lib" "$dest/include"
    install -m755 "$tdir/libghostty.$dylib" "$dest/lib/libghostty.$dylib"
    install -m644 "$tdir/libghostty.a" "$dest/lib/libghostty.a"
    install -m644 crates/ghostty-lib/include/ghostty.h "$dest/include/ghostty.h"
    install -m644 crates/ghostty-lib/include/module.modulemap "$dest/include/module.modulemap"
}

case $what in
all) install_vt; install_ghostty ;;
vt) install_vt ;;
ghostty) install_ghostty ;;
*) echo "unknown component: $what" >&2; exit 2 ;;
esac
echo "installed libraries to $dest"
