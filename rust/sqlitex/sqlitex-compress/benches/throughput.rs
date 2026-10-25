//! Compress/decompress throughput per algorithm. Run with
//! `cargo bench -p sqlitex-compress --all-features --bench throughput`.
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use sqlitex_compress::Compressor;
use std::hint::black_box;

const SIZES: &[(usize, &str)] = &[
    (1 << 10, "1KB"),
    (64 << 10, "64KB"),
    (1 << 20, "1MB"),
    (16 << 20, "16MB"),
];

fn compressible(n: usize) -> Vec<u8> {
    let line =
        b"{\"id\":12345,\"user\":\"alice\",\"event\":\"page_view\",\"path\":\"/docs/intro\"}\n";
    line.iter().copied().cycle().take(n).collect()
}

fn random(n: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

#[allow(unused_mut, clippy::vec_init_then_push)]
fn algorithms() -> Vec<(&'static str, Box<dyn Compressor>)> {
    let mut v: Vec<(&'static str, Box<dyn Compressor>)> = Vec::new();
    #[cfg(feature = "brotli")]
    v.push((
        "brotli",
        Box::new(sqlitex_compress::brotli::Brotli::default()),
    ));
    #[cfg(feature = "zstd")]
    v.push(("zstd", Box::new(sqlitex_compress::zstd::Zstd::default())));
    #[cfg(feature = "lz4")]
    v.push(("lz4", Box::new(sqlitex_compress::lz4::Lz4)));
    v
}

fn bench(c: &mut Criterion) {
    let big = std::env::var_os("SQLITEX_BENCH_16MB").is_some();
    for (kind, make) in [
        ("compressible", compressible as fn(usize) -> Vec<u8>),
        ("random", random),
    ] {
        for (name, alg) in algorithms() {
            let mut g = c.benchmark_group(format!("{name}/{kind}"));
            for &(size, label) in SIZES {
                if size > (1 << 20) && !big {
                    continue;
                }
                let input = make(size);
                let packed = alg.compress(&input).unwrap();
                g.throughput(Throughput::Bytes(size as u64));
                g.bench_with_input(BenchmarkId::new("compress", label), &input, |b, i| {
                    b.iter(|| alg.compress(black_box(i)).unwrap())
                });
                g.bench_with_input(BenchmarkId::new("decompress", label), &packed, |b, p| {
                    b.iter(|| alg.decompress(black_box(p)).unwrap())
                });
            }
            g.finish();
        }
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
