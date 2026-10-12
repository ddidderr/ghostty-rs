# Scenario bodies, sourced by scenario.sh.

scn_single_instance() {
  cfg "gtk-single-instance = true"
  start_bus_tools
  # (A launch with -e is never single-instance: see the "-e launch" step.)
  bg env GHOSTTY_DEBUG_INPUT_SCRIPT="$LD/replay-windows.txt" "$BIN" >"$D/app1.log" 2>&1
  wait_name com.mitchellh.ghostty; fact "first instance owns name: $?"
  settle 4; fact "step first: windows=$(windows) instances=$(instances)"
  run_ipc newwin +new-window; settle
  fact "step +new-window: windows=$(windows) instances=$(instances)"
  run_ipc newwin-e +new-window -e sleep 601; settle
  fact "step +new-window -e: windows=$(windows) instances=$(instances)"
  run_ipc newwin-wd +new-window --working-directory=/usr; settle
  fact "step +new-window --working-directory: windows=$(windows) instances=$(instances)"
  run_ipc newwin-e-wd +new-window -e sleep 602 --working-directory=/usr; settle
  fact "step +new-window -e..--working-directory: windows=$(windows) instances=$(instances)"
  run_ipc newwin-class +new-window --class=com.example.nobody; settle 1
  fact "step +new-window --class=nobody: windows=$(windows) instances=$(instances)"
  # A second GUI launch is forwarded to the first instance and exits.
  timeout 20 "$BIN" >"$D/second.log" 2>&1; fact "second launch rc=$?"
  settle; fact "step second launch: windows=$(windows) instances=$(instances)"
  children | sed 's/^/child: /' >>"$FACTS"
  # A launch with -e is not single-instance: it runs on its own.
  bg timeout 8 "$BIN" -e sleep 603 >"$D/dash-e.log" 2>&1
  settle 5; fact "step -e launch: windows=$(windows) instances=$(instances)"
  settle 5; fact "step -e launch ended: windows=$(windows) instances=$(instances)"
  # Custom class: a separate instance on its own bus name / object path.
  bg "$BIN" --class=com.example.custom-app >"$D/app2.log" 2>&1
  wait_name com.example.custom-app; fact "custom class owns name: $?"
  settle 4; fact "step custom: windows=$(windows com.example.custom-app) instances=$(instances)"
  run_ipc newwin-custom +new-window --class=com.example.custom-app; settle
  fact "step +new-window --class=custom: custom windows=$(windows com.example.custom-app) default windows=$(windows)"
  run_ipc newwin-custom-e +new-window --class=com.example.custom-app -e sleep 605; settle
  fact "step +new-window --class=custom -e: custom windows=$(windows com.example.custom-app) default windows=$(windows)"
  run_ipc newwin-badclass +new-window --class=1bad; fact "badclass done"
  busctl --user list --acquired --no-pager | awk '$1 ~ /^(com\.mitchellh|com\.example)/ {print "bus name: "$1}' >>"$FACTS"
  gdbus call --session --dest com.mitchellh.ghostty --object-path /com/mitchellh/ghostty \
    --method org.gtk.Actions.List 2>&1 | tr -d "'" >"$D/actions.txt"; fact "actions: $(cat "$D/actions.txt")"
  gdbus call --session --dest com.example.custom-app --object-path /com/example/custom_app \
    --method org.gtk.Actions.List 2>&1 | tr -d "'" >"$D/actions2.txt"; fact "custom actions: $(cat "$D/actions2.txt")"
}

scn_single_instance_detect() {
  cfg "gtk-single-instance = detect"
  start_bus_tools
  bg "$BIN" >"$D/app1.log" 2>&1
  wait_name com.mitchellh.ghostty; fact "plain launch owns name: $?"
  settle 4; fact "step plain: windows=$(windows) instances=$(instances)"
  timeout 20 "$BIN" >"$D/second.log" 2>&1; fact "second plain launch rc=$?"
  settle; fact "step second plain: windows=$(windows) instances=$(instances)"
  # TERM_PROGRAM set: a dedicated instance (false) that does not use the bus name.
  bg env TERM_PROGRAM=x timeout 12 "$BIN" >"$D/termprog.log" 2>&1
  settle 6; fact "step TERM_PROGRAM: windows=$(windows) instances=$(instances)"
  settle 7; fact "step TERM_PROGRAM ended: windows=$(windows) instances=$(instances)"
  # Any CLI argument: dedicated instance.
  bg timeout 12 "$BIN" --title=cli >"$D/cliarg.log" 2>&1
  settle 6; fact "step CLI arg: windows=$(windows) instances=$(instances)"
  settle 7; fact "step CLI arg ended: windows=$(windows) instances=$(instances)"
  run_ipc newwin +new-window; settle
  fact "step +new-window: windows=$(windows) instances=$(instances)"
}

