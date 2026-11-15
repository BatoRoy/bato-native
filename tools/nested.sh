#!/usr/bin/env bash
# tools/nested.sh: a hidden, nested Wayland session for tests that need real input.
#
# The off-screen output (offscreen.sh) lives inside the user's own Hyprland, so
# nothing may click or type there. This session is fully separate:
#
#   kwin_wayland --virtual (no window anywhere)  →  Hyprland nested in it  →  the app
#
# with its own XDG_RUNTIME_DIR, HOME, config and D-Bus, started from `env -i`.
# Clipboard, primary selection, keyboard focus and drag-and-drop are real, so
# paste, shortcuts and file drops can be exercised end to end. Nothing in here
# can reach the desktop session: hyprctl, wl-copy and grim talk to the nested
# sockets only.
#
#   tools/nested.sh up                         start the session (and build the driver)
#   tools/nested.sh check                      exit 0 if it is up
#   tools/nested.sh run [K=V …] BIN [ARGS…]    launch an app in it (fresh XDG_CONFIG_HOME), wait $WAIT s (3)
#   tools/nested.sh stop                       kill the apps started with `run`
#   tools/nested.sh drive STEP…                pointer script (below)
#   tools/nested.sh key MODS,KEY [WINDOW]      send a shortcut to a window, e.g. key CTRL,V  or  key ,x
#   tools/nested.sh focus [WINDOW]             focus a window
#   tools/nested.sh shot OUT.png [X Y W H]     screenshot the output, or a region of it
#   tools/nested.sh exec CMD…                  run any command inside the session (wl-copy, wl-paste, hyprctl …)
#   tools/nested.sh xwayland                   start a rootful, fullscreen Xwayland in it on
#                                              :$BATO_NESTED_XDISPLAY (default 87); prints DISPLAY
#   tools/nested.sh down                       stop everything and delete the session
#
# WINDOW is a Hyprland window selector; default class:batopoc-nested, the
# app_id `run` gives apps that read BATO_APP_ID. Coordinates are output pixels
# ($NESTED_W×$NESTED_H, default 1600×1000, scale 1).
#
# drive steps (one argument each):
#   "move X Y" · "click X Y [middle]" · "press [middle]" · "release [middle]" · "wheel N"
#   "sleep MS" · "drag PATH X Y" (a real text/uri-list drag from a source in the
#   bottom-left corner; once per call) · "shot OUT.png [X Y W H]" · "exec CMD …"
#
# Drop targets that log what they receive (start them with `run`, WAIT=1):
#   run tools/wldriver/target/release/wldriver target X Y W H SECS    Wayland (layer surface)
#   run DISPLAY=:87 tools/wldriver/target/release/wldriver xtarget X Y W H SECS   X11 (XDND)
#   REFUSE=1 refuses the drop, ACTION=move prefers move.
#
# X11 apps: `xwayland`, then `run WAYLAND_DISPLAY= DISPLAY=:87 BIN` (an empty
# WAYLAND_DISPLAY makes winit pick X11). Its root covers the output, so
# coordinates are the same.
#
# Example: paste into a text field
#   tools/nested.sh up
#   BATO_VIEW='view=examples&tab=inputs&card=text-fields' tools/nested.sh run target/release/gallery
#   tools/nested.sh exec wl-copy "hello"
#   tools/nested.sh drive "click 700 300"
#   tools/nested.sh key CTRL,V
#   tools/nested.sh shot /tmp/x/paste.png
#   tools/nested.sh down
#
# State lives in $BATO_NESTED_DIR (default /tmp/bato-nested-$UID), the sockets in
# $BATO_NESTED_RUN (default /tmp/bn-$UID; at most 26 characters).
# Needs kwin_wayland, Hyprland, dbus-run-session, grim, wl-clipboard and cargo.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
D="${BATO_NESTED_DIR:-${TMPDIR:-/tmp}/bato-nested-$(id -u)}"
NW="${NESTED_W:-1600}"
NH="${NESTED_H:-1000}"
# The runtime dir holds Hyprland's sockets at $R/hypr/<61-char signature>/.socket2.sock,
# and a Unix socket path must fit in 107 bytes, so $R stays short and apart from $D.
R="${BATO_NESTED_RUN:-/tmp/bn-$(id -u)}"
DRIVER_DIR="$ROOT/tools/wldriver"
DRIVER="$DRIVER_DIR/target/release/wldriver"
WINDOW_DEFAULT="class:batopoc-nested"

die() { echo "nested: $*" >&2; exit 1; }
alive() { [[ -f "$D/$1.pid" ]] && kill -0 "$(cat "$D/$1.pid")" 2>/dev/null; }

# Run a command inside the nested Hyprland, with nothing from the caller's
# environment except the variables passed as K=V before the command.
inside() {
  [[ -f "$D/sig" ]] || die "not up (tools/nested.sh up)"
  env -i bash -c 'source "$1/env.sh"
    export WAYLAND_DISPLAY=wayland-1 HYPRLAND_INSTANCE_SIGNATURE="$(cat "$1/sig")"
    shift; exec env "$@"' _ "$D" "$@"
}

