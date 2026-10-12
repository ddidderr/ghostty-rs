#!/usr/bin/env bash
# Build GhosttyKit.xcframework (macOS only; mirrors build/GhosttyLib.zig
# `initMacOSUniversal`/`initStatic`, build/GhosttyXCFramework.zig,
# build/LipoStep.zig and build/XCFrameworkStep.zig).
#
# Usage: scripts/xcframework.sh [universal|native] [PROFILE] [OUT]
#   universal  (default) a universal (arm64 + x86_64, via lipo) macOS static
#              library, an iOS device library and an iOS simulator library.
#   native     only the static library for the host macOS architecture.
#   PROFILE    cargo profile (default production)
#   OUT        output path (default macos/GhosttyKit.xcframework, like Zig)
#
# Each slice is `libghostty.a` (the Rust staticlib already bundles every
# dependency, so there is no libtool step) plus the `include/` directory with
# ghostty.h and module.modulemap. Static libraries have no dSYM.
#
# DRY_RUN=1 prints the commands instead of running them (usable on any host).
set -euo pipefail

cd "$(dirname "$0")/.."
target=${1:-universal}
profile=${2:-production}
out=${3:-macos/GhosttyKit.xcframework}
dir=$profile
[ "$profile" = dev ] && dir=debug
tdir=${CARGO_TARGET_DIR:-target}

# Config.zig osVersionMin: macOS 13, iOS 17.
export MACOSX_DEPLOYMENT_TARGET=13.0
export IPHONEOS_DEPLOYMENT_TARGET=17.0

run() {
    if [ "${DRY_RUN:-}" = 1 ]; then
        printf '+'
        printf ' %q' "$@"
        printf '\n'
    else
        "$@"
    fi
}

# Build only the static library for a Rust target triple (the crate also
# declares a cdylib, which needs no linking for the xcframework).
build() {
    run cargo rustc --profile "$profile" -p ghostty-lib --lib --crate-type staticlib --target "$1"
}
lib() { echo "$tdir/$1/$dir/libghostty.a"; }

host_arch=$(uname -m)
case $host_arch in
arm64 | aarch64) native=aarch64-apple-darwin ;;
*) native=x86_64-apple-darwin ;;
esac

args=()
case $target in
universal)
    build aarch64-apple-darwin
    build x86_64-apple-darwin
    build aarch64-apple-ios
    build aarch64-apple-ios-sim

    universal_dir="$tdir/universal-apple-darwin/$dir"
    run mkdir -p "$universal_dir"
    run lipo -create -output "$universal_dir/libghostty.a" \
        "$(lib aarch64-apple-darwin)" "$(lib x86_64-apple-darwin)"
    args+=(-library "$universal_dir/libghostty.a" -headers crates/ghostty-lib/include)
    args+=(-library "$(lib aarch64-apple-ios)" -headers crates/ghostty-lib/include)
    args+=(-library "$(lib aarch64-apple-ios-sim)" -headers crates/ghostty-lib/include)
    ;;
native)
    build "$native"
    args+=(-library "$(lib "$native")" -headers crates/ghostty-lib/include)
    ;;
*)
    echo "usage: $0 [universal|native] [PROFILE] [OUT]" >&2
    exit 2
    ;;
esac

# The output path is static, so delete the old framework first.
run rm -rf "$out"
run xcodebuild -create-xcframework "${args[@]}" -output "$out"
