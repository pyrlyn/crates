# Benchmarks

Measured on Ivan's Mac (Apple Silicon) against rtok's own index (rtok + cox roots, 317k symbols,
365,733 graph edges). "old" is rtok's current schema and queries (one denormalized `symbols` table,
SQL self-joins, impact as a per-path BFS in the host); "new" is this crate through the loadable
extension. Produced by `scripts/bench.py` and `examples/qps.rs` (feature `rtok-import`).

## Latency and size

```text
cold graph load (first TVF call): 8.6 ms; impact result mismatches: 0
query                          old p50       p95       max |   new p50       p95       max  (ms)
defs                             0.007     0.053     0.808 |     0.004     0.006     0.084
refs                             0.007     0.069     0.915 |     0.004     0.049     0.497
callers                          0.008     0.088     1.116 |     0.005     0.041     0.564
callees                          0.223     0.793     9.309 |     0.013     0.081     0.314
outline                          0.026     0.251     1.072 |     0.008     0.045     0.553
callers_native                                             |     0.003     0.016     0.223
callees_native                                             |     0.008     0.021     0.463
impact d1 random                 0.028     1.797    31.229 |     0.005     0.080     1.296
impact d1 top                   49.725   208.004   261.635 |     1.493     3.411     4.464
impact d2 random                 0.063    64.868   537.884 |     0.013     2.233     9.468
impact d2 top                  503.573  1942.038  2216.370 |     8.696    12.384    13.206
impact d3 random                 0.132  1387.888 12619.112 |     0.027    10.444    13.916
impact d3 top                11255.084 23814.936 27665.043 n=6 |    13.619    14.791    14.821
fts 'graph'                      0.199     0.395     3.544 |     0.224     0.294     6.010
fts 'symbol OR impact'           0.258     0.338     1.194 |     0.295     0.364     1.162
fts 'cache'                      0.235     0.308     2.860 |     0.264     0.364     2.810

old.db 132.4 MB = 417 B/symbol; new.db 38.7 MB = 122 B/symbol
graph: 365733 edges in 1384619 B of CSR segments = 3.79 B/edge
```

## Read throughput (queries/s)

```text
# full mix (defs, refs, callers, callees, impact depth 1), SQLite default MEMSTATUS=1
cold_open_plus_graph_load_ms=11.70
threads,old_mutex_qps,old_wal_multiconn_qps,new_qps
1,9635,9958,31165
2,9107,16067,25513
4,7587,21732,17587
8,7888,11750,11368
16,6528,16390,19183
# same, MEMSTATUS=0 for both
cold_open_plus_graph_load_ms=29.43
threads,old_mutex_qps,old_wal_multiconn_qps,new_qps
1,7565,9928,38442
2,5943,11157,63893
4,8348,15955,69548
8,8920,20515,197677
16,9843,23903,275802
```

With SQLite's global memory statistics on (the default), every allocation takes one mutex and
throughput stops scaling past one thread; `disable_sqlite_memstatus()` removes that cap.
