//! lz4 (feature `lz4`, crate `lz4_flex`, block format with prepended size).

use crate::{codec_err, expect_header, tagged, Algorithm, Compressor, Error};

/// lz4 compressor. `lz4_flex` has no levels, so there is no level setting.
#[derive(Debug, Clone, Copy, Default)]
pub struct Lz4;

impl Compressor for Lz4 {
    fn algorithm(&self) -> Algorithm {
        Algorithm::Lz4
    }

    fn compress(&self, data: &[u8]) -> Result<Vec<u8>, Error> {
        Ok(tagged(
            Algorithm::Lz4,
            lz4_flex::compress_prepend_size(data),
        ))
    }

    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, Error> {
        let payload = expect_header(data, Algorithm::Lz4)?;
        lz4_flex::decompress_size_prepended(payload).map_err(|e| codec_err(Algorithm::Lz4, e))
    }
}
