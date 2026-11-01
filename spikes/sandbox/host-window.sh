#!/bin/bash
# Usage: host-window.sh install | remove | snapshot FILE | compare FILE
# Holds what the spike does to the host's Hyprland. The rule and submap live
# only in the running compositor and never touch the developer's config files.
set -euo pipefail

W=${KEEK_WINDOW_W:-1280}
H=${KEEK_WINDOW_H:-800}

state() {
  python3 - <<'PY'
import hashlib, json, os, pathlib, subprocess

def ctl(*args):
    return json.loads(subprocess.check_output(["hyprctl", "-j", *args]))

config = pathlib.Path(os.path.expanduser("~/.config/hypr"))
digest = hashlib.sha256()
for path in sorted(config.rglob("*")):
    if path.is_file():
        digest.update(path.name.encode())
        digest.update(path.read_bytes())
print(json.dumps({
    "active_window": ctl("activewindow").get("address"),
    "active_workspace": ctl("activeworkspace").get("name"),
    "submap": subprocess.check_output(["hyprctl", "submap"], text=True).strip(),
    "windows": sorted(c["address"] for c in ctl("clients") if c["class"] != "aquamarine"),
    "config_sha256": digest.hexdigest(),
}, indent=1))
PY
}

case ${1:-} in
  install)
    hyprctl eval "if keek_spike_rule then keek_spike_rule:set_enabled(false) end"
    hyprctl eval "keek_spike_rule = hl.window_rule({ name = 'keek-spike-session-window', match = { class = '^aquamarine$' }, float = true, size = { $W, $H }, workspace = 'name:keek silent', no_initial_focus = true, no_focus = true, render_unfocused = true })"
    hyprctl eval "keek_spike_rule:set_enabled(true)"
    hyprctl eval "hl.define_submap('keek', function() hl.bind('SUPER + Escape', hl.dsp.submap('reset')) end)"
    ;;
  remove)
    hyprctl eval "if keek_spike_rule then keek_spike_rule:set_enabled(false) end"
    ;;
  snapshot)
    state >"$2"
    ;;
  compare)
    state | diff -u "$2" - && echo "host state unchanged"
    ;;
  *)
    echo "usage: host-window.sh install | remove | snapshot FILE | compare FILE" >&2
    exit 2
    ;;
esac
