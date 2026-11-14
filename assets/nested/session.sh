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

hyprctl plugin list -j >"$KEEK_SESSION/plugins.json"
hyprctl version -j >"$KEEK_SESSION/hyprland-version.json"

hyprctl eval "hl.exec_cmd('/keek/nested/start-app.sh')" >"$KEEK_SESSION/exec.log" 2>&1

(
  SECONDS=0
  shell_seen=false
  clear=0
  while [ "$SECONDS" -lt 30 ]; do
    if layers=$(hyprctl layers -j 2>/dev/null) && state=$(jq -er '
      [ .[] | .levels | to_entries[] | .key as $level | .value[] |
        {level: $level, namespace: (.namespace // "")} ] |
      [any(.level == "3" and .namespace == "omarchy-background"),
       any(.namespace | startswith("omarchy-"))] | @tsv
    ' <<<"$layers" 2>/dev/null); then
      read -r covered shell <<<"$state"
      [ "$shell" = true ] && shell_seen=true
      if [ "$covered" = false ] && { [ "$shell_seen" = true ] || [ "$SECONDS" -ge 3 ]; }; then
        clear=$((clear + 1))
      else
        clear=0
      fi
      # One clear sample could fall between the shell starting and its cover mapping.
      if [ "$clear" -ge 10 ]; then
        touch "$KEEK_SESSION/cover-clear"
        exit
      fi
    fi
    sleep 0.05
  done
) 8<&- 9>&- &

exec "$KEEK_DRIVER" mcp <&8 >&9 2>"$KEEK_SESSION/driver.log" 8<&- 9>&-
