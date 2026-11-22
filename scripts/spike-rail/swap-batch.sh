#!/usr/bin/env bash
# Runs swap-test.sh several times so one quiet run does not stand for the desktop's usual load.
# Usage: swap-batch.sh <client: mpv|chromium|foot> <hidden placement> <runs>
set -uo pipefail
here="$(dirname "${BASH_SOURCE[0]}")"
client="$1"
placement="$2"
runs="$3"
for n in $(seq 1 "$runs"); do
  CLIENT="$client" LABEL="r$n" bash "$here/swap-test.sh" "$placement" 10 1.2 1.8 >"$here/../../target/spike-rail/swap-$placement-$client-r$n.log" 2>&1
  code=$?
  echo "run $n ($client, $placement) exit $code"
  grep -E "CHECK FAILED|HOST CHANGED" "$here/../../target/spike-rail/swap-$placement-$client-r$n.log"
  [ "$code" -eq 0 ] || break
done
