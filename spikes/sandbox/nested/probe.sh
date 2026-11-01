#!/bin/bash
# Verification steps 1 and 3 from inside, with the environment apps get.
echo '## env | grep -E "^(DISPLAY|DBUS_SESSION)"'
env | grep -E '^(DISPLAY|DBUS_SESSION)'
echo '## env | grep -E "^(WAYLAND|HYPRLAND|XDG_RUNTIME)"'
env | grep -E '^(WAYLAND|HYPRLAND|XDG_RUNTIME)'
echo '## busctl --user list (names with an owner)'
busctl --user list --no-legend | awk '$2 != "-" && $1 !~ /^:/ {print $1, $2, $3}'
echo '## readlink /proc/self/ns/net'
readlink /proc/self/ns/net
echo '## readlink /proc/self/ns/pid'
readlink /proc/self/ns/pid
echo '## runtime dir'
ls -A "$XDG_RUNTIME_DIR"
echo '## host sockets reachable?'
ls -d /run/dbus/system_bus_socket /tmp/.X11-unix "$XDG_RUNTIME_DIR/bus" 2>&1
echo '## hyprctl instances'
hyprctl instances 2>&1 | grep -E 'instance|wl socket'
echo '## write to HOME'
touch "$HOME/keek-probe" && echo "ok: wrote $HOME/keek-probe"
echo '## write to ~/.config/hypr'
touch "$HOME/.config/hypr/keek-probe"; echo "exit=$?"
echo '## write to ~/.config/omarchy'
touch "$HOME/.config/omarchy/keek-probe"; echo "exit=$?"
echo '## write to /usr'
touch /usr/keek-probe; echo "exit=$?"
echo '## write to session folder'
touch "$KEEK_SESSION/keek-probe" && echo ok
echo '## outbound internet'
curl -sS -o /dev/null -m 10 -w '%{http_code}\n' https://example.com 2>&1
echo '## developer home hidden?'
ls -A "$HOME"
ls -A "$(dirname "$HOME")"
