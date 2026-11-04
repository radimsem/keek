#!/bin/bash
cd "$KEEK_WORKTREE" || exit 1
exec /usr/bin/bash -c "$KEEK_START" >"$KEEK_SESSION/start.log" 2>&1
