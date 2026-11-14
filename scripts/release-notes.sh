#!/bin/sh
# Prints the CHANGELOG.md section of one version, without its heading.
# Usage: scripts/release-notes.sh 0.1.0
set -eu

version="${1:?Usage: scripts/release-notes.sh <version>}"

notes=$(awk -v heading="## $version" '
  $0 == heading || index($0, heading " ") == 1 { found = 1; next }
  found && /^## / { exit }
  found { print }
' CHANGELOG.md | sed '/./,$!d')

if [ -z "$notes" ]; then
  echo "CHANGELOG.md has no notes under \"## $version\". Add the section, then run this again." >&2
  exit 1
fi

printf '%s\n' "$notes"
