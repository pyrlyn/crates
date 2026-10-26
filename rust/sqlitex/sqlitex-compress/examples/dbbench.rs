//! Driver for `bench/run.py`: compresses rows exported from a SQLite DB.
//!
//! Env: `SQLITEX_BENCH_INPUT` (rows file: u32 LE length + bytes, repeated),
//! `SQLITEX_BENCH_ALG` (`none`, `brotli`, `zstd`, `lz4`),
//! `SQLITEX_BENCH_OUTPUT` (same format, compressed rows).
//! Prints one JSON line with sizes and throughput.
use sqlitex_compress::Compressor;
use std::time::Instant;

fn read_rows(buf: &[u8]) -> Vec<&[u8]> {
    let mut rows = Vec::new();
    let mut i = 0;
    while i + 4 <= buf.len() {
        let n = u32::from_le_bytes(buf[i..i + 4].try_into().unwrap()) as usize;
        rows.push(&buf[i + 4..i + 4 + n]);
        i += 4 + n;
    }
    rows
}

fn pick(name: &str) -> Option<Box<dyn Compressor>> {
    match name {
        #[cfg(feature = "brotli")]
        "brotli" => Some(Box::new(sqlitex_compress::brotli::Brotli::default())),
        #[cfg(feature = "zstd")]
        "zstd" => Some(Box::new(sqlitex_compress::zstd::Zstd::default())),
        #[cfg(feature = "lz4")]
        "lz4" => Some(Box::new(sqlitex_compress::lz4::Lz4)),
        "none" => None,
        other => panic!("algorithm {other} not enabled in this build"),
    }
}

fn main() {
    let input = std::env::var("SQLITEX_BENCH_INPUT").expect("SQLITEX_BENCH_INPUT");
    let output = std::env::var("SQLITEX_BENCH_OUTPUT").expect("SQLITEX_BENCH_OUTPUT");
    let alg_name = std::env::var("SQLITEX_BENCH_ALG").unwrap_or_else(|_| "none".into());
    let alg = pick(&alg_name);
    let buf = std::fs::read(&input).unwrap();
    let rows = read_rows(&buf);
    let raw: usize = rows.iter().map(|r| r.len()).sum();

    let t = Instant::now();
    let packed: Vec<Vec<u8>> = rows
        .iter()
        .map(|r| match &alg {
            Some(c) => c.compress(r).unwrap(),
            None => r.to_vec(),
        })
        .collect();
    let ct = t.elapsed().as_secs_f64();

    let t = Instant::now();
    for (p, r) in packed.iter().zip(&rows) {
        let back = match &alg {
            Some(c) => c.decompress(p).unwrap(),
            None => p.clone(),
        };
        assert_eq!(back.len(), r.len());
    }
    let dt = t.elapsed().as_secs_f64();

    let mut out = Vec::new();
    for p in &packed {
        out.extend_from_slice(&(p.len() as u32).to_le_bytes());
        out.extend_from_slice(p);
    }
    std::fs::write(&output, &out).unwrap();
    let comp: usize = packed.iter().map(|p| p.len()).sum();
    let mb = raw as f64 / 1e6;
    println!(
        "{{\"alg\":\"{alg_name}\",\"rows\":{},\"raw_bytes\":{raw},\"compressed_bytes\":{comp},\"compress_mb_s\":{:.1},\"decompress_mb_s\":{:.1}}}",
        rows.len(),
        mb / ct.max(1e-9),
        mb / dt.max(1e-9)
    );
}
