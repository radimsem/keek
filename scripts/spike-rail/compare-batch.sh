#!/usr/bin/env bash
# Alternates a swap run with a control run that never moves the window, so both see the same desktop load.
# Usage: compare-batch.sh <client> <pairs>
set -uo pipefail
here="$(dirname "${BASH_SOURCE[0]}")"
out="$here/../../target/spike-rail"
client="$1"
pairs="$2"
for n in $(seq 1 "$pairs"); do
  CLIENT="$client" CONTROL=hidden LABEL="control$n" bash "$here/swap-test.sh" hidden_ws 10 1.2 1.8 >"$out/swap-hidden_ws-$client-control$n.log" 2>&1
  echo "control $n exit $?"
  CLIENT="$client" LABEL="swap$n" bash "$here/swap-test.sh" hidden_ws 10 1.2 1.8 >"$out/swap-hidden_ws-$client-swap$n.log" 2>&1
  echo "swap $n exit $?"
done
