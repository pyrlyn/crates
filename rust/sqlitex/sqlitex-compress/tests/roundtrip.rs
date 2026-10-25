#![allow(unused_imports, dead_code)]

use sqlitex_compress::{decompress, split_header, Algorithm, Compressor, Error};

fn pseudo_random(n: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn inputs() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),
        b"hello sqlite".to_vec(),
        b"abcabcabc".repeat(100_000),
        pseudo_random(64 * 1024),
    ]
}

fn check(c: &dyn Compressor) {
    for input in inputs() {
        let packed = c.compress(&input).unwrap();
        assert_eq!(packed[0], c.algorithm().tag());
        assert_eq!(c.decompress(&packed).unwrap(), input);
        assert_eq!(decompress(&packed).unwrap(), input);
    }
    let big = b"abcabcabc".repeat(100_000);
    assert!(c.compress(&big).unwrap().len() < big.len() / 10);

    let mut corrupt = c.compress(&big).unwrap();
    corrupt.truncate(corrupt.len() / 2);
    for b in corrupt.iter_mut().skip(1).take(8) {
        *b ^= 0xFF;
    }
    assert!(matches!(c.decompress(&corrupt), Err(Error::Codec { .. })));
    assert!(matches!(c.decompress(&[]), Err(Error::MissingHeader)));
    assert!(matches!(
        c.decompress(&[0xEE, 1, 2]),
        Err(Error::UnknownTag(0xEE))
    ));
}

#[test]
fn header_errors() {
    assert!(matches!(split_header(&[]), Err(Error::MissingHeader)));
    assert!(matches!(decompress(&[0x7F]), Err(Error::UnknownTag(0x7F))));
}

#[cfg(feature = "zstd")]
#[test]
fn zstd_roundtrip() {
    use sqlitex_compress::{zstd::Zstd, Level};
    check(&Zstd::default());
    check(&Zstd { level: Level::Best });
}

#[cfg(feature = "brotli")]
#[test]
fn brotli_roundtrip() {
    use sqlitex_compress::{brotli::Brotli, Level};
    check(&Brotli::default());
    check(&Brotli {
        level: Level::Fastest,
    });
}

#[cfg(feature = "lz4")]
#[test]
fn lz4_roundtrip() {
    check(&sqlitex_compress::lz4::Lz4);
}

#[cfg(all(feature = "zstd", feature = "lz4"))]
#[test]
fn tag_mismatch() {
    let packed = sqlitex_compress::zstd::Zstd::default()
        .compress(b"x")
        .unwrap();
    assert!(matches!(
        sqlitex_compress::lz4::Lz4.decompress(&packed),
        Err(Error::TagMismatch {
            expected: Algorithm::Lz4,
            found: Algorithm::Zstd
        })
    ));
}

#[cfg(not(feature = "zstd"))]
#[test]
fn disabled_algorithm() {
    assert!(matches!(
        decompress(&[0x02, 0]),
        Err(Error::AlgorithmDisabled(Algorithm::Zstd))
    ));
}
