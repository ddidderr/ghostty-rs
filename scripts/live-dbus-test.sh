#!/usr/bin/env bash
# Live D-Bus / portal comparison of original Ghostty and the Rust build.
#
#   scripts/live-dbus-test.sh [scenario...]
#
# Scenarios: single-instance single-instance-detect notifications shortcuts settings cgroup openuri openurl-click real-portal
# (default: all).  Each scenario runs once per binary on a *private* session
# bus (dbus-run-session with its own config: only our service dir, no user
# services) inside a private headless weston; mock services
# (scripts/live-dbus/mock.py) answer org.freedesktop.Notifications,
# org.freedesktop.systemd1 and the portal interfaces Ghostty uses.  The
# user's session bus, portal and desktop are never touched.
#
# The original's normalized result (D-Bus traffic + observable effects) is the
# oracle; the Rust result must be identical.  Exit status 1 on any difference,
# 0 when everything matches or the needed tools are missing (skip).
#
# env: RUST_BIN (default target/vp/production/ghostty: debug builds use a different app id), REF_BIN (default /usr/bin/ghostty),
# OUT (default target/live-dbus), KEEP=1 keeps the weston log.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
RUST_BIN=${RUST_BIN:-$ROOT/target/vp/production/ghostty}
REF_BIN=${REF_BIN:-/usr/bin/ghostty}
OUT=${OUT:-$ROOT/target/live-dbus}
LD=$ROOT/scripts/live-dbus
SCEN=${*:-single-instance single-instance-detect notifications shortcuts settings cgroup openuri openurl-click real-portal}

for t in weston dbus-run-session busctl python3 xdotool; do
  command -v $t >/dev/null || { echo "live-dbus: SKIP ($t missing)"; exit 0; }
done
python3 -c 'import gi; gi.require_version("Gio","2.0")' 2>/dev/null || { echo "live-dbus: SKIP (python gi missing)"; exit 0; }
[ -x "$RUST_BIN" ] || [ "${LABELS:-}" = ref ] || { echo "live-dbus: SKIP (no rust build at $RUST_BIN)"; exit 0; }

mkdir -p "$OUT"
export XDG_RUNTIME_DIR=${VP_RUNTIME_DIR:-/tmp/vp-runtime-$UID}
mkdir -p -m700 "$XDG_RUNTIME_DIR" /tmp/.X11-unix 2>/dev/null
WORK=$(mktemp -d "$OUT/work.XXXX")
TAG=ldb$$
SOCK=ldb$$
cat >"$WORK/weston.ini" <<INI
[core]
shell=desktop-shell.so
xwayland=true
[output]
name=headless
mode=1200x800
[shell]
panel-position=none
background-color=0xff202020
INI
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json XWAYLAND_NO_GLAMOR=1 \
  timeout 3000 weston --debug --backend=headless --renderer=gl --socket=$SOCK -c "$WORK/weston.ini" \
  >"$WORK/weston.log" 2>&1 &
WPID=$!
cleanup() {
  kill $WPID 2>/dev/null
  for p in /proc/[0-9]*; do
    grep -qa "LDB_TAG=$TAG" "$p/environ" 2>/dev/null && kill -9 "${p#/proc/}" 2>/dev/null
  done
  wait 2>/dev/null
  [ -n "${KEEP:-}" ] || rm -rf "$WORK"
}
trap cleanup EXIT
for _ in $(seq 50); do [ -S "$XDG_RUNTIME_DIR/$SOCK" ] && break; sleep 0.1; done
[ -S "$XDG_RUNTIME_DIR/$SOCK" ] || { echo "live-dbus: SKIP (weston did not start)"; cat "$WORK/weston.log"; exit 0; }
sleep 1
XDISPLAY=$(grep -o "display :[0-9]*" "$WORK/weston.log" | tail -1 | sed 's/display //')

