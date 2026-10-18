#!/usr/bin/env bash
# Prints the body of one version's section of a release-plz changelog read from
# stdin (the lines under `## [1.2.3](...)`, up to the next `## `). Prints
# nothing when the version has no section.
#
# usage: changelog-section.sh <version> < <crate>/CHANGELOG.md
set -euo pipefail

version="${1:?usage: changelog-section.sh <version> < CHANGELOG.md}"

awk -v v="$version" '
  /^## / {
    if (on) exit
    h = $0
    sub(/^## \[?v?/, "", h)
    # 0.1.1 must not match 0.1.10 or 0.1.1-rc.1
    if (index(h, v) == 1 && substr(h, length(v) + 1, 1) !~ /[0-9A-Za-z.+-]/) { on = 1; next }
  }
  on { print }
'
