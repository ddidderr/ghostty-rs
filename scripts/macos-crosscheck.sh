#!/usr/bin/env bash
# Compile-check the macOS code paths from Linux (no macOS SDK needed).
# Uses `zig cc` wrappers for the few C build scripts in the dependency
# tree; real macOS builds need none of this. See docs/rewrite/notes/macos-M1.md.
# Usage: scripts/macos-crosscheck.sh [check|clippy] [target]
# MACOS_CHECK_FEATURES="ghostty-font/font-backend-freetype" adds cargo features
# (for example the FreeType-only font backend on Apple).
# MACOS_CHECK_ONLY="ghostty-macos" restricts the check to the given crates
# (space separated), for quick iteration.
set -euo pipefail
cd "$(dirname "$0")/.."
MODE=${1:-clippy}
TARGET=${2:-aarch64-apple-darwin}
case "$TARGET" in
  aarch64-apple-darwin) ZT=aarch64-macos ;;
  x86_64-apple-darwin) ZT=x86_64-macos ;;
  *) echo "unsupported target $TARGET" >&2; exit 2 ;;
esac
W="$PWD/target/macos-cross/$ZT"
mkdir -p "$W/stub/os" "$W/zig-cache"
for tool in cc c++; do
  cat > "$W/z$tool" <<EOS
#!/usr/bin/env bash
args=()
skip=0
for a in "\$@"; do
  if [ \$skip = 1 ]; then skip=0; continue; fi
  case "\$a" in
    -arch) skip=1 ;;
    -mmacosx-version-min=*|--target=*|-target) ;;
    *) args+=("\$a") ;;
  esac
done
exec zig $tool -target $ZT "\${args[@]}"
EOS
  chmod +x "$W/z$tool"
done
cat > "$W/stub/os/log.h" <<'EOS'
#pragma once
/* Declaration-only stand-in for the SDK header, enough for cc-rs to
   compile the oslog crate's wrapper during a cross *check*. */
typedef void *os_log_t;
typedef unsigned char os_log_type_t;
#define OS_LOG_DEFAULT ((os_log_t)0)
os_log_t os_log_create(const char *subsystem, const char *category);
void os_log_with_type(os_log_t log, os_log_type_t type, const char *fmt, ...);
#define os_log(l, ...) os_log_with_type(l, 0, __VA_ARGS__)
#define os_log_info(l, ...) os_log_with_type(l, 1, __VA_ARGS__)
#define os_log_debug(l, ...) os_log_with_type(l, 2, __VA_ARGS__)
#define os_log_error(l, ...) os_log_with_type(l, 16, __VA_ARGS__)
#define os_log_fault(l, ...) os_log_with_type(l, 17, __VA_ARGS__)
EOS
T=${TARGET//-/_}
export CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER=
export "CC_$T=$W/zcc" "CXX_$T=$W/zc++" "AR_$T=zig ar" "CFLAGS_$T=-I$W/stub"
export GETTEXT_DIR=/nonexistent ZIG_GLOBAL_CACHE_DIR="$W/zig-cache" ZIG_LOCAL_CACHE_DIR="$W/zig-cache"
CRATES=(-p ghostty-vt -p ghostty-unicode -p ghostty-datastruct -p ghostty-os -p ghostty-input
  -p ghostty-config -p ghostty-terminal -p ghostty-font -p ghostty-renderer -p ghostty-termio
  -p ghostty-core -p ghostty-apprt -p ghostty-inspector -p ghostty-lib -p ghostty-crash
  -p ghostty-bench -p ghostty-vt-capi -p ghostty-macos)
if [ -n "${MACOS_CHECK_ONLY:-}" ]; then
  CRATES=()
  for c in $MACOS_CHECK_ONLY; do CRATES+=(-p "$c"); done
fi
FEATURES=()
if [ -n "${MACOS_CHECK_FEATURES:-}" ]; then
  FEATURES=(--features "$MACOS_CHECK_FEATURES")
fi
if [ "$MODE" = clippy ]; then
  cargo clippy --target "$TARGET" "${CRATES[@]}" "${FEATURES[@]}" --lib --tests -- -D warnings
  # The macOS app executable (an Apple-only binary).
  cargo clippy --target "$TARGET" -p ghostty-macos --bins -- -D warnings
else
  cargo check --target "$TARGET" "${CRATES[@]}" "${FEATURES[@]}" --lib --tests
  cargo check --target "$TARGET" -p ghostty-macos --bins
fi
