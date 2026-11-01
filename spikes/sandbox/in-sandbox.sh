#!/bin/bash
# Usage: in-sandbox.sh CMD [ARG...]
# Runs a command in the running sandbox's namespaces with its Hyprland's
# environment. The spike runs one session at a time, so the first match is it.
set -euo pipefail

pid=$(pgrep -f '^Hyprland --config /keek/nested/hyprland.lua' | head -n 1)
if [ -z "$pid" ]; then
  echo "No sandboxed Hyprland is running. Start a session with proxy.py first." >&2
  exit 1
fi

mapfile -d '' environment <"/proc/$pid/environ"
exec nsenter --preserve-credentials -U -m -n -p -i -u -t "$pid" -- \
  env -i "${environment[@]}" sh -c '
    socket=$(find "$XDG_RUNTIME_DIR/hypr" -name .socket.sock | head -n 1)
    export HYPRLAND_INSTANCE_SIGNATURE=$(basename "$(dirname "$socket")")
    export WAYLAND_DISPLAY=wayland-1
    exec "$@"' sh "$@"
