#!/bin/bash
cd "$KEEK_WORKTREE" || exit 1
exec ./.keek/setup.sh start >"$KEEK_SESSION/start.log" 2>&1
