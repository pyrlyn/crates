#!/usr/bin/env python3
"""Prints the last criterion run as a markdown table: benchmark id, median
time and, for throughput groups, elements per second. Run from bench/ after
`cargo bench -p criterion-bench`."""
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "target/criterion")


def fmt_time(ns):
    for unit, scale in (("s", 1e9), ("ms", 1e6), ("µs", 1e3)):
        if ns >= scale:
            return f"{ns / scale:.3g} {unit}"
    return f"{ns:.3g} ns"


print("| benchmark | median | throughput |")
print("| --- | ---: | ---: |")
for est in sorted(root.glob("**/new/estimates.json")):
    bench_dir = est.parent
    meta = json.loads((bench_dir / "benchmark.json").read_text())
    median = json.loads(est.read_text())["median"]["point_estimate"]
    thrpt = ""
    elements = (meta.get("throughput") or {}).get("Elements")
    if elements:
        per_sec = elements / (median / 1e9)
        thrpt = f"{per_sec / 1e6:,.1f} M/s"
    print(f"| {meta['full_id']} | {fmt_time(median)} | {thrpt} |")
