#!/usr/bin/env bash
# Shared helpers for the session rail spike (#88). Source this file, do not run it.
# A nested Hyprland stands in for a Session Window: same class, same host rule as keek's.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$ROOT/target/spike-rail"
RAW="$ROOT/scripts/spike-rail/output"
# Hyprland's sockets must fit sun_path (108 bytes), so the nested runtime folder is short.
NRUN="/run/user/$(id -u)/ks"
HOSTWS="keek-spike"
mkdir -p "$OUT" "$RAW"

log() { printf '%s %s\n' "$(date +%H:%M:%S.%3N)" "$*"; }

# Host Hyprland state the probes must leave as found: focused window, active workspace, window list.
host_snapshot() {
  printf 'active=%s ws=%s windows=%s\n' \
    "$(hyprctl activewindow -j | jq -r '.address // "none"')" \
    "$(hyprctl activeworkspace -j | jq -r '.id')" \
    "$(hyprctl clients -j | jq -r '[.[] | select(.class != "aquamarine")] | map(.address) | sort | join(",")')"
}

# Stop probing when the developer moves focus or workspace.
host_guard() {
  local now
  now="$(host_snapshot)"
  if [ "$now" != "$HOST_BASE" ]; then
    log "HOST CHANGED: was [$HOST_BASE] now [$now]; stopping"
    return 1
  fi
}

hl_eval() {
  local out
  out="$(hyprctl eval "$1")"
  [ "$out" = "ok" ] || { log "eval failed: $out"; return 1; }
}

# keek's rule, under another name and with a workspace of its own.
install_rule() {
  hl_eval "if keek_spike_rule then keek_spike_rule:set_enabled(false) end; keek_spike_rule = hl.window_rule({ name = 'keek-spike-window', match = { class = '^aquamarine\$' }, float = true, size = { 1280, 800 }, workspace = 'name:$HOSTWS silent', no_initial_focus = true, no_focus = true, render_unfocused = true }); keek_spike_rule:set_enabled(true)"
}

remove_rule() {
  hl_eval "if keek_spike_rule then keek_spike_rule:set_enabled(false) end" || true
}

host_window_json() { hyprctl clients -j | jq -c --argjson pid "$NPID" '.[] | select(.pid == $pid)'; }

# A nested Hyprland with a bare config. Sets NPID, NSIG, WADDR.
start_nested() {
  rm -rf "$NRUN"
  mkdir -p "$NRUN"
  chmod 700 "$NRUN"
  cat >"$OUT/bare.lua" <<'EOF'
hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })
hl.config({ xwayland = { enabled = false }, misc = { disable_watchdog_warning = true } })
EOF
  local host_display="$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
  XDG_RUNTIME_DIR="$NRUN" WAYLAND_DISPLAY="$host_display" \
    Hyprland --config "$OUT/bare.lua" >"$OUT/nested-hyprland.log" 2>&1 &
  NPID=$!
  local i
  for i in $(seq 1 100); do
    WADDR="$(host_window_json | jq -r '.address // empty')"
    [ -n "$WADDR" ] && break
    sleep 0.1
  done
  [ -n "$WADDR" ] || { log "nested window never mapped"; return 1; }
  NSIG="$(ls "$NRUN/hypr" | head -n 1)"
  log "nested Hyprland pid=$NPID window=$WADDR signature=$NSIG"
}

# hyprctl against the nested Hyprland.
nctl() { HYPRLAND_INSTANCE_SIGNATURE="$NSIG" XDG_RUNTIME_DIR="$NRUN" hyprctl "$@"; }