scn_notifications() {
  export FLAG2=$W/go2
  rm -f "$FLAG2"
  cfg "desktop-notifications = true" "notify-on-command-finished = always" "notify-on-command-finished-after = 1s" \
    "command = sh $LD/child-notify.sh"
  start_bus_tools
  bg "$BIN" >"$D/app.log" 2>&1
  settle 16
  fact "notifications so far: $(grep -c "\"method\": \"Notify\"" "$MOCK_LOG" 2>/dev/null || true)"
  # Activate the first notifications (default action): must not crash.
  mockctl Invoke 1 default; settle 2
  fact "alive after ActionInvoked: instances=$(instances) windows=$(windows)"
  mockctl Invoke 2 default; mockctl Invoke 999 default; settle 1
  fact "alive after more ActionInvoked: instances=$(instances) windows=$(windows)"
  cfg "desktop-notifications = false" "command = sh $LD/child-notify.sh"
  gdbus call --session --dest com.mitchellh.ghostty --object-path /com/mitchellh/ghostty \
    --method org.gtk.Actions.Activate reload-config '[]' '{}' >/dev/null; settle 2
  touch "$FLAG2"; settle 3
  fact "notifications after desktop-notifications=false: $(grep -c "\"method\": \"Notify\"" "$MOCK_LOG" 2>/dev/null || true)"
}

scn_shortcuts() {
  cfg "keybind = global:ctrl+alt+x=new_window" "keybind = global:super+shift+b=new_tab" \
    "keybind = global:ctrl+shift+f12=reload_config" "keybind = global:alt+space>q=quit" \
    "keybind = global:ctrl+alt+q=text:hi" "keybind = ctrl+alt+z=new_window"
  start_bus_tools
  bg "$BIN" >"$D/app.log" 2>&1
  wait_name com.mitchellh.ghostty; settle 5
  fact "step start: windows=$(windows)"
  id=$(python3 - "$MOCK_LOG" <<'PY'
import json, sys
for l in open(sys.argv[1]):
    j = json.loads(l)
    if j["method"] == "BindShortcuts":
        for sid, opts in j["args"][1]:
            if opts.get("description", "").startswith("new_window"):
                print(sid)
PY
)
  fact "new_window shortcut id: $id"
  mockctl Activate "$id"; settle 3
  fact "step Activated(new_window): windows=$(windows)"
  mockctl Activate nonexistent; settle 1
  fact "step Activated(unknown): windows=$(windows)"
  # Reload with the keybinds removed: the old session is closed.
  cfg "keybind = ctrl+alt+z=new_window"
  gdbus call --session --dest com.mitchellh.ghostty --object-path /com/mitchellh/ghostty \
    --method org.gtk.Actions.Activate reload-config '[]' '{}' >/dev/null; settle 3
  fact "step reload without global: windows=$(windows)"
  cfg "keybind = global:ctrl+alt+x=new_window"
  gdbus call --session --dest com.mitchellh.ghostty --object-path /com/mitchellh/ghostty \
    --method org.gtk.Actions.Activate reload-config '[]' '{}' >/dev/null; settle 3
  fact "step reload with global again: windows=$(windows)"
}

scn_settings() {
  mkdir -p "$W/themes"
  printf 'background = ffeedd\nforeground = 111111\n' >"$W/themes/light"
  printf 'background = 112233\nforeground = eeeeee\n' >"$W/themes/dark"
  cfg "theme = light:$W/themes/light,dark:$W/themes/dark" "window-theme = auto" "command = sh $LD/child-theme.sh"
  MOCK_COLOR_SCHEME=1 start_bus_tools
  export OUTF=$D/tty.txt
  : >"$OUTF"
  bg "$BIN" >"$D/app.log" 2>&1
  for _ in $(seq 600); do [ -s "$OUTF" ] && break; sleep 0.1; done
  [ -s "$OUTF" ] || fact "ERROR: terminal child produced no output"
  settle 3
  for scheme in 2 1 0 2; do
    printf "\n--- after portal color-scheme=$scheme\n" >>"$OUTF"
    mockctl SetColorScheme $scheme; settle 2
  done
  printf "\n--- after reload window-theme=dark\n" >>"$OUTF"
  cfg "theme = light:$W/themes/light,dark:$W/themes/dark" "window-theme = dark" "command = sh $LD/child-theme.sh"
  gdbus call --session --dest com.mitchellh.ghostty --object-path /com/mitchellh/ghostty \
    --method org.gtk.Actions.Activate reload-config '[]' '{}' >/dev/null; settle 2
  printf "\n--- after portal color-scheme=2 with window-theme=dark\n" >>"$OUTF"
  mockctl SetColorScheme 2; settle 2
  python3 "$LD/summarize-tty.py" "$OUTF" | sed 's/^/tty: /' >>"$FACTS"
}

