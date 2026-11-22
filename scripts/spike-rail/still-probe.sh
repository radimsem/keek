#!/usr/bin/env bash
# Takes stills of a hidden nested window a few seconds apart while its output is recorded.
# Usage: still-probe.sh <path: nested|toplevel> <interval s> <control s> <stills s>
#   nested   grim on the nested Hyprland's own socket (wlr-screencopy of its output).
#   toplevel grim -T on the host, naming the Session Window by its Hyprland stable id (ext-image-copy-capture).
# The first <control s> seconds are recorded with no stills, the next <stills s> with one every <interval s>.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

PATHKIND="$1"
INTERVAL="$2"
CONTROL_S="$3"
STILLS_S="$4"
HOST_BASE="$(host_snapshot)"
LOGLABEL="still-$PATHKIND"
cleanup() {
  stop_recorder 2>/dev/null
  stop_client
  remove_rule
  stop_nested
  log "host after: $(host_snapshot)"
}
trap cleanup EXIT

# Mean luma and size of an image, so a black or empty still shows.
image_facts() {
  local luma
  luma="$(ffmpeg -hide_banner -nostats -i "$1" -vf signalstats,metadata=print:key=lavfi.signalstats.YAVG -an -f null - 2>&1 | grep -o 'YAVG=[0-9.]*' | head -n 1)"
  printf '%s %s bytes=%s' "$(ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=s=x:p=0 "$1")" "${luma:-YAVG=none}" "$(stat -c %s "$1")"
}

take_still() { # file
  local stable
  case "$PATHKIND" in
    nested)
      XDG_RUNTIME_DIR="$NRUN" WAYLAND_DISPLAY=wayland-1 grim -t jpeg -q 70 -s 0.5 "$1"
      ;;
    toplevel)
      stable="$(host_window_json | jq -r '.stableId')"
      grim -T "$stable" -t jpeg -q 70 -s 0.5 "$1"
      ;;
  esac
}

log "host before: $HOST_BASE; still path $PATHKIND every $INTERVAL s"
log "hyprland: $(hyprctl version | head -n 1); grim: $(grim -h 2>&1 | head -n 1 | cut -c1-40)"
log "loadavg before: $(cut -d' ' -f1-3 /proc/loadavg) on $(nproc) cores"
install_rule || exit 1
start_nested || exit 1
start_client
sleep 3
file="$OUT/$LOGLABEL.mp4"
rm -f "$file" "$OUT"/$LOGLABEL-still-*.jpg
start_recorder "$file"
log "window: $(host_window_json | jq -c '{ws: .workspace.name, size, stableId}')"
sleep "$CONTROL_S"
log "stills begin"
TIMEFORMAT='%R %U %S'
n=0
end=$(( $(date +%s) + STILLS_S ))
while [ "$(date +%s)" -lt "$end" ]; do
  n=$((n + 1))
  still="$OUT/$LOGLABEL-still-$n.jpg"
  cost="$( { time take_still "$still" >/dev/null 2>"$OUT/$LOGLABEL-grim.err"; } 2>&1 )"
  status=$?
  log "still $n exit=$status real/user/sys s=[$cost] $(image_facts "$still" 2>&1 | tr '\n' ' ')"
  host_guard || exit 3
  sleep "$INTERVAL"
done
stop_recorder
log "video=$(video_stats "$file" | tr '\n' ' ')"
video_buckets "$file" 120
[ -s "$OUT/$LOGLABEL-grim.err" ] && log "grim stderr: $(head -c 300 "$OUT/$LOGLABEL-grim.err")"
