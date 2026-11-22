#!/usr/bin/env bash
# Size, frame spacing and frozen spans of one or more recordings.
# Usage: analyze.sh <file.mp4>...
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
for file in "$@"; do
  log "== $file"
  video_stats "$file"
  video_buckets "$file" 120
  video_fast_runs "$file"
  log "frozen spans of 0.2 s or more (freezedetect n=-60dB):"
  ffmpeg -hide_banner -nostats -i "$file" -vf freezedetect=n=-60dB:d=0.2 -an -f null - 2>&1 |
    grep -E "freeze_(start|duration)" | sed 's/.*lavfi.freezedetect.//' | paste -sd' ' - || true
done