cmd_up() {
  if alive host && alive hypr && [[ -f "$D/sig" ]]; then echo "nested: already up ($D)"; return; fi
  for b in kwin_wayland Hyprland dbus-run-session grim wl-copy hyprctl cargo; do
    command -v "$b" >/dev/null || die "missing $b"
  done
  [[ ${#R} -le 26 ]] || die "BATO_NESTED_RUN ($R) must be at most 26 characters (socket paths)"
  cmd_down >/dev/null 2>&1 || true
  mkdir -p "$D"/{home/data,home/cache,home/state,cfg} "$R"
  chmod 700 "$D" "$R"
  cat > "$D/env.sh" <<EOF
export HOME=$D/home XDG_RUNTIME_DIR=$R XDG_CONFIG_HOME=$D/cfg
export XDG_DATA_HOME=$D/home/data XDG_CACHE_HOME=$D/home/cache XDG_STATE_HOME=$D/home/state
export PATH=/usr/local/bin:/usr/bin:/bin LANG=${LANG:-C.UTF-8}
export LIBSEAT_BACKEND=seatd SEATD_SOCK=$D/no-seatd.sock
export HYPRLAND_NO_SD_NOTIFY=1 HYPRLAND_NO_SD_VARS=1 HYPRLAND_NO_CRASHREPORTER=1
EOF
  # Hyprland >= 0.57 reads Lua; a hyprlang file trips its emergency mode (default 1280x720 at scale 2).
  cat > "$D/hyprland.lua" <<EOF
hl.monitor({ output = "", mode = "${NW}x${NH}@60", position = "0x0", scale = 1 })
hl.config({
  xwayland = { enabled = false },
  misc = { disable_hyprland_logo = true, disable_splash_rendering = true, force_default_wallpaper = 0 },
  animations = { enabled = false },
  ecosystem = { no_update_news = true, no_donation_nag = true },
})
EOF

  # The host: a windowless KWin, on its own D-Bus.
  setsid env -i bash -c 'source "$1/env.sh"; export QT_QPA_PLATFORM=offscreen
    exec dbus-run-session -- kwin_wayland --virtual --no-lockscreen --socket wl-host --width "$2" --height "$3"' \
    _ "$D" "$NW" "$NH" > "$D/host.log" 2>&1 < /dev/null &
  echo $! > "$D/host.pid"
  for _ in $(seq 100); do [[ -S "$R/wl-host" ]] && break; sleep 0.1; done
  [[ -S "$R/wl-host" ]] || { cmd_down; die "kwin_wayland did not start; see $D/host.log"; }

  # Hyprland, nested in it.
  setsid env -i bash -c 'source "$1/env.sh"; export WAYLAND_DISPLAY=wl-host
    exec Hyprland --config "$1/hyprland.lua"' _ "$D" > "$D/hypr.log" 2>&1 < /dev/null &
  echo $! > "$D/hypr.pid"
  local sig=""
  for _ in $(seq 150); do
    sig="$(find "$R/hypr" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | head -1 || true)"
    [[ -n "$sig" && -S "$R/hypr/$sig/.socket.sock" && -S "$R/wayland-1" ]] && break
    sleep 0.1
  done
  [[ -n "$sig" && -S "$R/wayland-1" ]] || { cmd_down; die "Hyprland did not start; see $D/hypr.log"; }
  echo "$sig" > "$D/sig"

  if [[ ! -x "$DRIVER" ]]; then
    echo "nested: building the driver"
    nice cargo build --release --quiet --manifest-path "$DRIVER_DIR/Cargo.toml"
  fi
  inside hyprctl dismissnotify >/dev/null 2>&1 || true   # the "started without start-hyprland" banner
  echo "nested: up (${NW}x${NH}, state in $D)"
}

cmd_check() {
  if alive host && alive hypr && [[ -S "$R/wayland-1" ]]; then echo "nested: ok"; else echo "nested: down"; return 1; fi
}

cmd_run() {
  cmd_check >/dev/null || die "not up (tools/nested.sh up)"
  local vars=()
  while [[ $# -gt 0 && "$1" == *=* ]]; do vars+=("$1"); shift; done
  [[ $# -gt 0 ]] || die "run: no binary"
  local bin; bin="$(realpath "$1")"; shift
  # BATO_* from the caller (BATO_VIEW, BATO_SELFTEST …) pass through too.
  while IFS= read -r v; do vars+=("$v"); done < <(env | grep '^BATO_' || true)
  local n=0; while [[ -e "$D/app-$n.pid" ]]; do n=$((n + 1)); done
  local cfg="$D/app-cfg-$n"; rm -rf "$cfg"; mkdir -p "$cfg"
  setsid env -i bash -c 'source "$1/env.sh"
    export WAYLAND_DISPLAY=wayland-1 HYPRLAND_INSTANCE_SIGNATURE="$(cat "$1/sig")"
    shift; exec env "$@"' _ "$D" XDG_CONFIG_HOME="$cfg" BATO_APP_ID=batopoc-nested \
    RUST_LOG="${RUST_LOG:-warn}" "${vars[@]}" "$bin" "$@" > "$D/app-$n.log" 2>&1 < /dev/null &
  echo $! > "$D/app-$n.pid"
  sleep "${WAIT:-3}"
  kill -0 "$(cat "$D/app-$n.pid")" 2>/dev/null || die "the app exited; see $D/app-$n.log"
  echo "nested: started pid $(cat "$D/app-$n.pid"), log $D/app-$n.log"
}

cmd_stop() {
  local f
  for f in "$D"/app-*.pid; do
    [[ -f "$f" ]] || continue
    kill "$(cat "$f")" 2>/dev/null || true
    rm -f "$f"
  done
}

cmd_drive() {
  [[ -x "$DRIVER" ]] || nice cargo build --release --quiet --manifest-path "$DRIVER_DIR/Cargo.toml"
  inside OUT_W="$NW" OUT_H="$NH" timeout 120 "$DRIVER" script "$@" 2>&1 | grep -v '^driver: exec' || true
}

# Hyprland >= 0.57 takes Lua dispatchers; older ones the classic syntax.
dispatch() {
  local out
  out="$(inside hyprctl dispatch "$1" 2>&1)"
  [[ "$out" == ok* ]] || out="$(inside hyprctl dispatch $2 2>&1)"
  [[ "$out" == ok* ]] || die "hyprctl dispatch failed: $out"
}

cmd_key() {
  [[ $# -ge 1 && "$1" == *,* ]] || die "key: MODS,KEY [WINDOW]   e.g. key CTRL,V  or  key ,x"
  local mods="${1%%,*}" k="${1#*,}" w="${2:-$WINDOW_DEFAULT}"
  dispatch "hl.dsp.send_shortcut({ mods = \"$mods\", key = \"$k\", window = \"$w\" })" "sendshortcut $1,$w"
  sleep 0.25
}

cmd_focus() {
  local w="${1:-$WINDOW_DEFAULT}"
  dispatch "hl.dsp.focus({ window = \"$w\" })" "focuswindow $w"
  sleep 0.3
}

cmd_shot() {
  [[ $# -ge 1 ]] || die "shot: OUT.png [X Y W H]"
  local out; out="$(realpath -m "$1")"; mkdir -p "$(dirname "$out")"
  if [[ $# -ge 5 ]]; then inside grim -g "$2,$3 ${4}x${5}" "$out"; else inside grim "$out"; fi
  echo "$out"
}

cmd_xwayland() {
  cmd_check >/dev/null || die "not up (tools/nested.sh up)"
  local n="${BATO_NESTED_XDISPLAY:-87}"
  if alive xwayland; then echo "DISPLAY=:$n"; return; fi
  [[ ! -e "/tmp/.X11-unix/X$n" ]] || die "display :$n is taken; set BATO_NESTED_XDISPLAY"
  command -v Xwayland >/dev/null || die "missing Xwayland"
  setsid env -i bash -c 'source "$1/env.sh"
    export WAYLAND_DISPLAY=wayland-1 HYPRLAND_INSTANCE_SIGNATURE="$(cat "$1/sig")"
    exec Xwayland ":$2" -fullscreen -geometry "$3" -noreset -nolisten tcp' _ "$D" "$n" "${NW}x${NH}" > "$D/xwayland.log" 2>&1 < /dev/null &
  echo $! > "$D/xwayland.pid"
  for _ in $(seq 50); do [[ -S "/tmp/.X11-unix/X$n" ]] && break; sleep 0.1; done
  [[ -S "/tmp/.X11-unix/X$n" ]] || die "Xwayland did not start; see $D/xwayland.log"
  sleep 0.5
  echo "DISPLAY=:$n"
}

cmd_down() {
  cmd_stop
  if alive xwayland; then kill "$(cat "$D/xwayland.pid")" 2>/dev/null || true; fi
  local p
  for p in hypr host; do
    if alive "$p"; then
      # setsid made each a process-group leader: take the whole group (dbus-run-session, kwin, Hyprland).
      kill -- "-$(cat "$D/$p.pid")" 2>/dev/null || kill "$(cat "$D/$p.pid")" 2>/dev/null || true
    fi
  done
  sleep 0.5
  [[ -d "$R" && -f "$D/env.sh" ]] && rm -rf "$R"
  [[ -d "$D" && "$D" == *bato-nested* ]] && rm -rf "$D"
  echo "nested: down"
}

case "${1:-}" in
  up) cmd_up ;;
  check) cmd_check ;;
  run) shift; cmd_run "$@" ;;
  stop) cmd_stop ;;
  drive) shift; cmd_drive "$@" ;;
  key) shift; cmd_key "$@" ;;
  focus) shift; cmd_focus "$@" ;;
  shot) shift; cmd_shot "$@" ;;
  exec) shift; inside "$@" ;;
  xwayland) cmd_xwayland ;;
  down) cmd_down ;;
  *) sed -n '2,57p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
