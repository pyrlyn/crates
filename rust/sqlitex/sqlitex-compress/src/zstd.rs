//! zstd (feature `zstd`, crate `zstd`).

use crate::{codec_err, expect_header, tagged, Algorithm, Compressor, Error, Level};

/// zstd compressor. Levels: fastest 1, default 3, best 19, custom clamped to 1..=22.
#[derive(Debug, Clone, Copy, Default)]
pub struct Zstd {
    /// Compression level.
    pub level: Level,
}

impl Zstd {
    fn level(&self) -> i32 {
        match self.level {
            Level::Fastest => 1,
            Level::Default => 3,
            Level::Best => 19,
            Level::Custom(l) => l.clamp(1, 22),
        }
    }
}

impl Compressor for Zstd {
    fn algorithm(&self) -> Algorithm {
        Algorithm::Zstd
    }

    fn compress(&self, data: &[u8]) -> Result<Vec<u8>, Error> {
        let out = ::zstd::bulk::compress(data, self.level())
            .map_err(|e| codec_err(Algorithm::Zstd, e))?;
        Ok(tagged(Algorithm::Zstd, out))
    }

    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, Error> {
        let payload = expect_header(data, Algorithm::Zstd)?;
        ::zstd::stream::decode_all(payload).map_err(|e| codec_err(Algorithm::Zstd, e))
    }
}
