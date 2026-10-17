#!/usr/bin/env bash
# bench.sh - how much wall-clock time does scoped-check save on the current change?
#
# Usage (from the repository root that holds scoped-check.toml):
#   path/to/scoped-check/scripts/bench.sh [BASE]
#
#   BASE  ref or commit to compare against. Default: the merge-base of HEAD with
#         origin/main, or with main when origin/main does not exist.
#
# It times two runs over the same working tree and prints both times, the difference and
# the percentage saved:
#   1. scoped-check run --all  --base BASE   every gate, full-workspace values
#   2. scoped-check run        --base BASE   only the gates and packages the change touches
#
# WARNING: this runs the REAL gates from scoped-check.toml, twice (builds, tests, linters,
# whatever they are). Expect it to take as long as your CI. A failing gate does not abort
# the timing: its exit code is reported next to the time, and a run that failed early
# is not a fair "time saved" figure, so the summary says so.
#
# Environment: SCOPED_CHECK  path to the scoped-check binary (default: scoped-check on PATH).
# Needs bash (any version with EPOCHREALTIME, or perl, or python3 for the timer) and git.

set -u

SCOPED_CHECK=${SCOPED_CHECK:-scoped-check}

die() { printf 'bench.sh: %s\n' "$*" >&2; exit 2; }

[ -f scoped-check.toml ] || die "run me from the repository root that has scoped-check.toml"
command -v "$SCOPED_CHECK" >/dev/null 2>&1 || die "$SCOPED_CHECK not found (cargo install --path scoped-check, or set SCOPED_CHECK)"

# Portable seconds-with-fraction clock: macOS's BSD date has no %N, so use bash's
# EPOCHREALTIME (bash 5), else perl's Time::HiRes, else python3.
now() {
  if [ -n "${EPOCHREALTIME:-}" ]; then
    # Some locales print a comma as the decimal separator.
    printf '%s\n' "${EPOCHREALTIME/,/.}"
  elif command -v perl >/dev/null 2>&1; then
    perl -MTime::HiRes=time -e 'printf "%.6f\n", time'
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import time; print("%.6f" % time.time())'
  else
    die "no timer available: need bash 5, perl or python3"
  fi
}

base=${1:-}
if [ -z "$base" ]; then
  upstream=origin/main
  git rev-parse --verify --quiet "$upstream^{commit}" >/dev/null || upstream=main
  base=$(git merge-base HEAD "$upstream") \
    || die "cannot find a merge-base with $upstream (shallow clone? pass BASE explicitly)"
fi

printf 'bench.sh: base %s\n' "$base"
printf 'bench.sh: WARNING: running the real gates, twice\n\n'

# timed LABEL ARGS... : run scoped-check, set the globals SECONDS_TAKEN and CODE.
timed() {
  local label=$1
  shift
  printf '== %s: %s %s\n' "$label" "$SCOPED_CHECK" "$*"
  local start end
  start=$(now)
  "$SCOPED_CHECK" "$@"
  CODE=$?
  end=$(now)
  SECONDS_TAKEN=$(awk -v s="$start" -v e="$end" 'BEGIN { printf "%.3f", e - s }')
  printf '== %s: exit %s after %ss\n\n' "$label" "$CODE" "$SECONDS_TAKEN"
}

timed "all gates" run --all --base "$base"
all_time=$SECONDS_TAKEN
all_code=$CODE

timed "scoped" run --base "$base"
scoped_time=$SECONDS_TAKEN
scoped_code=$CODE

awk -v a="$all_time" -v s="$scoped_time" -v ac="$all_code" -v sc="$scoped_code" 'BEGIN {
  printf "all gates : %8.3f s  (exit %d)\n", a, ac
  printf "scoped    : %8.3f s  (exit %d)\n", s, sc
  printf "saved     : %8.3f s", a - s
  if (a > 0) printf "  (%.1f%%)", (a - s) / a * 100
  printf "\n"
  if (ac != 0 || sc != 0)
    print "note: a gate failed, so a run may have stopped early; fix it before trusting the numbers"
}'

# The script itself succeeds when it measured; gate failures are in the report above.
exit 0
