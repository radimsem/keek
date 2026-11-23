#!/bin/bash
exec 7<>"$1" || exit 1
while IFS= read -r -u 7 line; do
  if [[ $line =~ ^copy\ ([0-9]+)$ ]]; then
    # --sensitive offers x-kde-passwordManagerHint, which Omarchy skips in
    # $OMARCHY_PATH/shell/plugins/clipboard/capture.sh to keep text out of clipboard history.
    head -c "${BASH_REMATCH[1]}" <&7 7<&- | wl-copy --sensitive --type 'text/plain;charset=utf-8' 7<&-
  elif [[ $line == clear ]]; then
    wl-copy --clear 7<&-
  fi
done
