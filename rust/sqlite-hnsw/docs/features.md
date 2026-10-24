# Performance features

## Decision

Each candidate was weighed by expected gain, risk to existing files and
defaults, and code it adds. Defaults stay as before: every new behaviour is
behind a cargo feature, a new `Quant` variant, an explicit method or a
parameter that changes nothing unless set.

| candidate | decision | why |
|---|---|---|
| SIMD distance kernels | **chosen**, feature `simd` (off by default) | The dot product is the inner loop of insert and search. The scalar code already auto-vectorizes to 128-bit lanes without FMA. NEON with FMA and 4 accumulators, and AVX2 + FMA on x86_64 (detected at runtime; the default x86_64 target only has SSE2), are the cheapest win. It needs `unsafe` (raw-pointer loads, calling a `#[target_feature]` function), so it is opt-in and kept in one module. |
| i8 search with f32 rerank | **chosen**, `Quant::I8Rerank` + `Params::rerank` | The benchmark shows i8 walks faster than f32 (p50 10 vs 15 ms at ef 1024) but loses recall (0.951 vs 0.963). Rescoring a few candidates with exact `f32` fixes the order where quantization swapped neighbours. A new mode, so `F32` and `I8` files and behaviour stay unchanged. |
| parallel build + batch insert | **chosen**, `insert_batch` + feature `parallel` (off by default) | `insert` is already thread-safe, so the rayon version is a small loop, and a bulk load no longer needs a hand-written thread pool (as `vecbench` has). The batch saves once, in one transaction. rayon is already in the workspace lock; the feature keeps it out of default builds. |
| filtered search | **chosen**, `search_filtered(q, k, predicate)` | Post-filtering the top `k` returns too few hits. Filtering inside the beam (the hnswlib approach) is a few lines in `search_layer` and costs nothing when unused. An allow-list is the predicate `|id| set.contains(&id)`, so one API covers both. The same hook later serves tombstones. |

Skipped:

| candidate | why skipped |
|---|---|
| Prefetch | Needs more `unsafe` intrinsics in the hot loop. Each node's vector is its own heap allocation, so a prefetch of the next neighbour only helps with a known layout. Without a benchmark run (not allowed here) a gain cannot be shown, and a wrong prefetch distance costs speed. |

## SIMD distance kernels (`simd`)

**How it works.** `sqlite_hnsw::distance` holds the two kernels every distance
goes through: `dot` (`f32` x `f32`) and `dot_i8` (`f32` query x `i8` vector,
widened to `f32`). Without the feature they are the scalar loops (eight partial
sums). With it:

- aarch64 (NEON is part of the target baseline): 16 components per step in
  four `float32x4` accumulators with fused multiply-add; `i8` is widened
  `i8 -> i16 -> i32 -> f32` in registers.
- x86_64: `is_x86_feature_detected!("avx2")` and `("fma")` pick a 2 x 256-bit
  FMA loop; `i8` goes through `_mm256_cvtepi8_epi32`. Without AVX2 + FMA the
  scalar loop runs. std caches the CPUID result, so a call costs one atomic load.
- Other targets: scalar.

Tails (length not a multiple of 16) run through the scalar kernel.
`distance::backend()` reports `Scalar`, `Neon` or `Avx2`.

**Enable.** `sqlite-hnsw = { version = "...", features = ["simd"] }`.

**Disable.** Leave the feature off (the default). Nothing else changes; only
the summation order differs, so scores can differ in the last bits.

**Test.** `cargo test -p sqlite-hnsw --features simd --test distance` checks the
dispatched kernels against the scalar ones for every length 0..=133 (within
`1e-5` of the sum of `|a_i * b_i|`), bit equality where the sum is exact
(small integers, `i8::MIN`/`i8::MAX` extremes), mismatched lengths, and that
`backend()` is `Neon` on aarch64 with the feature and `Scalar` without it.
The index tests run on top of the same kernels.

**Limitations.**

- `unsafe` code: the module `src/simd.rs` (loads through raw pointers inside
  bounds-checked 16-element chunks, and the AVX2 call after the runtime check).
- The AVX2 path is compile- and clippy-checked for `x86_64-apple-darwin`; on
  the Apple Silicon machine used here Rosetta reports no AVX2 + FMA, so its
  test fell back to scalar. The AVX2 kernel runs in tests only on a real
  x86_64 CPU with AVX2.
- No AVX-512 and no `i8 x i8` integer dot product: the query stays `f32` so
  quantization error stays on the stored side only.
- No speedup figure: benchmarks were not re-run for this change.

## i8 search with f32 rerank (`Quant::I8Rerank`)

**How it works.** Each node keeps the `i8` codes and scale (as `Quant::I8`)
plus its normalized `f32` vector. Insert and the graph walk use only the `i8`
distance, so the graph is built exactly like an `I8` index. `search(q, k)`
widens the layer-0 beam to at least `k * rerank` (still at least `ef_search`),
keeps the best `k * rerank` candidates by `i8` distance, recomputes their
distance with the `f32` copy, sorts again and returns the top `k`. The scores
returned are then the exact `f32` cosine similarity. Pages store
`scale, i8 x dim, f32 x dim` per node; `vdb_meta` quantization code `2`.

