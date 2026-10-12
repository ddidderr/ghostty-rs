set positional-arguments

run *args:
    cargo run -- "$@"

build:
    cargo build

build-release:
    cargo build --release

build-production:
    cargo build --profile production

fmt:
    cargo +nightly fmt
    tombi format
    fd -tf -e md -x prettier --write --prose-wrap always --print-width 80
    rumdl check --flavor commonmark --fix
    just --fmt

_fix:
    cargo fix --workspace --all-targets --all-features
    cargo clippy --fix --workspace --all-targets --all-features

fix: _fix fmt

# Build libghostty-vt (C ABI, include/ghostty/vt.h) as target/<profile>/libghostty-vt.{so,a}
# and, if the target is installed, target/wasm32-unknown-unknown/<profile>/ghostty-vt.wasm.
vt-lib profile="release":
    cargo build -p ghostty-vt-capi --profile {{profile}}
    cp target/{{profile}}/libghostty_vt_capi.so target/{{profile}}/libghostty-vt.so
    cp target/{{profile}}/libghostty_vt_capi.a target/{{profile}}/libghostty-vt.a
    cargo build -p ghostty-vt-capi --profile {{profile}} --target wasm32-unknown-unknown
    cp target/wasm32-unknown-unknown/{{profile}}/ghostty_vt_capi.wasm target/wasm32-unknown-unknown/{{profile}}/ghostty-vt.wasm

# Run the libghostty-vt ABI check and the C/wasm example tests.
vt-test:
    cargo test -p ghostty-vt-capi

clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace --all-targets --all-features

clean:
    cargo clean

# Build the build-data generator (Zig: `ghostty-build-data`,
# src/main_build_data.zig) in release mode.
build-data:
    cargo build --release -p ghostty-build-data

# Print generated build data to stdout. MODE is one of: bash zsh fish sublime
# vim-syntax vim-ftdetect vim-ftplugin vim-compiler terminfo mdgen-ghostty-1
# mdgen-ghostty-5. Example: `just build-data-run bash`.
build-data-run MODE:
    cargo run -q --release -p ghostty-build-data -- "+{{ MODE }}"

# Generate the man page markdown (ghostty.1.md / ghostty.5.md) plus html and
# roff man pages via pandoc into OUT (Zig: GhosttyDocs.zig). When pandoc is
# not installed only the markdown is produced.
docs OUT="target/docs": build-data
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{ OUT }}"
    for s in 1 5; do
        target/release/ghostty-build-data "+mdgen-ghostty-$s" > "{{ OUT }}/ghostty.$s.md"
        if command -v pandoc >/dev/null 2>&1; then
            pandoc --standalone --from markdown --to html "{{ OUT }}/ghostty.$s.md" > "{{ OUT }}/ghostty.$s.html"
            pandoc --standalone --from markdown --to man "{{ OUT }}/ghostty.$s.md" > "{{ OUT }}/ghostty.$s"
        else
            echo "pandoc not found: only markdown written for ghostty($s)" >&2
        fi
    done