stop_nested() {
  if [ -n "${LOGLABEL:-}" ]; then
    cp "$NRUN"/hypr/*/hyprland.log "$OUT/nested-hyprland-$LOGLABEL.log" 2>/dev/null || true
  fi
  if [ -n "${NPID:-}" ]; then
    kill -TERM "$NPID" 2>/dev/null || true
    wait "$NPID" 2>/dev/null || true
  fi
  rm -rf "$NRUN"
}

# A client that draws a new frame every vsync, with a running timestamp on it.
start_client() {
  case "${CLIENT:-mpv}" in
    mpv)
      XDG_RUNTIME_DIR="$NRUN" WAYLAND_DISPLAY=wayland-1 \
        mpv --no-config --really-quiet --fs --loop=inf --vo=gpu --gpu-context=wayland \
        "av://lavfi:testsrc2=size=1280x800:rate=60" >"$OUT/client.log" 2>&1 &
      ;;
    foot)
      XDG_RUNTIME_DIR="$NRUN" WAYLAND_DISPLAY=wayland-1 \
        foot -e bash -c 'while :; do printf "\r%s" "$EPOCHREALTIME"; done' >"$OUT/client.log" 2>&1 &
      ;;
    chromium)
      XDG_RUNTIME_DIR="$NRUN" WAYLAND_DISPLAY=wayland-1 \
        chromium --ozone-platform=wayland --kiosk --no-first-run --no-default-browser-check --password-store=basic \
        --user-data-dir="$OUT/chromium-profile" "file://$ROOT/scripts/spike-rail/animate.html" >"$OUT/client.log" 2>&1 &
      ;;
  esac
  CPID=$!
}

stop_client() {
  [ -n "${CPID:-}" ] && { kill -TERM "$CPID" 2>/dev/null || true; wait "$CPID" 2>/dev/null || true; }
}

# wf-recorder the way Cua Driver runs it: every output frame, x264, yuv420p.
start_recorder() {
  XDG_RUNTIME_DIR="$NRUN" WAYLAND_DISPLAY=wayland-1 \
    wf-recorder --no-damage -c libx264 -x yuv420p -f "$1" >"$OUT/recorder.log" 2>&1 &
  RPID=$!
  sleep 1
}

stop_recorder() {
  kill -INT "$RPID" 2>/dev/null || true
  wait "$RPID" 2>/dev/null || true
}

# Frames per second of recording time, and every gap between frames longer than $2 ms.
video_buckets() {
  ffprobe -v error -select_streams v:0 -show_entries frame=pts_time -of csv=p=0 "$1" |
    awk -F, -v limit="$2" 'NF && $1 != "" { t = $1 + 0; b = int(t); c[b]++; if (NR > 1 && (t - p) * 1000 > limit) g = g sprintf(" %.3fs(+%.0fms)", p, (t - p) * 1000); p = t; last = b }
      END { s = ""; for (i = 0; i <= last; i++) s = s " " c[i] + 0; print "frames per second of video:" s; print "gaps over " limit " ms:" (g == "" ? " none" : g) }'
}

# Spans where frames come faster than 30 ms apart (the window was shown), as start+length in seconds.
video_fast_runs() {
  ffprobe -v error -select_streams v:0 -show_entries frame=pts_time -of csv=p=0 "$1" |
    awk -F, 'NF && $1 != "" { t = $1 + 0; if (NR > 1 && t - p < 0.03) { if (!run) { start = p; run = 1 } end = t } else if (run) { printf " %.2f+%.2f", start, end - start; run = 0 } p = t }
      END { if (run) printf " %.2f+%.2f", start, end - start; print "" }' | sed 's/^/shown spans (start+length, s):/'
}

# Size, frame count and frame spacing of one recording, from ffprobe.
video_stats() {
  local file="$1"
  ffprobe -v error -select_streams v:0 -show_entries stream=width,height,avg_frame_rate,r_frame_rate,nb_frames \
    -show_entries format=duration -of json "$file" |
    jq -c '{width: .streams[0].width, height: .streams[0].height, avg_frame_rate: .streams[0].avg_frame_rate, r_frame_rate: .streams[0].r_frame_rate, nb_frames: .streams[0].nb_frames, duration: .format.duration}'
  ffprobe -v error -select_streams v:0 -show_entries frame=pts_time -of csv=p=0 "$file" |
    awk -F, 'NF && $1 != "" { t[NR] = $1 + 0 } END {
      n = NR; if (n < 2) { print "{\"frames\":" n "}"; exit }
      max = 0; maxat = 0; sum = 0
      for (i = 2; i <= n; i++) { g = t[i] - t[i-1]; sum += g; if (g > max) { max = g; maxat = t[i-1] } }
      printf "{\"frames\":%d,\"span_s\":%.3f,\"mean_fps\":%.2f,\"max_gap_ms\":%.1f,\"max_gap_at_s\":%.3f}\n", n, t[n]-t[1], (n-1)/(t[n]-t[1]), max*1000, maxat
    }'
}
