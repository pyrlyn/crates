#!/usr/bin/env bash
# Prerelease version step for bump.yml (pyrlyn/ci bump.yml `release-script`).
# Usage: scripts/prerelease.sh <level> <pre> <package>
#   level    patch | minor | major: how far past the last released version to go
#   pre      the semver prerelease suffix, e.g. alpha.1
#   package  the workspace crate to version
# Sets <package> to <next>-<pre>, rewrites the sibling `path` + `version`
# requirements on it (a caret on a prerelease also accepts the later stable),
# refreshes Cargo.lock, makes one commit and writes `version=` to
# $GITHUB_OUTPUT. A crate that was never tagged keeps its version as the base
# (its first release); a released one moves by <level>.
set -euo pipefail
level="$1" pre="$2" package="$3"
[[ "$pre" =~ ^[0-9A-Za-z.-]+$ ]] || { echo "bad prerelease suffix: $pre" >&2; exit 1; }
manifest="rust/$package/Cargo.toml"
[ -f "$manifest" ] || { echo "no $manifest" >&2; exit 1; }
current="$(cargo metadata --no-deps --format-version 1 \
  | jq -r --arg p "$package" '.packages[] | select(.name == $p) | .version')"
core="${current%%-*}"
if git ls-remote --exit-code --tags origin "refs/tags/$package-v$core" >/dev/null 2>&1; then
  IFS=. read -r major minor patch <<<"$core"
  case "$level" in
    major) core="$((major + 1)).0.0" ;;
    minor) core="$major.$((minor + 1)).0" ;;
    *) core="$major.$minor.$((patch + 1))" ;;
  esac
fi
version="$core-$pre"
python3 - "$package" "$version" <<'PY'
import glob, re, sys
package, version = sys.argv[1], sys.argv[2]
path = f"rust/{package}/Cargo.toml"
text = open(path).read()
text, n = re.subn(r'(?m)^version = "[^"]*"', f'version = "{version}"', text, count=1)
if n != 1:
    sys.exit(f"{path}: no package version line")
open(path, "w").write(text)
dep = re.compile(r'(?m)^(%s\s*=\s*\{[^}\n]*\bversion\s*=\s*")=?[^"]*(")' % re.escape(package))
for other in glob.glob("rust/*/Cargo.toml") + ["Cargo.toml"]:
    if other == path:
        continue
    t = open(other).read()
    t2 = dep.sub(lambda m: m.group(1) + ("=" if '"=' in m.group(0) else "") + version + m.group(2), t)
    if t2 != t:
        open(other, "w").write(t2)
        print(f"{other}: {package} -> {version}")
PY
cargo metadata --format-version 1 >/dev/null
git add -A
git commit --quiet -m "chore: release $package-v$version"
echo "version=$version" >>"${GITHUB_OUTPUT:-/dev/null}"
echo "$package $current -> $version"
