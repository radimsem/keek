#!/bin/bash
# Usage: sandbox.sh SESSION_DIR
# Starts the Agent Session sandbox. stdin and stdout carry `cua-driver mcp`.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
session=$1
helper=$here/helper/target/release/keek-sandbox-helper
driver_dir=$HOME/.cua-driver/packages/releases/0.38.0-x86_64-unknown-linux-gnu
uid=$(id -u)
gid=$(id -g)
runtime=/run/user/$uid

# The nested shell reads its idle settings from shell.json, so a session gets a
# copy with the screensaver and lock turned off.
python3 "$here/nested/shell-json.py" "$HOME/.config/omarchy/shell.json" >"$session/shell.json"

# pasta forwards this address to the host's resolver. The host's own
# resolv.conf names 127.0.0.53, which the private network can't reach.
dns=169.254.1.1
printf 'nameserver %s\n' "$dns" >"$session/resolv.conf"

# NVIDIA's EGL driver opens these beside /dev/dri, and Hyprland aborts without them.
gpu_devices=()
for device in /dev/nvidia*; do
  [ -e "$device" ] && gpu_devices+=(--dev-bind "$device" "$device")
done

# pasta closes every inherited descriptor above stderr, so the helper has to
# open the Wayland connection from inside pasta. By default pasta forwards every
# port between the namespaces and maps the gateway to the host's loopback, so
# the sandbox needs all of that off to have its own localhost.
exec pasta -f -q --config-net --dns-forward "$dns" --no-map-gw \
  -t none -u none -T none -U none -- \
  "$helper" connect-exec -- \
  bwrap \
  --unshare-user --uid "$uid" --gid "$gid" \
  --unshare-pid --unshare-ipc --unshare-uts \
  --die-with-parent --new-session --clearenv \
  --setenv HOME "$HOME" \
  --setenv USER "$USER" \
  --setenv LANG "${LANG:-C.UTF-8}" \
  --setenv PATH "/keek/nested/bin:/usr/share/omarchy/bin:/usr/bin" \
  --setenv OMARCHY_PATH /usr/share/omarchy \
  --setenv XDG_RUNTIME_DIR "$runtime" \
  --setenv XDG_SESSION_TYPE wayland \
  --setenv XDG_CURRENT_DESKTOP Hyprland \
  --setenv ATSPI_DBUS_IMPLEMENTATION dbus-daemon \
  --setenv KEEK_SESSION "$session" \
  --setenv KEEK_PAGE "${KEEK_PAGE:-signin}" \
  --setenv KEEK_DRIVER "$driver_dir/cua-driver" \
  --setenv CUA_DRIVER_RS_ENABLE_WAYLAND 1 \
  --setenv CUA_DRIVER_RS_TELEMETRY_ENABLED false \
  --ro-bind /usr /usr \
  --symlink usr/bin /bin \
  --symlink usr/bin /sbin \
  --symlink usr/lib /lib \
  --symlink usr/lib /lib64 \
  --ro-bind /etc /etc \
  --dir /run/systemd/resolve \
  --ro-bind "$session/resolv.conf" /run/systemd/resolve/stub-resolv.conf \
  --dev /dev \
  --dev-bind /dev/dri /dev/dri \
  "${gpu_devices[@]}" \
  --ro-bind /sys /sys \
  --proc /proc \
  --tmpfs /tmp \
  --tmpfs "$HOME" \
  --perms 0700 --tmpfs "$runtime" \
  --ro-bind "$HOME/.config/hypr" "$HOME/.config/hypr" \
  --ro-bind "$HOME/.config/omarchy" "$HOME/.config/omarchy" \
  --ro-bind "$session/shell.json" "$HOME/.config/omarchy/shell.json" \
  --ro-bind-try "$HOME/.local/state/omarchy/current" "$HOME/.local/state/omarchy/current" \
  --ro-bind-try "$HOME/.local/state/omarchy/toggles" "$HOME/.local/state/omarchy/toggles" \
  --ro-bind-try "$HOME/.local/state/omarchy/settings" "$HOME/.local/state/omarchy/settings" \
  --bind "$session" "$session" \
  --ro-bind "$driver_dir" "$driver_dir" \
  --ro-bind "$here" /keek \
  /keek/nested/init.sh
