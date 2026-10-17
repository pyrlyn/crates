#!/bin/sh
# Runs the comparison: resolution timings (di-bench) and, per library, the stripped release size
# and clean release build time of an executable wiring the same graph. Writes results/<date>.md.
# Rerunning on the same day overwrites that file.
# Timings depend on machine load; the load average is recorded with them.
set -eu
cd "$(dirname "$0")"
mkdir -p results
out="results/$(date +%Y-%m-%d).md"
{
  echo "# Benchmark run $(date '+%Y-%m-%d %H:%M %Z')"
  echo
  echo "- Machine: $(sysctl -n machdep.cpu.brand_string), $(sysctl -n hw.ncpu) cores"
  echo "- Toolchain: $(swift --version 2>&1 | head -1)"
  echo "- Load average before: $(sysctl -n vm.loadavg)"
  echo
  echo "## Resolution (ns per operation, median of 15 runs, release)"
  echo
  swift run -c release di-bench
  echo
  echo "## Binary size and clean build time (release, stripped)"
  echo
  echo "| library | stripped size (bytes) | delta vs manual | clean build (s) |"
  echo "| --- | ---: | ---: | ---: |"
  base=""
  for lib in manual injecta factory swinject resolver dependencies; do
    scratch=".build-size-$lib"
    rm -rf "$scratch"
    # Dependencies are fetched first so the timing is compilation, not the network.
    swift package --scratch-path "$scratch" resolve >/dev/null 2>&1
    start=$(date +%s)
    swift build -c release --product "size-$lib" --scratch-path "$scratch" >/dev/null 2>&1
    end=$(date +%s)
    bin="$scratch/release/size-$lib"
    cp "$bin" "/tmp/size-$lib" && strip "/tmp/size-$lib"
    size=$(stat -f %z "/tmp/size-$lib")
    [ -z "$base" ] && base=$size
    echo "| $lib | $size | $((size - base)) | $((end - start)) |"
    rm -f "/tmp/size-$lib"
    if [ "$lib" = injecta ]; then
      # How much of that is Injecta itself: touch only its sources and rebuild. The rest is
      # swift-syntax, which the macro plugin needs.
      find ../Sources/InjectaGraph ../Sources/InjectaSyntax ../Sources/InjectaMacros ../Sources/Injecta \
        -name '*.swift' -exec touch {} +
      start=$(date +%s)
      swift build -c release --product "size-$lib" --scratch-path "$scratch" >/dev/null 2>&1
      end=$(date +%s)
      syntax=$(find "$scratch" -path '*swift-syntax.build*' -name '*.o' | wc -l | tr -d ' ')
      echo "| injecta, rebuild of Injecta's own modules only | | | $((end - start)) |"
      echo "| (swift-syntax objects compiled from source in the clean build: $syntax) | | | |"
    fi
  done
  echo
  echo "- Load average after: $(sysctl -n vm.loadavg)"
} | tee "$out"
