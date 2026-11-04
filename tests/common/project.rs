fn fixture_script(forwarded: u16, blocked: u16) -> String {
    let lan_probe = include_str!("project_lan.py");
    format!(
        r#"#!/bin/bash
set -e
case "$1" in
install)
    echo 1 >> counter
    pwd > install-cwd
    python3 - <<'PY'
import socket
with socket.create_connection(('127.0.0.1', {forwarded}), timeout=2) as stream:
    stream.sendall(b'install\n')
PY
    ;;
start)
    python3 - <<'PY'
import json, os, pathlib, socket
ports = []
for port in [{forwarded}, {blocked}]:
    try:
        with socket.create_connection(('127.0.0.1', port), timeout=2) as stream:
            stream.sendall(b'probe\n')
        ports.append(True)
    except OSError:
        ports.append(False)
result = dict(install_cwd=pathlib.Path('install-cwd').read_text().strip(), cwd=os.getcwd(), project=os.environ['KEEK_PROJECT'], counter=len(pathlib.Path('counter').read_text().splitlines()), ignored=pathlib.Path('.env').exists(), untracked=pathlib.Path('untracked').read_text(), source=pathlib.Path('source').read_text(), marker=pathlib.Path('session-marker').exists(), ports=ports)
{lan_probe}
pathlib.Path('session-marker').write_text('throwaway')
folder = pathlib.Path(os.environ['KEEK_SESSION'])
(folder/'probes.tmp').write_text(json.dumps(result))
(folder/'probes.tmp').rename(folder/'probes.txt')
PY
    python3 -m http.server 8000 --bind 127.0.0.1 &
    exec chromium --ozone-platform=wayland --kiosk --no-first-run --no-default-browser-check --password-store=basic --user-data-dir="$HOME/chromium" http://127.0.0.1:8000
    ;;
esac
"#
    )
}
