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
| delete via tombstones + compaction | **chosen**, `delete(&ids)`, `compact()` | The crate had no delete at all. A tombstone is one flag per node checked in the same accept hook as the filter, plus a new `vdb_tomb` table, so pages and old files stay as they are. Compaction rebuilds from the live vectors: simpler and safer than repairing links in place, at the cost of a full build. |
| persisted entry point / levels | **already present**, now tested | The format stored them from the start: `vdb_meta` key 4 holds the entry node and its level packed into one integer, and every node record in a page starts with its level count followed by one link list per level. `open` restores both, so there is no rebuild or re-descent on load. Only a test that pins this down was missing. |
| ef/M presets for recall 0.90 / 0.95 | **chosen**, `Params::for_recall(Recall, Quant)` | The recorded benchmark already maps `ef_search` to recall per mode; a constructor turns that table into code. It is a plain function that changes nothing unless called, and `Params::default()` keeps its values. |

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
rewrites (see [how-it-works.md](how-it-works.md)). Until the graph holds one
page (1024 nodes), rows are inserted in order first. Many threads starting on
a near-empty graph miss each other's nodes and link poorly: self-recall of a
3000-vector batch varied from 180 to 200/200 without this step, and was
200/200 in 25 of 25 runs with it. Without the feature the rows
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

## Delete: tombstones and compaction (`delete`, `compact`)

**How it works.** `delete(&ids)` scans the arena once and sets a tombstone
flag on every node whose id is in `ids` (all copies of a duplicate id). It
returns how many nodes were newly tombstoned. Searches check the flag in the
same accept hook as `search_filtered`: a tombstoned node is still walked
through, so the graph stays connected, but it is never returned. When there
are no tombstones the check is skipped. `save` writes the new tombstones as
rows of a new table `vdb_tomb (node INTEGER PRIMARY KEY)` in the same
transaction as the pages. Pages are not rewritten for a delete. `open` reads
the table back.

`compact(&mut self)` takes the arena, keeps the live nodes, and re-inserts them
into a fresh graph. Each vector keeps its stored encoding (`f32`, `i8`, or
`i8` + `f32`), so there is no requantization. With `parallel` this uses the
same warm-up-then-rayon path as `insert_batch`. Then one transaction runs
`DELETE FROM vdb_page; DELETE FROM vdb_tomb;`, writes every page and the meta
row, and commits. It returns the number of vectors removed. With no
tombstones it does nothing and returns `0`.

`save` now rolls back on any error and keeps the pending pages, tombstones
and (after `compact`) the full rewrite pending, so a later `save` can retry.
Before, a failed save lost the dirty-page list and left the transaction open.

**Enable.** Call `db.delete(&[id, ...])`, then `db.save()` to persist, and
`db.compact()` when `db.tombstones()` gets large, for example above 10-20% of
`db.len()`. An update is `delete` followed by `insert`.

**Disable.** Do not call them. Files without deletes get only an empty
`vdb_tomb` table, created on `open`.

**Test.** `cargo test -p sqlite-hnsw --test delete`: deleted ids vanish from
search at once, and repeated or unknown ids count 0; every copy of a
duplicate id is deleted; tombstones persist only after `save` and need no
page rewrite; `compact` drops 2000 of 2500 vectors, leaves nothing dirty,
keeps self-recall >= 95% and survives a reopen; compacting everything leaves
a usable empty index; `compact` keeps `I8Rerank` vectors exact; a filter and
tombstones combine.

**Limitations.**

- `delete` scans every node (no id index): O(n) per call, so batch the ids.
- Tombstoned nodes still take memory, disk, and slots in neighbour lists
  until `compact`. Many tombstones lower recall at a given `ef` and make
  searches walk more nodes.
- `compact` costs a full rebuild, needs `&mut self` (no concurrent searches
  meanwhile), changes the internal node order, and saves.
- An unsaved `delete` is lost on crash. An insert running at the same time as
  a `delete` of the same id may survive it.
- Older builds ignore `vdb_tomb`: they show deleted vectors again (until
  `compact`, which removes them from the pages for good).

## Persisted entry point and levels

**How it works.** Nothing new: `save` writes the entry point (node index and
top level, packed as `node << 8 | level`) to `vdb_meta` key 4. Each node
record in a `vdb_page` BLOB stores its level count and the link list of every
level. `open` parses the pages straight into the arena and sets the entry
point from the meta row, so a reopened index walks exactly the graph that
was saved. Inserts after a reopen continue from that graph.

**Enable.** Always on; it is part of the file format.

**Disable.** Not applicable.

**Test.** `cargo test -p sqlite-hnsw --test reopen`: for `F32`, `I8` and
`I8Rerank` (2500 nodes, three pages, `m = 8`) the hits after a reopen equal
the hits before it, ids and scores bit for bit, and a `save` right after
`open` writes 0 pages. An index grown over two sessions keeps self-recall
>= 98%.

**Limitations.**

- The level generator's state is not stored: every `open` restarts it from
  the same fixed seed, so each session repeats the same level sequence. The
  levels still follow the HNSW distribution, but two sessions do not
  reproduce one uninterrupted run.

## Parameter presets (`Params::for_recall`)

**How it works.** `Params::for_recall(target, quant)` returns `m = 32`,
`ef_construction = 400`, the given `quant`, `rerank` 4 and an `ef_search`
taken from the recorded benchmark ([benchmark.md](benchmark.md): 100k x 384,
half uniform, half clustered, recall@10):

| target | `F32` | `I8` | `I8Rerank` |
|---|---|---|---|
| `Recall::P90` | 768 | 768 | 768 |
| `Recall::P95` | 1024 | 1280 | 1024 |

Measured at `m = 32`: `ef_search` 512 gave 0.892 (`F32`) / 0.884 (`I8`),
1024 gave 0.963 / 0.951, 2048 gave 0.992 / 0.980. 768 lies between the 512
and 1024 points; 1280 gives `I8` a margin over its 0.951 at 1024. `m = 16`
needed `ef_search` 2048 for 0.94, so the presets use `m = 32`.
`I8Rerank` uses the `F32` values: its walk is the `I8` walk, and the rescoring
can only move recall towards `F32`.

```rust,ignore
let db = VecDb::open("v.db", 384, Params::for_recall(Recall::P95, Quant::I8))?;
```

**Enable.** Pass `Params::for_recall(...)` to `VecDb::open` when creating the
file. `m` and `ef_construction` are stored then; on reopen only `ef_search`
(and `rerank`) from the preset apply.

**Disable.** Build `Params` by hand or use `Params::default()` (`m = 16`,
`ef_search = 64`), which did not change.

**Test.** `cargo test -p sqlite-hnsw --test presets`: the values match the
table above for every mode, a higher target never searches less, and an
index built with the presets meets both targets on small data.

**Limitations.**

- 768 and 1280 are interpolated, not measured; nothing here re-ran the
  benchmark.
- The numbers come from uniform synthetic vectors, the hard case. Clustered,
  embedding-like data reached 0.986 already at `ef_search` 64, so real
  embeddings usually need far less. Measure your own recall and lower it with
  `VecDb::set_ef_search`.
- Recall also depends on the dimension and the index size; the presets are
  sized for about 100k vectors of dimension 384.
