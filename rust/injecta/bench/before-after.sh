#!/usr/bin/env bash
# Before/after check for a change to injecta's generated code: the divan
# resolve suite for injecta and the hand-written baseline, five crate-only
# release builds of impl-injecta (wall and CPU), and the binary size.
# Usage from bench/: ./before-after.sh <tag>  -> target/perf-<tag>.txt
# Touch the sources after copying files in: cargo's fingerprints are mtime based.
set -euo pipefail
cd "$(dirname "$0")"
tag=$1
mkdir -p target
out=target/perf-$tag.txt
size_of() { stat -f %z "$1" 2>/dev/null || stat -c %s "$1"; }
{
echo "== $tag $(date '+%F %T %Z') $(rustc --version)"
echo "load: $(uptime)"
cargo bench -q -p di-bench --bench resolve -- injecta baseline 2>&1 | tail -12
cargo build -q --release -p impl-injecta
for _ in 1 2 3 4 5; do
  cargo clean -q --release -p impl-injecta
  /usr/bin/time -p cargo build -q --release -p impl-injecta 2>&1 | awk '/^(user|sys)/ {t += $2} /^real/ {r=$2} END {printf "crate-only build: real %s cpu %.2f\n", r, t}'
done
strip -o target/release/impl-injecta.stripped target/release/impl-injecta
echo "bin $(size_of target/release/impl-injecta) stripped $(size_of target/release/impl-injecta.stripped)"
echo "load after: $(uptime)"
} > "$out" 2>&1
cat "$out"
