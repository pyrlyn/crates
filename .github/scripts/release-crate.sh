#!/usr/bin/env bash
# Checks that <crate> can be released from this checkout and prints
# `package=`, `version=`, `dir=` and `changelog=` lines for $GITHUB_OUTPUT.
# bump.yml runs it before anything is committed; release.yml runs it again on
# the tagged commit, so a bad crate name fails before a PR or a publish.
#
# usage: release-crate.sh <crate> [<expected-version>]
#
# CRATES_INDEX overrides the sparse index base URL (tests).
set -euo pipefail

crate="${1:?usage: release-crate.sh <crate> [<expected-version>]}"
expected="${2:-}"
index="${CRATES_INDEX:-https://index.crates.io}"

fail() {
  echo "error: $*" >&2
  exit 1
}

# The name ends up in a tag, a branch and a commit message.
[[ "$crate" =~ ^[A-Za-z0-9_-]+$ ]] || fail "'$crate' is not a crate name"

meta="$(cargo metadata --no-deps --format-version 1)"
pkg="$(jq -c --arg p "$crate" '[.packages[] | select(.name == $p)][0] // empty' <<<"$meta")"
[ -n "$pkg" ] || fail "$crate is not a workspace member (members: $(jq -r '[.packages[].name] | sort | join(", ")' <<<"$meta"))"

# `publish = false` is `[]` in the metadata; a list without crates-io means another registry.
jq -e '.publish == null or (.publish | index("crates-io") != null)' <<<"$pkg" >/dev/null \
  || fail "$crate is not publishable to crates.io (publish = false or another registry)"

version="$(jq -r .version <<<"$pkg")"
if [ -n "$expected" ] && [ "$expected" != "$version" ]; then
  fail "version $expected does not match $crate's Cargo.toml ($version)"
fi

root="$(jq -r .workspace_root <<<"$meta")"
dir="$(jq -r --arg root "$root/" '.manifest_path | ltrimstr($root) | sub("(^|/)Cargo\\.toml$"; "")' <<<"$pkg")"
[[ "$dir" =~ ^[A-Za-z0-9._/-]*$ ]] || fail "unexpected crate directory '$dir'"
if [ -z "$dir" ]; then
  changelog=CHANGELOG.md
  dir=.
else
  changelog="$dir/CHANGELOG.md"
fi

# crates.io resolves a crate's dependencies when it is published, so each
# workspace dependency must already be on the registry at the version built from
# this checkout. A version-less dev-dependency is dropped on publish.
published() {
  local name want="$2" prefix status
  name="$(tr '[:upper:]' '[:lower:]' <<<"$1")"
  case ${#name} in
    1) prefix=1 ;;
    2) prefix=2 ;;
    3) prefix="3/${name:0:1}" ;;
    *) prefix="${name:0:2}/${name:2:2}" ;;
  esac
  local body
  body="$(mktemp)"
  status="$(curl -sS --retry 3 -o "$body" -w '%{http_code}' "$index/$prefix/$name")" \
    || fail "cannot reach $index"
  if [ "$status" = 404 ]; then
    rm -f "$body"
    return 1
  fi
  [ "$status" = 200 ] || fail "$index/$prefix/$name answered HTTP $status"
  jq -e --arg v "$want" 'select(.vers == $v and (.yanked | not))' "$body" >/dev/null
  local found=$?
  rm -f "$body"
  return $found
}

missing=()
while IFS=$'\t' read -r dep req kind; do
  if [ "$req" = "*" ]; then
    [ "$kind" = dev ] && continue
    fail "$crate depends on $dep by path without a version: crates.io needs one"
  fi
  dep_json="$(jq -c --arg d "$dep" '[.packages[] | select(.name == $d)][0] // empty' <<<"$meta")"
  [ -n "$dep_json" ] || fail "$crate depends on $dep by path outside the workspace"
  jq -e '.publish == null or (.publish | index("crates-io") != null)' <<<"$dep_json" >/dev/null \
    || fail "$crate depends on $dep, which is not publishable"
  dep_version="$(jq -r .version <<<"$dep_json")"
  published "$dep" "$dep_version" || missing+=("$dep $dep_version")
done < <(jq -r '.dependencies[] | select(.source == null and .path != null) | [.name, .req, (.kind // "normal")] | @tsv' <<<"$pkg")

if [ "${#missing[@]}" -gt 0 ]; then
  printf 'error: %s needs these workspace crates on crates.io first:\n' "$crate" >&2
  printf '  %s\n' "${missing[@]}" >&2
  exit 1
fi

printf 'package=%s\nversion=%s\ndir=%s\nchangelog=%s\n' "$crate" "$version" "$dir" "$changelog"
