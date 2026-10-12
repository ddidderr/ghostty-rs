#!/usr/bin/env bash
# Create the versioned source tarball (Zig: `zig build dist`,
# build/GhosttyDist.zig) or check one (`zig build distcheck`).
#
#   scripts/dist.sh [OUT_DIR]          create OUT_DIR/ghostty-<version>.tar.gz
#   scripts/dist.sh --check [TARBALL]  extract it and verify it
#
# The tarball extracts into `ghostty-<version>/` (expected by debhelper,
# rpmbuild, ...) and contains, in addition to the checked-in sources:
#   * `VERSION`: the embedded Ghostty version (Zig: the `VERSION` file),
#   * the prebuilt dist resources that are not committed but built here to cut
#     the build-time dependencies of downstream packagers: the compiled GTK
#     GResource bundle `crates/ghostty-apprt-gtk/dist/ghostty.gresource`
#     (Zig: `src/apprt/gtk/ghostty_resources.{c,h}`; picked up by
#     ghostty-apprt-gtk/build.rs outside of a git checkout, replacing
#     blueprint-compiler + glib-compile-resources). The boo frame data
#     (Zig: `framedata.compressed`) is already committed
#     (`crates/ghostty-cli/src/boo_frames.deflate`).
#
# In a git checkout the file set is `git ls-files` (tracked files, plus
# untracked files that are not ignored), otherwise all files except build
# output. Files marked `export-ignore` in .gitattributes are dropped.
set -euo pipefail

cd "$(dirname "$0")/.."
root=$PWD

# Resources that are generated into the tarball (dist path relative to root).
dist_resources=(crates/ghostty-apprt-gtk/dist/ghostty.gresource)

version() {
    cargo build -q --release -p ghostty-build-data >&2
    "${CARGO_TARGET_DIR:-target}/release/ghostty-build-data" +version
}

if [ "${1:-}" = --check ]; then
    version=$(version)
    tarball=${2:-target/dist/ghostty-$version.tar.gz}
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    tar -xzf "$tarball" -C "$work"
    src="$work/ghostty-$version"
    [ -f "$src/VERSION" ] || { echo "missing VERSION" >&2; exit 1; }
    [ "$(cat "$src/VERSION")" = "$version" ] || { echo "VERSION mismatch" >&2; exit 1; }
    for r in "${dist_resources[@]}"; do
        [ -f "$src/$r" ] || { echo "missing dist resource $r" >&2; exit 1; }
    done
    # Check that the extracted tree builds and passes its tests.
    (cd "$src" && cargo test --offline -q -p ghostty-build-data)
    echo "distcheck ok: $tarball"
    exit 0
fi

out=${1:-target/dist}
version=$(version)
name=ghostty-$version
mkdir -p "$out"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Generate the dist resources.
cargo build -q -p ghostty-apprt-gtk
gres=$(find "${CARGO_TARGET_DIR:-target}/debug/build" -path '*ghostty-apprt-gtk-*/out/ghostty.gresource' -printf '%T@ %p\n' |
    sort -rn | head -1 | cut -d' ' -f2-)
[ -n "$gres" ] || { echo "ghostty.gresource was not built" >&2; exit 1; }
mkdir -p "$work/stage/crates/ghostty-apprt-gtk/dist"
cp "$gres" "$work/stage/crates/ghostty-apprt-gtk/dist/ghostty.gresource"
printf '%s' "$version" >"$work/stage/VERSION"

# The file list.
if [ -e .git ] && command -v git >/dev/null 2>&1; then
    git ls-files -z --cached --others --exclude-standard
else
    find . -path ./target -prune -o -path ./.git -prune -o -path ./.zig-cache -prune \
        -o -name '*.orig' -prune -o -type f -print0 | sed -z 's|^\./||'
fi >"$work/files0"
# export-ignore (git attribute).
if [ -e .git ] && command -v git >/dev/null 2>&1; then
    git check-attr -z export-ignore --stdin -z <"$work/files0" |
        tr '\0' '\n' | awk 'NR%3==1{f=$0} NR%3==0 && $0=="set"{print f}' >"$work/ignored" || true
    if [ -s "$work/ignored" ]; then
        tr '\0' '\n' <"$work/files0" | grep -vxFf "$work/ignored" | tr '\n' '\0' >"$work/files0.new"
        mv "$work/files0.new" "$work/files0"
    fi
fi
# Drop files that no longer exist (deleted but still tracked).
tr '\0' '\n' <"$work/files0" | while IFS= read -r f; do [ -e "$f" ] && printf '%s\0' "$f"; done >"$work/files"

tar --create --null --files-from "$work/files" \
    --transform "s,^,$name/," --sort=name --owner=0 --group=0 --numeric-owner \
    -f "$work/src.tar"
(cd "$work/stage" && tar --append -f "$work/src.tar" --transform "s,^,$name/," \
    --owner=0 --group=0 --numeric-owner VERSION crates)
gzip -n -c "$work/src.tar" >"$out/$name.tar.gz"
echo "$out/$name.tar.gz"
