#!/usr/bin/env bash
# Records a nested Hyprland's output for a few seconds in each named placement of its host window,
# then prints the host window's state and the recording's size and frame spacing.
# Usage: placement-matrix.sh <seconds> <placement>...
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

SECS="$1"
shift
HOST_BASE="$(host_snapshot)"
ACTIVE_WS="$(hyprctl activeworkspace -j | jq -r '.id')"
MON_W="$(hyprctl monitors -j | jq -r '.[0].width')"
MON_H="$(hyprctl monitors -j | jq -r '.[0].height')"

cleanup() {
  stop_recorder 2>/dev/null
  stop_client
  remove_rule
  stop_nested
  log "host after: $(host_snapshot)"
}
trap cleanup EXIT

move_to() { # workspace, x, y
  hl_eval "hl.dispatch(hl.dsp.window.move({ window = 'address:$WADDR', workspace = '$1', follow = false })); hl.dispatch(hl.dsp.window.move({ window = 'address:$WADDR', x = $2, y = $3, relative = false }))"
}

place() {
  case "$1" in
    hidden_ws) move_to "name:$HOSTWS" 20 20 ;;
    special_ws) move_to "special:$HOSTWS" 20 20 ;;
    shown) move_to "$ACTIVE_WS" 2000 40 ;;
    offscreen_right) move_to "$ACTIVE_WS" $((MON_W + 200)) 100 ;;
    offscreen_left) move_to "$ACTIVE_WS" -1500 100 ;;
    offscreen_below) move_to "$ACTIVE_WS" 100 $((MON_H + 200)) ;;
    alpha0)
      move_to "$ACTIVE_WS" 2000 40
      hl_eval "hl.dispatch(hl.dsp.window.set_prop({ window = 'address:$WADDR', prop = 'opacity', value = '0' }))"
      ;;
    *) log "unknown placement $1"; return 1 ;;
  esac
}

log "host before: $HOST_BASE (active workspace $ACTIVE_WS, monitor ${MON_W}x${MON_H})"
log "hyprland: $(hyprctl version | head -n 1)"
install_rule || exit 1
start_nested || exit 1
start_client
sleep 5
for p in "$@"; do
  host_guard || exit 3
  # Reset alpha from the previous placement.
  hl_eval "hl.dispatch(hl.dsp.window.set_prop({ window = 'address:$WADDR', prop = 'opacity', value = '1' }))"
  place "$p" || exit 1
  sleep 1.5
  host_guard || exit 3
  file="$OUT/matrix-$p.mp4"
  rm -f "$file"
  start_recorder "$file"
  sleep "$SECS"
  stop_recorder
  win="$(host_window_json | jq -c '{workspace: .workspace.name, at, size, floating, hidden, mapped}')"
  nf="$(hyprctl getprop "address:$WADDR" no_focus)"
  out="$(nctl monitors -j | jq -c '.[0] | {output: "\(.width)x\(.height)", refresh: .refreshRate}')"
  log "PLACEMENT $p host_window=$win no_focus=$nf nested_output=$out"
  log "PLACEMENT $p video=$(video_stats "$file" | tr '\n' ' ')"
done
