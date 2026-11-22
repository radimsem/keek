#!/usr/bin/env bash
# Copies the small raw logs and sample stills of a spike run from target/ into output/, which is committed.
# Videos stay in target/ because they are large.
set -uo pipefail
here="$(dirname "${BASH_SOURCE[0]}")"
from="$here/../../target/spike-rail"
to="$here/output"
mkdir -p "$to"
for name in \
  matrix-1.log \
  swap-hidden_ws.log \
  swap-hidden_ws-mpv-a.log \
  swap-hidden_ws-mpv-r1.log \
  swap-hidden_ws-mpv-r2.log \
  swap-hidden_ws-mpv-r3.log \
  swap-hidden_ws-mpv-control1.log \
  swap-hidden_ws-mpv-swap1.log \
  swap-hidden_ws-mpv-control2.log \
  swap-hidden_ws-chromium.log \
  swap-hidden_ws-chromium-b.log \
  still-nested-1.log \
  still-toplevel-1.log \
  still-nested-2.log \
  still-toplevel-2.log \
  live-1.log \
  still-toplevel-still-3.jpg \
  still-nested-still-3.jpg; do
  if [ -f "$from/$name" ]; then
    case "$name" in
      # The live test log carries ANSI colour codes from tracing.
      *.log) sed 's/\x1b\[[0-9;]*m//g' "$from/$name" >"$to/$name" ;;
      *) cp "$from/$name" "$to/$name" ;;
    esac
  fi
done