**Enable.** Create the file with
`Params { quant: Quant::I8Rerank, rerank: 4, ..Params::default() }`.
`rerank` (default 4) is a search-time knob, not stored: pass it on every open
or change it with `VecDb::set_rerank`. `0` counts as `1` (rescore only `k`).

**Disable.** Use `Quant::I8` (no `f32` copy) or `Quant::F32`. The mode is
fixed when the file is created, like `m`; switching means rebuilding the
index. `set_rerank(1)` keeps the mode but stops widening the candidate list.

**Test.** `cargo test -p sqlite-hnsw --test rerank`: returned scores equal the
exact `f32` cosine (within `1e-5`), recall@10 is at least that of plain `I8`
on the same data and graph, the factor can change at runtime (including `0`),
and the mode and vectors survive a reopen.

**Limitations.**

- The `f32` copy stays in memory: about 1.25x the RAM and disk of `F32`. The
  gain over `F32` is the faster `i8` walk, not memory. Keeping `f32` on disk
  only would need SQLite reads during every search, through the connection
  mutex, and is not implemented.
- A file in this mode needs this version or newer. Older builds read code `2`
  as `F32` and misparse the pages.
- With a large `k * rerank` the beam grows past `ef_search`, which costs
  latency like a larger `ef_search`.
- No measured recall/latency numbers yet for this mode (`vecbench` accepts
  quant `i8r`; the benchmark was not run here).

## Batch insert and parallel build (`insert_batch`, `parallel`)

**How it works.** `VecDb::insert_batch(ids, vectors)` takes `n` ids and `n * dim`
values in row-major order. It checks the lengths first (on a mismatch it
returns an error and inserts nothing), inserts every row, then calls `save`,
which writes all dirty pages and the meta row in one `BEGIN IMMEDIATE ...
COMMIT` transaction. It returns the number of pages written. With the
`parallel` feature the rows go through `rayon`'s `into_par_iter` on the
global pool. That works because `insert` only locks the adjacency lists it
rewrites (see [how-it-works.md](how-it-works.md)). Without the feature the rows
are inserted in order on the calling thread.

**Enable.** Call `insert_batch` (always available). For multi-threaded
inserts add `features = ["parallel"]`. Size the pool with
`RAYON_NUM_THREADS` or `rayon::ThreadPoolBuilder` as usual.

**Disable.** Keep using `insert` + `save`, or leave `parallel` off to insert
the batch on the calling thread.

**Test.** `cargo test -p sqlite-hnsw --features parallel --test batch` (and
without the feature): a 3000-vector batch writes three pages, leaves nothing
dirty, reopens with self-recall >= 95%; a batch appended to an existing index
is searchable; a length mismatch inserts nothing; an empty batch is a plain
`save`.

**Limitations.**

- With `parallel` the insert order, and so the graph, differs from run to run
  (as with `insert` from several threads); results are equivalent, not identical.
- The batch is one transaction on disk, not in memory: if `save` fails, the
  vectors stay inserted in memory and are written by the next `save`.
- Ids are not checked for duplicates, as with `insert`.
- No measured speedup here. The old `vecbench` build with 16 threads already
  used the same concurrent `insert`.

## Filtered search (`search_filtered`)

**How it works.** `search_filtered(q, k, |id| ...)` runs the normal descent
through the upper layers, then the layer-0 beam with an accept test. Every
visited node still enters the candidate queue, so the walk can pass through
rejected regions. Only accepted nodes enter the result list. The beam stops
when it holds `ef` accepted results and the next candidate is farther than
the worst of them, so it cannot stop early because of rejected nodes. With
`Quant::I8Rerank` the rerank runs on the accepted candidates.

```rust,ignore
let allowed: HashSet<u64> = tenant_ids();
let hits = db.search_filtered(&q, 10, |id| allowed.contains(&id));
let even = db.search_filtered(&q, 10, |id| id % 2 == 0);
```

**Enable.** Call `search_filtered` instead of `search`. The predicate gets
the id given to `insert` and must be cheap: it runs once per visited node.

**Disable.** Call `search`. Without a filter the path is the old one plus one
`Option` check per neighbour.

**Test.** `cargo test -p sqlite-hnsw --test filter`: a 1-in-3 predicate returns
only matching ids, sorted, with recall@10 >= 0.9 against exact filtered
search; a 21-id allow-list out of 2000 is found (recall >= 0.9); fewer
matches than `k` returns exactly the matches, and a reject-all filter returns
nothing; an accept-all filter returns exactly what `search` returns.

**Limitations.**

- Latency grows as the filter gets more selective. With fewer than `ef`
  matches reachable, the walk visits every reachable node: correct, but up to
  brute-force cost. For a small allow-list it can be cheaper to score those
  vectors directly.
- No payload index: the predicate sees only the id. Map ids to attributes
  yourself.
