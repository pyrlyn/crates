//! Feature-matrix tests: every case compiles and runs under all 8 combinations
//! of `brotli`, `zstd` and `lz4` (see plan.md, "Feature matrix").
#![allow(unused_imports, dead_code)]

use sqlitex_compress::{decompress, split_header, Algorithm, Compressor, Error};

fn random(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// Every compressor enabled in this build.
#[allow(unused_mut, clippy::vec_init_then_push)]
fn enabled() -> Vec<Box<dyn Compressor>> {
    let mut v: Vec<Box<dyn Compressor>> = Vec::new();
    #[cfg(feature = "brotli")]
    v.push(Box::new(sqlitex_compress::brotli::Brotli::default()));
    #[cfg(feature = "zstd")]
    v.push(Box::new(sqlitex_compress::zstd::Zstd::default()));
    #[cfg(feature = "lz4")]
    v.push(Box::new(sqlitex_compress::lz4::Lz4));
    v
}

fn feature_count() -> usize {
    usize::from(cfg!(feature = "brotli"))
        + usize::from(cfg!(feature = "zstd"))
        + usize::from(cfg!(feature = "lz4"))
}

#[test]
fn enabled_set_matches_features() {
    assert_eq!(enabled().len(), feature_count());
}

#[test]
fn roundtrip_small() {
    for c in enabled() {
        let input = b"the quick brown fox jumps over the lazy dog".to_vec();
        let packed = c.compress(&input).unwrap();
        assert_eq!(c.decompress(&packed).unwrap(), input);
        assert_eq!(decompress(&packed).unwrap(), input);
    }
}

#[test]
fn roundtrip_empty() {
    for c in enabled() {
        let packed = c.compress(&[]).unwrap();
        assert_eq!(packed[0], c.algorithm().tag());
        assert!(c.decompress(&packed).unwrap().is_empty());
        assert!(decompress(&packed).unwrap().is_empty());
    }
}

#[test]
fn roundtrip_incompressible() {
    let input = random(256 * 1024, 42);
    for c in enabled() {
        let packed = c.compress(&input).unwrap();
        assert_eq!(decompress(&packed).unwrap(), input);
        // Random data must not blow up: tag + codec framing only.
        assert!(packed.len() < input.len() + input.len() / 100 + 1024);
    }
}

#[test]
fn roundtrip_large() {
    let mut input = Vec::with_capacity(8 << 20);
    while input.len() < 8 << 20 {
        input.extend_from_slice(b"{\"user\":1234,\"event\":\"click\",\"ok\":true}\n");
    }
    input.extend_from_slice(&random(4096, 7));
    for c in enabled() {
        let packed = c.compress(&input).unwrap();
        assert!(packed.len() < input.len() / 5, "{:?}", c.algorithm());
        assert_eq!(decompress(&packed).unwrap(), input);
    }
}

#[test]
fn first_byte_tag() {
    for c in enabled() {
        let packed = c.compress(b"tag").unwrap();
        let expected = match c.algorithm() {
            Algorithm::Brotli => 0x01,
            Algorithm::Zstd => 0x02,
            Algorithm::Lz4 => 0x03,
        };
        assert_eq!(packed[0], expected);
        assert_eq!(split_header(&packed).unwrap().0, c.algorithm());
    }
}

#[test]
fn tags_are_stable() {
    assert_eq!(Algorithm::Brotli.tag(), 0x01);
    assert_eq!(Algorithm::Zstd.tag(), 0x02);
    assert_eq!(Algorithm::Lz4.tag(), 0x03);
    for t in [0x01u8, 0x02, 0x03] {
        assert_eq!(Algorithm::from_tag(t).unwrap().tag(), t);
    }
}

#[test]
fn missing_tag() {
    assert!(matches!(decompress(&[]), Err(Error::MissingHeader)));
    assert!(matches!(split_header(&[]), Err(Error::MissingHeader)));
    for c in enabled() {
        assert!(matches!(c.decompress(&[]), Err(Error::MissingHeader)));
    }
}

#[test]
fn unknown_tag() {
    for t in [0x00u8, 0x04, 0x7F, 0xFF] {
        assert!(matches!(decompress(&[t, 1, 2, 3]), Err(Error::UnknownTag(x)) if x == t));
        for c in enabled() {
            assert!(matches!(c.decompress(&[t]), Err(Error::UnknownTag(x)) if x == t));
        }
    }
}

#[test]
fn disabled_features_error() {
    let all = [
        (Algorithm::Brotli, cfg!(feature = "brotli")),
        (Algorithm::Zstd, cfg!(feature = "zstd")),
        (Algorithm::Lz4, cfg!(feature = "lz4")),
    ];
    for (alg, on) in all {
        if !on {
            let r = decompress(&[alg.tag(), 0, 0, 0, 0]);
            assert!(
                matches!(r, Err(Error::AlgorithmDisabled(a)) if a == alg),
                "{alg:?}: {r:?}"
            );
        }
    }
}

#[test]
fn cross_tag_mismatch() {
    let cs = enabled();
    for a in &cs {
        for b in &cs {
            if a.algorithm() == b.algorithm() {
                continue;
            }
            let packed = a.compress(b"x").unwrap();
            assert!(matches!(
                b.decompress(&packed),
                Err(Error::TagMismatch { expected, found })
                    if expected == b.algorithm() && found == a.algorithm()
            ));
        }
    }
}

#[test]
fn error_display() {
    assert!(Error::MissingHeader.to_string().contains("empty"));
    assert!(Error::UnknownTag(0xAB).to_string().contains("0xab"));
    assert!(Error::AlgorithmDisabled(Algorithm::Lz4)
        .to_string()
        .contains("Lz4"));
}
