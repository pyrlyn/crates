//! brotli (feature `brotli`, crate `brotli`).

use std::io::{Read, Write};

use crate::{codec_err, expect_header, tagged, Algorithm, Compressor, Error, Level};

/// brotli compressor. Quality: fastest 0, default 6, best 11, custom clamped to 0..=11.
#[derive(Debug, Clone, Copy, Default)]
pub struct Brotli {
    /// Compression level (brotli quality).
    pub level: Level,
}

impl Brotli {
    fn quality(&self) -> u32 {
        match self.level {
            Level::Fastest => 0,
            Level::Default => 6,
            Level::Best => 11,
            Level::Custom(l) => l.clamp(0, 11) as u32,
        }
    }
}

impl Compressor for Brotli {
    fn algorithm(&self) -> Algorithm {
        Algorithm::Brotli
    }

    fn compress(&self, data: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        {
            let mut w = ::brotli::CompressorWriter::new(&mut out, 4096, self.quality(), 22);
            w.write_all(data)
                .map_err(|e| codec_err(Algorithm::Brotli, e))?;
            w.flush().map_err(|e| codec_err(Algorithm::Brotli, e))?;
        }
        Ok(tagged(Algorithm::Brotli, out))
    }

    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, Error> {
        let payload = expect_header(data, Algorithm::Brotli)?;
        let mut out = Vec::new();
        ::brotli::Decompressor::new(payload, 4096)
            .read_to_end(&mut out)
            .map_err(|e| codec_err(Algorithm::Brotli, e))?;
        Ok(out)
    }
}