# Compile the gettext catalogs in po/ into OUT/<lang>/LC_MESSAGES/<domain>.mo
# (Zig: GhosttyI18n.zig; domain com.mitchellh.ghostty).
locales OUT="target/locale":
    #!/usr/bin/env bash
    set -euo pipefail
    for po in po/*.po; do
        lang=$(basename "$po" .po)
        mkdir -p "{{ OUT }}/$lang/LC_MESSAGES"
        msgfmt -o "{{ OUT }}/$lang/LC_MESSAGES/com.mitchellh.ghostty.mo" "$po"
    done

# Install bin + share/ghostty (shell-integration, themes, docs) +
# share/terminfo (compiled db, ghostty.terminfo, ghostty.termcap) + shell
# completions, vim/neovim/bat syntax, man pages, translations, desktop file,
# D-Bus/systemd/metainfo and icons, plus libghostty-vt (lib, headers,
# libghostty-vt.pc), mirroring Ghostty's GhosttyResources.zig,
# GhosttyDocs.zig, GhosttyI18n.zig and GhosttyLibVt.zig.
#
# Usage: `just install [PREFIX] [FLATPAK] [SYSTEM_PACKAGE] [PROFILE]`;
# honours DESTDIR.
#   PREFIX          install prefix (default /usr/local)
#   FLATPAK         "true": use the Flatpak D-Bus template, no systemd unit
#                   and no 1024x1024 icon (Zig: -Dflatpak)
#   SYSTEM_PACKAGE  "true": install the systemd user unit to lib/systemd/user
#                   instead of share/systemd/user (Zig: --system / system
#                   package mode)
#   PROFILE         cargo profile to build and install (default "production":
#                   no debug assertions, the Zig ReleaseFast equivalent).
#                   Like Zig's Debug/ReleaseSafe modes, any other profile
#                   (dev, release) installs the "-debug" application id
#                   (com.mitchellh.ghostty-debug, "Ghostty (Debug)"), which is
#                   the id the binary uses at runtime when built with debug
#                   assertions.
install PREFIX="/usr/local" FLATPAK="false" SYSTEM_PACKAGE="false" PROFILE="production": build-data docs locales
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --profile {{ PROFILE }}
    just install-lib '{{ PREFIX }}' '{{ PROFILE }}' vt
    P="${DESTDIR:-}{{ PREFIX }}"
    BD=target/release/ghostty-build-data
    dir={{ PROFILE }}
    if [ "$dir" = dev ]; then dir=debug; fi
    # GhosttyResources.zig: Debug/ReleaseSafe get the "-debug" id and name.
    if [ "{{ PROFILE }}" = production ]; then
        APPID=com.mitchellh.ghostty
        NAME=Ghostty
    else
        APPID=com.mitchellh.ghostty-debug
        NAME="Ghostty (Debug)"
    fi
    install -Dm755 "target/$dir/ghostty" "$P/bin/ghostty"
    mkdir -p "$P/share/ghostty" "$P/share/terminfo"
    cp -R resources/shell-integration resources/themes "$P/share/ghostty/"
    find "$P/share/ghostty/shell-integration" -name '*.md' -delete
    find "$P/share/ghostty/themes" -name '*.md' -delete

    # Terminfo: compiled database plus the textual sources.
    cp -R resources/terminfo/. "$P/share/terminfo/"
    $BD +terminfo > "$P/share/terminfo/ghostty.terminfo"
    if command -v infotocap >/dev/null 2>&1; then
        infotocap "$P/share/terminfo/ghostty.terminfo" > "$P/share/terminfo/ghostty.termcap" 2>/dev/null
    else
        echo "infotocap not found: skipping ghostty.termcap" >&2
    fi

    # Shell completions.
    $BD +fish | install -Dm644 /dev/stdin "$P/share/fish/vendor_completions.d/ghostty.fish"
    $BD +zsh | install -Dm644 /dev/stdin "$P/share/zsh/site-functions/_ghostty"
    $BD +bash | install -Dm644 /dev/stdin "$P/share/bash-completion/completions/ghostty.bash"

    # Vim and Neovim plugin files, and the sublime syntax used by bat.
    for root in share/vim/vimfiles share/nvim/site; do
        $BD +vim-syntax | install -Dm644 /dev/stdin "$P/$root/syntax/ghostty.vim"
        $BD +vim-ftdetect | install -Dm644 /dev/stdin "$P/$root/ftdetect/ghostty.vim"
        $BD +vim-ftplugin | install -Dm644 /dev/stdin "$P/$root/ftplugin/ghostty.vim"
        $BD +vim-compiler | install -Dm644 /dev/stdin "$P/$root/compiler/ghostty.vim"
    done
    $BD +sublime | install -Dm644 /dev/stdin "$P/share/bat/syntaxes/ghostty.sublime-syntax"

    # Docs (markdown always; html and man pages when pandoc is available).
    for s in 1 5; do
        install -Dm644 "target/docs/ghostty.$s.md" "$P/share/ghostty/doc/ghostty.$s.md"
        if [ -f "target/docs/ghostty.$s" ]; then
            install -Dm644 "target/docs/ghostty.$s.html" "$P/share/ghostty/doc/ghostty.$s.html"
            install -Dm644 "target/docs/ghostty.$s" "$P/share/man/man$s/ghostty.$s"
        fi
    done

    # Translations.
    for mo in target/locale/*/LC_MESSAGES/com.mitchellh.ghostty.mo; do
        lang=$(basename "$(dirname "$(dirname "$mo")")")
        install -Dm644 "$mo" "$P/share/locale/$lang/LC_MESSAGES/com.mitchellh.ghostty.mo"
    done

    gen() { # template out
        mkdir -p "$(dirname "$2")"
        sed -e "s|@NAME@|$NAME|g" -e "s|@APPID@|$APPID|g" \
            -e "s|@GHOSTTY@|{{ PREFIX }}/bin/ghostty|g" "$1" > "$2"
    }
    gen dist/linux/app.desktop.in "$P/share/applications/$APPID.desktop"
    if [ "{{ FLATPAK }}" = "true" ]; then
        gen dist/linux/dbus.service.flatpak.in "$P/share/dbus-1/services/$APPID.service"
    else
        gen dist/linux/dbus.service.in "$P/share/dbus-1/services/$APPID.service"
        # systemd looks in lib/ for system packages and share/ otherwise.
        sysd=share
        if [ "{{ SYSTEM_PACKAGE }}" = "true" ]; then sysd=lib; fi
        gen dist/linux/systemd.service.in "$P/$sysd/systemd/user/app-$APPID.service"
    fi
    gen dist/linux/com.mitchellh.ghostty.metainfo.xml.in "$P/share/metainfo/$APPID.metainfo.xml"
    install -Dm644 dist/linux/ghostty_dolphin.desktop "$P/share/kio/servicemenus/com.mitchellh.ghostty.desktop"
    install -Dm755 dist/linux/ghostty_nautilus.py "$P/share/nautilus-python/extensions/ghostty.py"
    ic="$P/share/icons/hicolor"
    for pair in 16:16x16 32:32x32 128:128x128 256:256x256 512:512x512 1024:1024x1024 \
                32:16x16@2 64:32x32@2 256:128x128@2 512:256x256@2; do
        # Flatpaks only support icons up to 512x512.
        if [ "{{ FLATPAK }}" = "true" ] && [ "${pair%%:*}" = "1024" ]; then continue; fi
        install -Dm644 "dist/icons/${pair%%:*}.png" "$ic/${pair##*:}/apps/com.mitchellh.ghostty.png"
    done
    echo "installed to $P (application id $APPID)"