scn_cgroup() {
  start_bus_tools
  phase() { # name, config lines...
    local name=$1; shift
    : >"$MOCK_LOG"
    cfg "$@"
    bg "$BIN" -e sleep 30 >"$D/app-$name.log" 2>&1
    local pid=$!
    # wait for the terminal child (the scope is created right before it is exec'd)
    for _ in $(seq 250); do pgrep -P $pid -x sleep >/dev/null && break; sleep 0.1; done
    settle 3
    fact "phase $name:"
    grep StartTransientUnit "$MOCK_LOG" | sed 's/^/  call: /' >>"$FACTS"
    kill $pid 2>/dev/null
    for _ in $(seq 100); do kill -0 $pid 2>/dev/null || break; sleep 0.1; done
    kill -9 $pid 2>/dev/null; kill $(tagged sleep) 2>/dev/null; settle 1
  }
  phase default "gtk-single-instance = false"
  phase always "gtk-single-instance = false" "linux-cgroup = always"
  phase always-limits "gtk-single-instance = false" "linux-cgroup = always" "linux-cgroup-memory-limit = 123456789" "linux-cgroup-processes-limit = 77"
  phase never "gtk-single-instance = false" "linux-cgroup = never"
  mockctl SetFail true
  phase always-failing "gtk-single-instance = false" "linux-cgroup = always"
  fact "cgroup done"
}

scn_openuri() {
  # Ghostty 1.3.1 does not use the OpenURI portal (upstream TODO): links and
  # the config file are opened by spawning `xdg-open`.  A fake xdg-open on
  # PATH records the invocations; the mock records any portal OpenURI call.
  mkdir -p "$W/bin"
  cat >"$W/bin/xdg-open" <<XDG
#!/bin/sh
printf 'xdg-open argc=%s' "\$#" >>"$D/xdg-open.log"
for a in "\$@"; do printf ' [%s]' "\$a" >>"$D/xdg-open.log"; done
printf ' cwd=%s\n' "\$(pwd)" >>"$D/xdg-open.log"
XDG
  chmod +x "$W/bin/xdg-open"
  export PATH=$W/bin:$PATH GTK_USE_PORTAL=1
  : >"$D/xdg-open.log"
  cfg "gtk-single-instance = true"
  start_bus_tools
  bg "$BIN" >"$D/app.log" 2>&1
  wait_name com.mitchellh.ghostty; settle 5
  gdbus call --session --dest com.mitchellh.ghostty --object-path /com/mitchellh/ghostty \
    --method org.gtk.Actions.Activate open-config '[]' '{}' >/dev/null; settle 3
  sed "s|$W|\$RUN|g" "$D/xdg-open.log" | sed 's/^/spawn: /' >>"$FACTS"
  fact "portal OpenURI/OpenFile calls: $(grep -c 'portal.OpenURI' "$MOCK_LOG")"
}

scn_openurl_click() {
  # The original has no input replay and weston's headless pointer is not
  # reachable through Xwayland, so this scenario only runs for the Rust
  # build; its result is compared against golden/openurl-click.txt, which is
  # derived from the Zig source (internal_os/open.zig spawns `xdg-open <url>`).
  [ "${LDB_LABEL:-}" = rust ] || return
  mkdir -p "$W/bin"
  cat >"$W/bin/xdg-open" <<XDG
#!/bin/sh
printf 'xdg-open argc=%s' "\$#" >>"$D/xdg-open.log"
for a in "\$@"; do printf ' [%s]' "\$a" >>"$D/xdg-open.log"; done
printf '\n' >>"$D/xdg-open.log"
XDG
  chmod +x "$W/bin/xdg-open"
  export PATH=$W/bin:$PATH GDK_BACKEND=x11 OUTF=$D/tty.txt
  : >"$D/xdg-open.log"; : >"$OUTF"
  cfg "gtk-single-instance = false" "gtk-titlebar = false" "window-padding-x = 0" "window-padding-y = 0" \
    "font-size = 14" "window-decoration = none" "command = sh $LD/child-links.sh" "window-width = 60" "window-height = 12"
  start_bus_tools
  # Launch 1 measures the cell size.
  bg "$BIN" >"$D/app-measure.log" 2>&1
  local pid=$!
  settle 7
  rows=$(grep -o '\^\[\[8;[0-9]*;[0-9]*t' "$OUTF" | head -1 | cut -d';' -f2)
  cols=$(grep -o '\^\[\[8;[0-9]*;[0-9]*t' "$OUTF" | head -1 | cut -d';' -f3 | tr -d t)
  wid=$(xdotool search --onlyvisible --class ghostty 2>/dev/null | head -1)
  eval "$(xdotool getwindowgeometry --shell "$wid" 2>/dev/null)"
  kill $pid; settle 1
  [ -n "${WIDTH:-}" ] && [ -n "${rows:-}" ] || { fact "no geometry"; return; }
  cw=$((WIDTH / cols)); ch=$((HEIGHT / rows))
  {
    for row in 0 1 2 3; do
      echo "mouse-move $((cw * 3)) $((row * ch + ch / 2)) ctrl"
      echo "wait 600"
      echo "click 1 ctrl"
      echo "wait 1500"
    done
    echo "wait 500"
  } >"$W/click.txt"
  : >"$D/xdg-open.log"; : >"$OUTF"
  bg env GHOSTTY_DEBUG_INPUT_SCRIPT="$W/click.txt" "$BIN" >"$D/app.log" 2>&1
  settle 16
  sed "s|$W|\$RUN|g" "$D/xdg-open.log" | sed 's/^/spawn: /' >>"$FACTS"
}

