#!/bin/bash
set -u
mkdir -p "$KEEK_SESSION/probe"
touch "$KEEK_SESSION/probe/ready"
sequence=1
while :; do
    base="$KEEK_SESSION/probe/$sequence"
    if [[ -f "$base.cmd" ]]; then
        timeout 5 bash "$base.cmd" >"$base.out" 2>"$base.err"
        rc=$?
        printf '%s\n' "$rc" >"$base.rc.pending"
        mv "$base.rc.pending" "$base.rc"
        sequence=$((sequence + 1))
    else
        sleep 0.05
    fi
done