# Regenerate resources/terminfo from the terminfo source in ghostty-termio.
terminfo:
    scripts/gen-terminfo.sh

# Install libghostty and libghostty-vt below PREFIX (honours DESTDIR):
# lib/libghostty.{so,a}, include/ghostty.h, include/module.modulemap
# (GhosttyLib.zig) and lib/libghostty-vt.so{,.0,.0.1.0},
# include/ghostty/vt*.h, share/pkgconfig/libghostty-vt.pc (GhosttyLibVt.zig).
# WHAT is one of: all, vt, ghostty.
install-lib PREFIX="/usr/local" PROFILE="production" WHAT="all":
    scripts/install-lib.sh '{{ PREFIX }}' '{{ PROFILE }}' '{{ WHAT }}'

# Build libghostty (the embedded C API) as static and shared libraries.
lib PROFILE="production":
    cargo build -p ghostty-lib --profile {{ PROFILE }}

# Build GhosttyKit.xcframework with lipo + xcodebuild (macOS only; Zig:
# GhosttyXCFramework.zig). TARGET is "universal" (macOS arm64+x86_64, iOS,
# iOS simulator) or "native". Set DRY_RUN=1 to only print the commands.
xcframework TARGET="universal" PROFILE="production" OUT="macos/GhosttyKit.xcframework":
    scripts/xcframework.sh '{{ TARGET }}' '{{ PROFILE }}' '{{ OUT }}'

# Generate the website reference data (Zig: GhosttyWebdata.zig): config.mdx,
# actions.mdx (keybind actions) and commands.mdx (CLI actions) in OUT.
webdata OUT="target/webdata": build-data
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "{{ OUT }}"
    for m in config actions commands; do
        target/release/ghostty-build-data "+webgen-$m" > "{{ OUT }}/$m.mdx"
    done

# Install the website data to PREFIX/share/ghostty/webdata (Zig:
# `-Demit-webdata`); honours DESTDIR.
install-webdata PREFIX="/usr/local": webdata
    #!/usr/bin/env bash
    set -euo pipefail
    for m in config actions commands; do
        install -Dm644 "target/webdata/$m.mdx" "${DESTDIR:-}{{ PREFIX }}/share/ghostty/webdata/$m.mdx"
    done

# Regenerate po/com.mitchellh.ghostty.pot (xgettext over the GTK blueprints,
# the GTK apprt Rust sources and the Nautilus extension) and msgmerge every
# po/*.po against it (Zig: `zig build update-translations`).
update-translations:
    scripts/update-translations.sh

# Create the versioned source tarball target/dist/ghostty-<version>.tar.gz with
# the embedded VERSION file and the prebuilt dist resources (Zig: `zig build
# dist`).
dist:
    scripts/dist.sh target/dist

# Extract the dist tarball and check that it builds and tests (Zig:
# `zig build distcheck`).
distcheck: dist
    scripts/dist.sh --check
