#!/usr/bin/env bash
# Swaps a Session Window between shown (on the active workspace) and hidden (a workspace nobody views)
# while the nested output is recorded, and checks that the recording stays continuous.
# Usage: swap-test.sh <hidden placement: hidden_ws|special_ws> <round trips> <shown seconds> <hidden seconds>
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

HIDDEN="$1"
TRIPS="$2"
SHOWN_S="$3"
HIDDEN_S="$4"
HOST_BASE="$(host_snapshot)"
ACTIVE_WS="$(hyprctl activeworkspace -j | jq -r '.id')"

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

hide() {
  case "$HIDDEN" in
    hidden_ws) move_to "name:$HOSTWS" 20 20 ;;
    special_ws) move_to "special:$HOSTWS" 20 20 ;;
  esac
}
show() { move_to "$ACTIVE_WS" 2000 40; }
# CONTROL=hidden runs the same schedule without moving the window, to tell swap stalls from load stalls.
if [ "${CONTROL:-}" = hidden ]; then show() { :; }; fi

check() { # label
  local nf out win
  nf="$(hyprctl getprop "address:$WADDR" no_focus)"
  out="$(nctl monitors -j | jq -r '.[0] | "\(.width)x\(.height)"')"
  win="$(host_window_json | jq -c '{ws: .workspace.name, size, floating}')"
  log "$1 no_focus=$nf nested_output=$out host_window=$win"
  [ "$nf" = "true" ] && [ "$out" = "1280x800" ] || { log "CHECK FAILED"; return 1; }
  host_guard
}

log "host before: $HOST_BASE; hidden placement $HIDDEN; $TRIPS round trips"
log "hyprland: $(hyprctl version | head -n 1)"
log "loadavg before: $(cut -d' ' -f1-3 /proc/loadavg) on $(nproc) cores"
install_rule || exit 1
start_nested || exit 1
start_client
sleep 5
hide
sleep 1.5
LOGLABEL="swap-$HIDDEN-${CLIENT:-mpv}${LABEL:+-$LABEL}"
file="$OUT/$LOGLABEL.mp4"
rm -f "$file"
start_recorder "$file"
T0="$(date +%s.%N)"
rel() { awk -v a="$(date +%s.%N)" -v b="$T0" 'BEGIN { printf "%.3f", a - b }'; }
sleep "$HIDDEN_S"
for n in $(seq 1 "$TRIPS"); do
  s="$(date +%s%N)"
  show || exit 1
  e="$(date +%s%N)"
  log "t=$(rel)s swap $n show took $(( (e - s) / 1000000 )) ms"
  check "after show $n" || exit 3
  sleep "$SHOWN_S"
  s="$(date +%s%N)"
  hide || exit 1
  e="$(date +%s%N)"
  log "t=$(rel)s swap $n hide took $(( (e - s) / 1000000 )) ms"
  check "after hide $n" || exit 3
  sleep "$HIDDEN_S"
done
stop_recorder
log "loadavg after: $(cut -d' ' -f1-3 /proc/loadavg)"
log "video=$(video_stats "$file" | tr '\n' ' ')"
video_buckets "$file" 120
video_fast_runs "$file"
log "freezedetect (n=-60dB, d=0.2 s):"
ffmpeg -hide_banner -nostats -i "$file" -vf freezedetect=n=-60dB:d=0.2 -an -f null - 2>&1 | grep -E "freeze_(start|end|duration)" || echo "no frozen span of 0.2 s or more"
