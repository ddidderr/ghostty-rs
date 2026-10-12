#!/usr/bin/env bash
# One scenario against one binary, executed inside `dbus-run-session`
# (see ../live-dbus-test.sh).  usage: scenario.sh <scenario> <bin> <outdir>
# Writes <outdir>/{bus.json,mock.jsonl,facts.txt,...}; normalize.py turns them
# into the comparable result.txt.
set -u
SCEN=$1; BIN=$2; D=$3
W=$LDB_RUN
LD=$LDB_ROOT/scripts/live-dbus
mkdir -p "$W/home" "$W/cfg/ghostty" "$W/data" "$W/cache"
export HOME=$W/home XDG_CONFIG_HOME=$W/cfg XDG_DATA_HOME=$W/data XDG_CACHE_HOME=$W/cache
export GSETTINGS_BACKEND=memory GDK_BACKEND=wayland LANG=C.UTF-8 SHELL=/bin/sh
export MOCK_LOG=$D/mock.jsonl
unset TERM_PROGRAM TERM_PROGRAM_VERSION GHOSTTY_BIN_DIR GHOSTTY_SHELL_FEATURES
export GHOSTTY_RESOURCES_DIR=${GHOSTTY_RESOURCES_DIR:-}
[ -z "$GHOSTTY_RESOURCES_DIR" ] && unset GHOSTTY_RESOURCES_DIR
FACTS=$D/facts.txt; : >"$FACTS"
fact() { echo "$*" >>"$FACTS"; }
PIDS=()
bg() { "$@" & PIDS+=($!); }
finish() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done
  for p in /proc/[0-9]*; do
    grep -qa "LDB_TAG=$LDB_TAG" "$p/environ" 2>/dev/null || continue
    case "$(cat "$p/comm" 2>/dev/null)" in dbus-*|bash|timeout) continue ;; esac
    kill "${p#/proc/}" 2>/dev/null
  done
  sleep 0.5
}
trap finish EXIT

cfg() { printf '%s\n' "shell-integration = none" "confirm-close-surface = false" \
  "quit-after-last-window-closed = false" "window-theme = auto" "$@" >"$W/cfg/ghostty/config"; }

start_bus_tools() { # [mock names]
  busctl --user monitor --json=short >"$D/bus.json" 2>"$D/monitor.err" &
  PIDS+=($!)
  MOCK_NAMES=${1:-notifications,systemd,portal} python3 -W ignore "$LD/mock.py" >"$D/mock.out" 2>&1 &
  PIDS+=($!)
  for _ in $(seq 50); do grep -q READY "$D/mock.out" && break; sleep 0.1; done
  sleep 0.5
}

# Wait until the app owns its bus name.
wait_name() { for _ in $(seq 300); do busctl --user list --acquired --no-pager 2>/dev/null | grep -q "^$1 " && return 0; sleep 0.1; done; return 1; }
mockctl() { gdbus call --session --dest test.ghostty.Mock --object-path /test/ghostty/Mock --method test.ghostty.Mock."$@" >/dev/null; }

# GUI window objects exported on the bus by GtkApplication.
windows() { # [bus name]
  # (never activate the service: only query a name that is already owned)
  busctl --user list --acquired --no-pager 2>/dev/null | grep -q "^${1:-com.mitchellh.ghostty} " || { echo 0; return; }
  busctl --user tree "${1:-com.mitchellh.ghostty}" --list --no-pager 2>/dev/null | grep -c '/window/[0-9]*$'
}
# Number of GUI instances of the binary under test (not +actions).
instances() {
  local n=0 p
  for p in /proc/[0-9]*; do
    [ "$(readlink "$p/exe" 2>/dev/null)" = "$(readlink -f "$BIN")" ] || continue
    tr '\0' ' ' <"$p/cmdline" | grep -q ' +' && continue
    n=$((n+1))
  done
  echo $n
}
# Command lines of terminal children (sh/sleep/...) of GUI instances, sorted.
children() {
  local p c
  for p in /proc/[0-9]*; do
    [ "$(readlink "$p/exe" 2>/dev/null)" = "$(readlink -f "$BIN")" ] || continue
    tr '\0' ' ' <"$p/cmdline" | grep -q ' +' && continue
    for c in $(pgrep -P "${p#/proc/}"); do
      echo "$(tr '\0' ' ' <"/proc/$c/cmdline") cwd=$(readlink /proc/$c/cwd)"
    done
  done | sort
}
# pids of processes with this comm that belong to this run
tagged() { local p; for p in /proc/[0-9]*; do
  [ "$(cat "$p/comm" 2>/dev/null)" = "$1" ] && grep -qa "LDB_TAG=$LDB_TAG" "$p/environ" 2>/dev/null && echo "${p#/proc/}"
done; }
settle() { sleep "${1:-2}"; }

run_ipc() { # label args...
  local label=$1; shift
  timeout 15 "$BIN" "$@" >"$D/ipc-$label.log" 2>&1
  fact "ipc $label rc=$?"
  grep -E "D-Bus|not valid|error" "$D/ipc-$label.log" | grep -v "^warning\|^info" | sed 's/^/  ipc-err: /' >>"$FACTS"
}

. "$LD/scenarios.sh"
"scn_${SCEN//-/_}"