rc=0
RUST_ONLY=" openurl-click "   # the original cannot be driven (no input injection): golden file only
cut_bus() { sed '/^== bus traffic/,$d' "$1"; }
run_label() { # scenario label bin
  local s=$1 label=$2 bin=$3
  local d=$OUT/$s/$label; rm -rf "$d"; mkdir -p "$d"
  local run=$WORK/run-$s-$label; mkdir -p "$run/services"
  # Private bus: only our service dir (activation must start the binary
  # under test, not /usr/bin/ghostty), no user services.
  cat >"$run/session.conf" <<CONF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><keep_umask/><listen>unix:tmpdir=$XDG_RUNTIME_DIR</listen><auth>EXTERNAL</auth>
<servicedir>$run/services</servicedir>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>
CONF
  printf '[D-BUS Service]\nName=com.mitchellh.ghostty\nExec=%s --gtk-single-instance=true --initial-window=false\n' "$bin" \
    >"$run/services/com.mitchellh.ghostty.service"
  LDB_LABEL=$label LDB_TAG=$TAG WAYLAND_DISPLAY=$SOCK DISPLAY=$XDISPLAY LDB_BIN=$bin LDB_RUN=$run LDB_ROOT=$ROOT \
    timeout 240 dbus-run-session --config-file="$run/session.conf" -- \
    bash "$LD/scenario.sh" "$s" "$bin" "$d" >"$d/driver.log" 2>&1
  LDB_RUN=$run LDB_ROOT=$ROOT python3 "$LD/normalize.py" "$d" >"$d/result.txt" 2>"$d/normalize.err"
}
for s in $SCEN; do
  golden=$LD/golden/$s.txt
  mkdir -p "$OUT/$s"
  have_ref=0
  case "$RUST_ONLY" in *" $s "*) ;; *) [ -x "$REF_BIN" ] && [ "${LABELS:-ref rust}" != rust ] && have_ref=1 ;; esac
  if [ $have_ref = 1 ]; then run_label "$s" ref "$REF_BIN"; fi
  if [ -x "$RUST_BIN" ] && [ "${LABELS:-rust}" != ref ]; then
    if [ $have_ref = 1 ] || [ -f "$golden" ] || [ -n "${UPDATE_GOLDEN:-}" ]; then
      run_label "$s" rust "$RUST_BIN"
    fi
  fi
  [ "${LABELS:-}" = ref ] && continue
  ok=1
  if grep -qs '^ERROR' "$OUT"/$s/*/facts.txt; then echo "live-dbus: FAIL $s (scenario error, see $OUT/$s/*/facts.txt)"; rc=1; continue; fi
  if [ $have_ref = 1 ] && [ -f "$OUT/$s/rust/result.txt" ]; then
    if diff -u "$OUT/$s/ref/result.txt" "$OUT/$s/rust/result.txt" >"$OUT/$s/diff.txt"; then :; else
      echo "live-dbus: FAIL $s (original vs Rust)"; cat "$OUT/$s/diff.txt"; ok=0; fi
    if [ -n "${UPDATE_GOLDEN:-}" ] || [ ! -f "$golden" ]; then cut_bus "$OUT/$s/ref/result.txt" >"$golden"; fi
  fi
  if [ -f "$golden" ] && [ -f "$OUT/$s/rust/result.txt" ]; then
    if [ -n "${UPDATE_GOLDEN:-}" ] && [ $have_ref = 0 ]; then cut_bus "$OUT/$s/rust/result.txt" >"$golden"; fi
    if ! diff -u "$golden" <(cut_bus "$OUT/$s/rust/result.txt") >"$OUT/$s/diff-golden.txt"; then
      echo "live-dbus: FAIL $s (Rust vs recorded original, golden/$s.txt)"; cat "$OUT/$s/diff-golden.txt"; ok=0; fi
  fi
  if [ $ok = 1 ]; then
    echo "live-dbus: PASS $s$([ $have_ref = 1 ] && echo ' (original + golden)' || echo ' (golden only)')"
  else rc=1; fi
done
exit $rc
