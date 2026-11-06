#!/usr/bin/env bash
# Drives the real Senzor GUI on a virtual X display (Xvfb), so it can be looked at and clicked
# without touching the desktop. See dev/README.md.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DISPLAY_NUM=${SENZOR_GUI_DISPLAY:-77}
SIZE=${SENZOR_GUI_SIZE:-1200x760}
STATE="$ROOT/target/gui-driver"
X=":$DISPLAY_NUM"

usage() {
  cat <<'EOF'
Usage: dev/gui.sh <command> [args]

  start [project.toml] [-- senzor args]   build and launch the GUI on the virtual display
  shot [out.png]                          screenshot (default target/gui-driver/shot.png)
  click X Y [button]                      click at window coordinates (button 1 left, 3 right)
  dclick X Y                              double-click
  drag X1 Y1 X2 Y2                        drag with the left button
  move X Y                                move the pointer (hover)
  scroll X Y up|down [n]                  scroll n steps (default 3) over X Y
  type TEXT                               type text into the focused widget
  key KEY...                              press keys, e.g. Return, Tab, ctrl+s, ctrl+a BackSpace
  close                                   ask the app to close, as the window's close button does,
                                          and wait for it to exit (it saves its settings)
  log                                     the app's output so far
  status                                  whether the display and the app are running
  stop                                    close the app and the display

Environment: SENZOR_GUI_DISPLAY (default 77), SENZOR_GUI_SIZE (default 1200x760, the window size).
Coordinates are pixels in the screenshot: the window fills the display, with no decorations.
EOF
}

pid_alive() { [[ -f "$1" ]] && kill -0 "$(cat "$1")" 2>/dev/null; }

need_app() {
  pid_alive "$STATE/app.pid" || { echo "the GUI is not running; use: dev/gui.sh start" >&2; exit 1; }
}

xdo() { DISPLAY=$X xdotool "$@"; }

start() {
  local project=()
  if [[ $# -gt 0 && $1 != -- ]]; then project=("$(realpath "$1")"); shift; fi
  [[ ${1:-} == -- ]] && shift
  if pid_alive "$STATE/app.pid"; then echo "already running; use: dev/gui.sh stop" >&2; exit 1; fi
  mkdir -p "$STATE"
  (cd "$ROOT" && cargo build -q -p senzor)

  if ! pid_alive "$STATE/xvfb.pid"; then
    Xvfb "$X" -screen 0 "${SIZE}x24" -nolisten tcp >"$STATE/xvfb.log" 2>&1 &
    echo $! >"$STATE/xvfb.pid"
    for _ in $(seq 50); do DISPLAY=$X xdotool getmouselocation >/dev/null 2>&1 && break; sleep 0.1; done
  fi

  # X11 only (no Wayland), its own D-Bus session so file dialogs open on the virtual display,
  # and a scratch HOME so the app's files stay out of the real one.
  mkdir -p "$STATE/home"
  env -u WAYLAND_DISPLAY DISPLAY=$X XDG_SESSION_TYPE=x11 HOME="$STATE/home" \
    XDG_CONFIG_HOME="$STATE/home/.config" XDG_DATA_HOME="$STATE/home/.local/share" \
    dbus-run-session -- "${CARGO_TARGET_DIR:-$ROOT/target}/debug/senzor" "${project[@]}" "$@" \
    >"$STATE/app.log" 2>&1 &
  echo $! >"$STATE/app.pid"

  # Wait for the window, then put it at the origin at the display's size.
  local win=""
  for _ in $(seq 100); do
    win=$(DISPLAY=$X xdotool search --name '^Senzor' 2>/dev/null | head -1 || true)
    [[ -n $win ]] && break
    pid_alive "$STATE/app.pid" || { echo "the GUI exited:" >&2; cat "$STATE/app.log" >&2; exit 1; }
    sleep 0.1
  done
  [[ -n $win ]] || { echo "no window after 10 s; see dev/gui.sh log" >&2; exit 1; }
  # With no window manager nothing gives the window keyboard focus, and typing goes nowhere.
  # The window can be found before it is viewable; focusing it then fails, so retry.
  xdo windowmove "$win" 0 0 windowsize "$win" "${SIZE%x*}" "${SIZE#*x}"
  for _ in $(seq 50); do xdo windowfocus --sync "$win" 2>/dev/null && break; sleep 0.1; done
  sleep 1
  echo "running on $X (window $win)"
}

shot() {
  need_app
  local out=${1:-$STATE/shot.png}
  sleep 0.3 # let egui paint the last input
  import -display "$X" -window root "$out"
  echo "$out"
}

stop() {
  for f in app xvfb; do
    if pid_alive "$STATE/$f.pid"; then kill "$(cat "$STATE/$f.pid")" 2>/dev/null || true; fi
    rm -f "$STATE/$f.pid"
  done
  pkill -f "senzor.*$STATE" 2>/dev/null || true
  echo stopped
}

cmd=${1:-help}
shift || true
case $cmd in
  start) start "$@" ;;
  shot) shot "$@" ;;
  click) need_app; xdo mousemove "$1" "$2" sleep 0.05 click "${3:-1}" ;;
  dclick) need_app; xdo mousemove "$1" "$2" sleep 0.05 click --repeat 2 --delay 80 1 ;;
  drag) need_app; xdo mousemove "$1" "$2" mousedown 1 sleep 0.1 mousemove "$3" "$4" sleep 0.1 mouseup 1 ;;
  move) need_app; xdo mousemove "$1" "$2" ;;
  scroll)
    need_app
    b=5; [[ $3 == up ]] && b=4
    xdo mousemove "$1" "$2" click --repeat "${4:-3}" --delay 50 "$b" ;;
  type) need_app; xdo type --delay 20 -- "$*" ;;
  key) need_app; xdo key --delay 50 "$@" ;;
  close)
    need_app
    # With no window manager, xdotool windowquit reaches nobody: send WM_DELETE_WINDOW directly.
    DISPLAY=$X python3 -I - "$(DISPLAY=$X xdotool search --name '^Senzor' | head -1)" <<'PY'
import sys
from Xlib import X, display, protocol
d = display.Display()
w = d.create_resource_object("window", int(sys.argv[1]))
event = protocol.event.ClientMessage(
    window=w, client_type=d.intern_atom("WM_PROTOCOLS"),
    data=(32, [d.intern_atom("WM_DELETE_WINDOW"), X.CurrentTime, 0, 0, 0]))
w.send_event(event, event_mask=X.NoEventMask)
# sync, not flush: exiting right after a flush sometimes loses the request.
d.sync()
PY
    for _ in $(seq 100); do pid_alive "$STATE/app.pid" || break; sleep 0.1; done
    if pid_alive "$STATE/app.pid"; then echo "still running: a prompt may be open; see shot" >&2; exit 1; fi
    rm -f "$STATE/app.pid"
    echo closed ;;
  log) cat "$STATE/app.log" ;;
  status)
    pid_alive "$STATE/xvfb.pid" && echo "display $X: running" || echo "display $X: stopped"
    pid_alive "$STATE/app.pid" && echo "GUI: running" || echo "GUI: stopped" ;;
  stop) stop ;;
  *) usage ;;
esac
