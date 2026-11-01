#!/bin/bash
# Runs inside the sandbox, under init.sh. Everything but the driver logs to the
# session folder.
set -uo pipefail

# Only Hyprland sees WAYLAND_SOCKET. A bus-activated service that inherited it
# would take whatever its own fd 3 is for a compositor connection.
WAYLAND_SOCKET=3 Hyprland --config /keek/nested/hyprland.lua >"$KEEK_SESSION/hyprland.log" 2>&1 8<&- 9>&- &
hypr_pid=$!
exec 3<&-

for _ in $(seq 1 200); do
  socket=$(find "$XDG_RUNTIME_DIR/hypr" -name .socket.sock 2>/dev/null | head -n 1)
  [ -n "$socket" ] && break
  kill -0 "$hypr_pid" 2>/dev/null || { echo "Hyprland exited during start" >"$KEEK_SESSION/session.err"; exit 1; }
  sleep 0.05
done
if [ -z "${socket:-}" ]; then
  echo "Hyprland opened no control socket within 10 s" >"$KEEK_SESSION/session.err"
  exit 1
fi

export HYPRLAND_INSTANCE_SIGNATURE
HYPRLAND_INSTANCE_SIGNATURE=$(basename "$(dirname "$socket")")
export WAYLAND_DISPLAY
for _ in $(seq 1 100); do
  WAYLAND_DISPLAY=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -printf '%f\n' | head -n 1)
  [ -n "$WAYLAND_DISPLAY" ] && break
  sleep 0.05
done

python3 /keek/pages/server.py >"$KEEK_SESSION/page-server.log" 2>&1 8<&- 9>&- &

# Apps start through the nested Hyprland, so they get the environment a real
# Session Window's apps get.
hyprctl eval "hl.exec_cmd('/keek/nested/probe.sh >$KEEK_SESSION/probe.txt 2>&1')" >"$KEEK_SESSION/exec.log" 2>&1
hyprctl eval "hl.exec_cmd('chromium --ozone-platform=wayland --kiosk --no-first-run --no-default-browser-check --password-store=basic --user-data-dir=$HOME/chromium http://localhost:8765/$KEEK_PAGE.html >$KEEK_SESSION/chromium.log 2>&1')" >>"$KEEK_SESSION/exec.log" 2>&1

exec "$KEEK_DRIVER" mcp <&8 >&9 2>"$KEEK_SESSION/driver.log" 8<&- 9>&-
