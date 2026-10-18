#!/usr/bin/env bash
set -euo pipefail
out="$(mktemp)"
trap 'rm -f "$out"' EXIT
GITHUB_OUTPUT="$out" bash scripts/release-current.sh patch shell-completions
expected="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "shell-completions") | .version')"
[ "$(cat "$out")" = "version=$expected" ]
: > "$out"
if GITHUB_OUTPUT="$out" bash scripts/release-current.sh patch no-such-workspace-crate; then
  echo 'unknown package must fail' >&2
  exit 1
fi
[ ! -s "$out" ]