# The real xdg-desktop-portal frontend + the gtk backend (gsettings keyfile
# backend, so we can change color-scheme live) on the private bus.
scn_real_portal() {
  mkdir -p "$W/cfg/xdg-desktop-portal" "$W/cfg/glib-2.0/settings" "$W/themes"
  printf '[preferred]\ndefault=gtk\n' >"$W/cfg/xdg-desktop-portal/portals.conf"
  portal_env="/usr/bin/env XDG_CURRENT_DESKTOP=GNOME GSETTINGS_BACKEND=keyfile XDG_CONFIG_HOME=$W/cfg HOME=$W/home XDG_DATA_HOME=$W/data XDG_CACHE_HOME=$W/cache WAYLAND_DISPLAY=$WAYLAND_DISPLAY"
  svc() { printf '[D-BUS Service]\nName=%s\nExec=%s\n' "$1" "$2" >"$LDB_RUN/services/$1.service"; }
  svc org.freedesktop.portal.Desktop "$portal_env /usr/lib/xdg-desktop-portal"
  svc org.freedesktop.impl.portal.desktop.gtk "$portal_env /usr/lib/xdg-desktop-portal-gtk"
  svc org.freedesktop.impl.portal.PermissionStore "$portal_env /usr/lib/xdg-permission-store"
  printf 'background = ffeedd\nforeground = 111111\n' >"$W/themes/light"
  printf 'background = 112233\nforeground = eeeeee\n' >"$W/themes/dark"
  keyfile=$W/cfg/glib-2.0/settings/keyfile
  setscheme() { printf '[org/gnome/desktop/interface]\ncolor-scheme='"'"'%s'"'"'\n' "$1" >"$keyfile"; }
  setscheme prefer-dark
  cfg "theme = light:$W/themes/light,dark:$W/themes/dark" "window-theme = auto" "command = sh $LD/child-theme.sh" \
    "keybind = global:ctrl+alt+x=new_window"
  start_bus_tools notifications,systemd
  # Start the portal first so that its backends are up.
  gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.DBus.Properties.Get org.freedesktop.portal.Settings version >"$D/portal-version.txt" 2>&1
  fact "portal frontend: $(tr -d '\n' <"$D/portal-version.txt" | cut -c1-80)"
  settle 2
  export OUTF=$D/tty.txt; : >"$OUTF"
  bg env GSETTINGS_BACKEND=memory "$BIN" >"$D/app.log" 2>&1
  for _ in $(seq 600); do [ -s "$OUTF" ] && break; sleep 0.1; done
  [ -s "$OUTF" ] || fact "ERROR: terminal child produced no output"
  settle 3
  for scheme in prefer-light prefer-dark default prefer-dark; do
    printf "\n--- after gsettings color-scheme=$scheme\n" >>"$OUTF"
    setscheme $scheme; settle 3
  done
  python3 "$LD/summarize-tty.py" "$OUTF" | sed 's/^/tty: /' >>"$FACTS"
  # Only the D-Bus error names (the log prefixes differ between the builds).
  sed -n 's/.*request failed=\(GDBus.Error:[A-Za-z.]*\).*/\1/p' "$D/app.log" | sed 's/^/global-shortcuts request error: /' >>"$FACTS"
  fact "alive: instances=$(instances)"
}
