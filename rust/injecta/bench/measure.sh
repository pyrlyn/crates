#!/usr/bin/env bash
# Clean build time and release binary size per implementation, plus lines of
# code. Run from bench/: `./measure.sh` (RUNS=n for more crate-only builds).
# Everything stays in bench/target.
set -euo pipefail
cd "$(dirname "$0")"
IMPLS=(baseline injecta nject shaku teloc dill)
RUNS=${RUNS:-3}

now() { perl -MTime::HiRes=time -e 'printf "%.3f\n", time'; }
elapsed() { perl -e "printf '%.2f', $2 - $1"; }
median() { printf '%s\n' "$@" | sort -n | awk '{a[NR]=$1} END {print a[int((NR+1)/2)]}'; }
# CPU seconds (user + sys) of a command: steadier than wall time on a busy machine.
cpu() { /usr/bin/time -p "$@" 2>&1 >/dev/null | awk '/^(user|sys)/ {t += $2} END {printf "%.2f", t}'; }
size_of() { stat -f %z "$1" 2>/dev/null || stat -c %s "$1"; }
# Non-blank, non-comment lines: whole file, and the wiring region between
# the `// --- wiring ---` and `// --- bench api ---` markers.
loc() { grep -v -E '^\s*(//.*)?$' "$1" | wc -l | tr -d ' '; }
wiring_loc() { awk '/\/\/ --- wiring ---/{on=1;next} /\/\/ --- bench api ---/{on=0} on' "$1" | grep -v -E '^\s*(//.*)?$' | wc -l | tr -d ' '; }

echo "impl,full_cold_s,full_cold_cpu_s,crate_only_clean_s,crate_only_cpu_s,touch_rebuild_s,bin_bytes,stripped_bytes,loc_total,loc_wiring"
for impl in "${IMPLS[@]}"; do
    pkg="impl-$impl"
    cargo clean -q
    t0=$(now); cargo build -q --release -p "$pkg"; t1=$(now)
    full=$(elapsed "$t0" "$t1")
    cargo clean -q
    full_cpu=$(cpu cargo build -q --release -p "$pkg")
    runs=()
    cpus=()
    for _ in $(seq "$RUNS"); do
        cargo clean -q --release -p "$pkg"
        t0=$(now); cargo build -q --release -p "$pkg"; t1=$(now)
        runs+=("$(elapsed "$t0" "$t1")")
        cargo clean -q --release -p "$pkg"
        cpus+=("$(cpu cargo build -q --release -p "$pkg")")
    done
    crate=$(median "${runs[@]}")
    crate_cpu=$(median "${cpus[@]}")
    touch "$pkg/src/lib.rs"
    t0=$(now); cargo build -q --release -p "$pkg"; t1=$(now)
    touched=$(elapsed "$t0" "$t1")
    bin="target/release/$pkg"
    strip -o "$bin.stripped" "$bin"
    echo "$impl,$full,$full_cpu,$crate,$crate_cpu,$touched,$(size_of "$bin"),$(size_of "$bin.stripped"),$(loc "$pkg/src/lib.rs"),$(wiring_loc "$pkg/src/lib.rs")"
done
