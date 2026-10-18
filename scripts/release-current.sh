#!/usr/bin/env bash
# The shared bump workflow needs a version step even when releasing an unchanged version.
# Avoid planning bumps for unrelated workspace crates in release-untagged-head mode.
set -euo pipefail
package="${2:?workspace package required}"
version="$(cargo metadata --no-deps --format-version 1 \
  | jq -er --arg p "$package" '.packages[] | select(.name == $p) | .version')"
printf 'version=%s\n' "$version" >> "${GITHUB_OUTPUT:?GITHUB_OUTPUT required}"
