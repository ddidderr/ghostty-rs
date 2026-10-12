#!/usr/bin/env bash
# Visual parity: capture original Ghostty and the Rust build in an isolated,
# headless nested weston and compare pixel-wise.
#
#   scripts/visual-parity.sh [scenario...]     (default: page page-bar tabs;
#   also available: inspector = ctrl+shift+i via xdotool)
#
# env: RUST_BIN (default target/production/ghostty), REF_BIN (default
# /usr/bin/ghostty), OUT (default target/visual-parity), W/H output size.
# Output: $OUT/<scenario>/{ref,rust,diff}.png (+ chrome/term crops) and stats.txt.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
RUST_BIN=${RUST_BIN:-$ROOT/target/production/ghostty}
REF_BIN=${REF_BIN:-/usr/bin/ghostty}
OUT=${OUT:-$ROOT/target/visual-parity}
W=${W:-1200}; H=${H:-800}
VP=$ROOT/scripts/visual-parity
SCEN=${*:-page page-bar tabs}
mkdir -p "$OUT"
export XDG_RUNTIME_DIR=${VP_RUNTIME_DIR:-/tmp/vp-runtime-$UID}
mkdir -p -m700 "$XDG_RUNTIME_DIR"
WORK=$(mktemp -d "$OUT/work.XXXX")
SOCK=vp$$

XWL=
case " $SCEN " in *" tabs "*|*" inspector "*) XWL=xwayland=true; mkdir -p /tmp/.X11-unix ;; esac
cat >"$WORK/weston.ini" <<INI
[core]
shell=desktop-shell.so
$XWL
[output]
name=headless
mode=${W}x${H}
[shell]
panel-position=none
background-color=0xff202020
INI

__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json XWAYLAND_NO_GLAMOR=1 weston --debug --backend=headless --renderer=gl --socket=$SOCK -c "$WORK/weston.ini" \
  >"$WORK/weston.log" 2>&1 &
WPID=$!
trap 'kill $WPID 2>/dev/null; wait 2>/dev/null' EXIT
for _ in $(seq 50); do [ -S "$XDG_RUNTIME_DIR/$SOCK" ] && break; sleep 0.1; done
sleep 1

write_cfg() { # dir, extra
  mkdir -p "$1/ghostty"
  cat >"$1/ghostty/config" <<CFG
font-family = JetBrains Mono
font-size = 13
background = 1e1e2e
foreground = cdd6f4
maximize = true
window-padding-x = 8
window-padding-y = 8
cursor-style = block
cursor-style-blink = false
window-theme = dark
title = Ghostty
confirm-close-surface = false
shell-integration = none
quit-after-last-window-closed = true
$2
CFG
}

shoot() { # outfile
  local d; d=$(mktemp -d "$WORK/shot.XXXX")
  (cd "$d" && WAYLAND_DISPLAY=$SOCK timeout 20 weston-screenshooter >/dev/null 2>&1)
  mv "$d"/*.png "$1" && rmdir "$d"
}

# run BIN LABEL SCENARIO OUTPNG
run_one() {
  local bin=$1 label=$2 scen=$3 out=$4 cfgx=""
  local cfg="$WORK/cfg-$label-$scen"
  case $scen in page-bar) cfgx="cursor-style = bar" ;; esac
  write_cfg "$cfg" "$cfgx"
  local env=(LANG=C.UTF-8 XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR WAYLAND_DISPLAY=$SOCK XDG_CONFIG_HOME=$cfg GSETTINGS_BACKEND=memory
    DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent GDK_BACKEND=wayland
    HOME=$WORK/home-$label)
  mkdir -p "$WORK/home-$label"
  case $scen in
  tabs|inspector)
    # Needs input: run under Xwayland and drive with xdotool.
    env=("${env[@]/GDK_BACKEND=wayland/GDK_BACKEND=x11}")
    env+=(DISPLAY="$(xwl_display)")
    env -i PATH="$PATH" "${env[@]}" timeout 60 "$bin" --gtk-single-instance=false \
      -e "$VP/testpage.sh" >"$WORK/$label-$scen.log" 2>&1 &
    local pid=$!
    sleep 6
    xdotool_keys "$scen"
    sleep 3
    shoot "$out"
    kill $pid 2>/dev/null; wait $pid 2>/dev/null
    ;;
  *)
    env -i PATH="$PATH" "${env[@]}" timeout 60 "$bin" --gtk-single-instance=false \
      -e "$VP/testpage.sh" >"$WORK/$label-$scen.log" 2>&1 &
    local pid=$!
    sleep 7
    shoot "$out"
    kill $pid 2>/dev/null; wait $pid 2>/dev/null
    ;;
  esac
}
xwl_display() { grep -o "display :[0-9]*" "$WORK/weston.log" | tail -1 | sed 's/display //'; }
xdotool_keys() {
  export DISPLAY=$(xwl_display)
  if [ "${1:-}" = inspector ]; then
    xdotool key --clearmodifiers ctrl+shift+i; sleep 3
    return
  fi
  xdotool key --clearmodifiers ctrl+shift+t; sleep 1
  xdotool key --clearmodifiers ctrl+shift+o; sleep 1
  xdotool key --clearmodifiers ctrl+shift+e; sleep 1
}

: >"$OUT/stats.txt"
for s in $SCEN; do
  mkdir -p "$OUT/$s"
  run_one "$REF_BIN" ref "$s" "$OUT/$s/ref.png"
  run_one "$RUST_BIN" rust "$s" "$OUT/$s/rust.png"
  python3 "$VP/compare.py" "$OUT/$s/ref.png" "$OUT/$s/rust.png" "$OUT/$s/diff.png" --name "$s/full" | tee -a "$OUT/stats.txt"
  # terminal area: window content below the headerbar; chrome: the headerbar strip.
  # Window geometry is deterministic under desktop-shell; override with TERM_CROP / CHROME_CROP.
  python3 "$VP/compare.py" "$OUT/$s/ref.png" "$OUT/$s/rust.png" "$OUT/$s/diff-term.png" \
    --crop "${TERM_CROP:-0,48,1200,752}" --name "$s/terminal" | tee -a "$OUT/stats.txt"
  python3 "$VP/compare.py" "$OUT/$s/ref.png" "$OUT/$s/rust.png" "$OUT/$s/diff-chrome.png" \
    --crop "${CHROME_CROP:-0,0,1200,48}" --name "$s/chrome" | tee -a "$OUT/stats.txt"
done
[ -n "${KEEP:-}" ] || rm -rf "$WORK"
