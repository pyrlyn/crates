#!/bin/bash
# usage: bench.sh A|B [runs] [tag]   — only inside the scratch worktree rtok-prune-bench2
set -u
S=$1; RUNS=${2:-10}; TAG=${3:-$S}
W=$HOME/Grok/worktrees/rtok-prune-bench2
TP=$HOME/Grok/target-prune-rs/target/release/target-prune
OUT=$HOME/Grok/target-prune-rs/bench/out-$TAG; mkdir -p "$OUT"
cd "$W" || exit 1
[ "$(basename "$W")" = rtok-prune-bench2 ] || exit 1
git checkout -q -- . && rm -rf target
export CARGO_TARGET_DIR=$W/target
now() { python3 -c 'import time;print(f"{time.time():.2f}")'; }
printf "run\tvariant\tbuild_s\tsize_mb\tfreed_mb\tprune_s\n" > "$OUT/table.tsv"
for i in $(seq 1 "$RUNS"); do
  # code changes in 4 crates, every run
  for f in crates/rtok-log/src/lib.rs crates/rtok-config/src/lib.rs crates/rtok-store/src/lib.rs src/lib.rs; do
    sed -i '' '/PRUNE_BENCH/d' "$f"; printf '#[doc = "bench"] pub const PRUNE_BENCH: u32 = %d; // PRUNE_BENCH\n' "$i" >> "$f"
  done
  FEAT=""; RF=""; DBG=""; V="code"
  case $i in 3|6|9) FEAT='--no-default-features --features measure,cmd,read,json_tree,archive,proxy,inject,guard,memory,docs,toon,compress'; V="$V+nofeat-graph";; esac
  case $i in 4|8) RF="--cfg prune_bench"; V="$V+rustflags";; esac
  case $i in 2|5|7|10) DBG=false; V="$V+debug=false";; esac
  case $i in 6) sed -i '' 's|^workspace-hack = { version = "0.1", path = "../../workspace-hack" }$|&\nitoa = "1"|' crates/rtok-log/Cargo.toml; V="$V+add-dep";; esac
  case $i in 9) sed -i '' 's|^itoa = "1"$|itoa = "=1.0.17"|' crates/rtok-log/Cargo.toml; V="$V+bump-dep";; esac
  CMD="cargo build $FEAT && cargo test --no-run --workspace"
  fr_gb=$(df -g "$W" | awk 'NR==2{print $4}'); if [ "$fr_gb" -lt 30 ]; then echo "STOP: free ${fr_gb}G < 30G" >> "$OUT/table.tsv"; break; fi
  t0=$(now)
  if [ "$S" = B ]; then
    RUSTFLAGS="$RF" CARGO_PROFILE_DEV_DEBUG=${DBG:-line-tables-only} "$TP" run --keep 2 --apply target -- sh -c "$CMD" > "$OUT/prune-$i.log" 2> "$OUT/err-$i.log"
  else
    RUSTFLAGS="$RF" CARGO_PROFILE_DEV_DEBUG=${DBG:-line-tables-only} sh -c "$CMD" > /dev/null 2> "$OUT/err-$i.log"
  fi
  rc=$?; t1=$(now)
  ps=$(grep -o 'prune took [0-9.]*' "$OUT/err-$i.log" | awk '{print $3}'); ps=${ps:-0}
  fr=$(grep -o 'freed [0-9.]* [KMGT]*B' "$OUT/prune-$i.log" 2>/dev/null | tail -1)
  sz=$(du -sk target | awk '{printf "%d", $1/1024}')
  "$TP" snapshot target > "$OUT/snap-$i.json"
  du -sk target/debug/* target/debug/.fingerprint 2>/dev/null | sort -rn | head -12 > "$OUT/du-$i.txt"
  printf "%d\t%s\t%.1f\t%s\t%s\t%s\trc=%d\n" "$i" "$V" "$(echo "$t1 - $t0 - $ps" | bc)" "$sz" "${fr:-}" "$ps" "$rc" >> "$OUT/table.tsv"
done
git checkout -q -- .
