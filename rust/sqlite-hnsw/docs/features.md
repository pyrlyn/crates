# Performance features

## Decision

Each candidate was weighed by expected gain, risk to existing files and
defaults, and code it adds. Defaults stay as before: every new behaviour is
behind a cargo feature, a new `Quant` variant, an explicit method or a
parameter that changes nothing unless set.

| candidate | decision | why |
|---|---|---|
| SIMD distance kernels | **chosen**, feature `simd` (off by default) | The dot product is the inner loop of insert and search. The scalar code already auto-vectorizes to 128-bit lanes without FMA. NEON with FMA and 4 accumulators, and AVX2 + FMA on x86_64 (detected at runtime; the default x86_64 target only has SSE2), are the cheapest win. It needs `unsafe` (raw-pointer loads, calling a `#[target_feature]` function), so it is opt-in and kept in one module. |

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
