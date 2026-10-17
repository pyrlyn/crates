#!/usr/bin/env bash
# Publish every publishable workspace crate whose current version is not on crates.io yet,
# in dependency order (dependencies first). Needs CARGO_REGISTRY_TOKEN (the credentials
# file is written by the caller). DRY_RUN=1 only lists what would be published.
set -euo pipefail

meta="$(cargo metadata --no-deps --format-version 1)"

# name<TAB>version for publishable crates, topologically sorted over workspace-internal deps.
order="$(jq -r '
  (.packages | map(select(.publish != [])) ) as $pk
  | ($pk | map(.name)) as $names
  | $pk | map({name, version, deps: [.dependencies[] | select(.kind != "dev") | .name | select(. as $d | $names | index($d))]})
  | .[] | "\(.name)\t\(.version)\t\(.deps | join(","))"' <<<"$meta")"

done_=" "
published=""
skipped=""
remaining="$order"
while [ -n "$remaining" ]; do
  progress=0
  next=""
  while IFS=$'\t' read -r name version deps; do
    [ -n "$name" ] || continue
    ready=1
    for d in ${deps//,/ }; do
      [ -z "$d" ] || case "$done_" in *" $d "*) ;; *) ready=0 ;; esac
    done
    if [ "$ready" = 0 ]; then
      next+="$name"$'\t'"$version"$'\t'"$deps"$'\n'
      continue
    fi
    progress=1
    done_="$done_$name "
    code="$(curl -s -A "pyrlyn-crates-ci (https://github.com/pyrlyn/crates)" -o /dev/null -w '%{http_code}' \
      "https://crates.io/api/v1/crates/$name/$version")"
    if [ "$code" = 200 ]; then
      skipped="$skipped $name@$version"
      continue
    fi
    if [ "${DRY_RUN:-0}" = 1 ]; then
      echo "would publish $name@$version"
      continue
    fi
    echo "::group::publish $name@$version"
    set +e
    cargo publish -p "$name" --locked > publish.log 2>&1
    status=$?
    set -e
    cat publish.log
    echo "::endgroup::"
    if [ $status -ne 0 ] && ! grep -qE "already (published|exists)" publish.log; then
      echo "publish of $name@$version failed" >&2
      exit $status
    fi
    published="$published $name@$version"
  done <<<"$remaining"
  remaining="${next%$'\n'}"
  if [ "$progress" = 0 ] && [ -n "$remaining" ]; then
    echo "dependency cycle among: $remaining" >&2
    exit 1
  fi
done
echo "published: ${published:-none}"
echo "already on crates.io: ${skipped:-none}"
