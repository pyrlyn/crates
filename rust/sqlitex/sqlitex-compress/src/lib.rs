//! Algorithm-agnostic compression for SQLite BLOB data.
//!
//! The core is the [`Compressor`] trait and the [`Error`] type. Each algorithm
//! lives in its own module behind a Cargo feature (`brotli`, `zstd`, `lz4`);
//! no feature is enabled by default, so without one no algorithm code is
//! compiled.
//!
//! Every compressed blob starts with a one-byte [`Algorithm`] tag, so
//! [`decompress`] can pick the right algorithm without extra metadata.
//!
//! Tests for all feature combinations and the criterion and SQLite benchmarks are
//! described in the crate README (`## Testing`, `## Benchmarks`).

use std::fmt;

#[cfg(feature = "brotli")]
pub mod brotli;
#[cfg(feature = "lz4")]
pub mod lz4;
#[cfg(feature = "zstd")]
pub mod zstd;

/// Compression level hint. Each algorithm clamps it to its own range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// Fastest setting the algorithm offers.
    Fastest,
    /// The algorithm's default.
    #[default]
    Default,
    /// Best compression ratio the algorithm offers.
    Best,
    /// An explicit algorithm-specific level.
    Custom(i32),
}

/// Algorithm tag stored as the first byte of every compressed blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Algorithm {
    /// brotli (tag `0x01`).
    Brotli = 0x01,
    /// zstd (tag `0x02`).
    Zstd = 0x02,
    /// lz4 block format with prepended size (tag `0x03`).
    Lz4 = 0x03,
}

impl Algorithm {
    /// The tag byte.
    pub fn tag(self) -> u8 {
        self as u8
    }

    /// Parse a tag byte.
    pub fn from_tag(tag: u8) -> Result<Self, Error> {
        match tag {
            0x01 => Ok(Algorithm::Brotli),
            0x02 => Ok(Algorithm::Zstd),
            0x03 => Ok(Algorithm::Lz4),
            other => Err(Error::UnknownTag(other)),
        }
    }
}

/// Errors from compression or decompression.
#[derive(Debug)]
pub enum Error {
    /// Input was empty where a tag byte was expected.
    MissingHeader,
    /// The tag byte names no known algorithm.
    UnknownTag(u8),
    /// The tag names a known algorithm whose feature is not compiled in.
    AlgorithmDisabled(Algorithm),
    /// The tag belongs to a different algorithm than the compressor used.
    TagMismatch {
        /// Tag the compressor expected.
        expected: Algorithm,
        /// Tag found in the data.
        found: Algorithm,
    },
    /// The algorithm failed (corrupt input, I/O inside the codec).
    Codec {
        /// Algorithm that failed.
        algorithm: Algorithm,
        /// Codec message.
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MissingHeader => write!(f, "compressed data is empty, no algorithm tag"),
            Error::UnknownTag(t) => write!(f, "unknown algorithm tag 0x{t:02x}"),
            Error::AlgorithmDisabled(a) => {
                write!(f, "algorithm {a:?} is not enabled in this build")
            }
            Error::TagMismatch { expected, found } => {
                write!(f, "expected {expected:?} data, found {found:?}")
            }
            Error::Codec { algorithm, message } => write!(f, "{algorithm:?}: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// A compression algorithm for SQLite BLOB data.
///
/// `compress` output always starts with [`Compressor::algorithm`]'s tag byte;
/// `decompress` checks it and fails with [`Error::TagMismatch`] otherwise.
pub trait Compressor {
    /// Algorithm this compressor implements.
    fn algorithm(&self) -> Algorithm;
    /// Compress `data`, returning a tagged blob.
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>, Error>;
    /// Decompress a tagged blob produced by [`Compressor::compress`].
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, Error>;
}

/// Split a tagged blob into its algorithm and payload.
pub fn split_header(data: &[u8]) -> Result<(Algorithm, &[u8]), Error> {
    let (&tag, rest) = data.split_first().ok_or(Error::MissingHeader)?;
    Ok((Algorithm::from_tag(tag)?, rest))
}

/// Check the tag of `data` against `expected` and return the payload.
pub fn expect_header(data: &[u8], expected: Algorithm) -> Result<&[u8], Error> {
    let (found, rest) = split_header(data)?;
    if found != expected {
        return Err(Error::TagMismatch { expected, found });
    }
    Ok(rest)
}

fn with_header(algorithm: Algorithm, mut payload: Vec<u8>) -> Vec<u8> {
    payload.insert(0, algorithm.tag());
    payload
}

#[allow(dead_code)]
fn codec_err(algorithm: Algorithm, e: impl fmt::Display) -> Error {
    Error::Codec {
        algorithm,
        message: e.to_string(),
    }
}

/// Decompress any tagged blob with the default compressor for its tag.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, Error> {
    let (algorithm, _) = split_header(data)?;
    match algorithm {
        #[cfg(feature = "brotli")]
        Algorithm::Brotli => brotli::Brotli::default().decompress(data),
        #[cfg(feature = "zstd")]
        Algorithm::Zstd => zstd::Zstd::default().decompress(data),
        #[cfg(feature = "lz4")]
        Algorithm::Lz4 => lz4::Lz4.decompress(data),
        #[allow(unreachable_patterns)]
        other => Err(Error::AlgorithmDisabled(other)),
    }
}

#[allow(dead_code)]
pub(crate) fn tagged(algorithm: Algorithm, payload: Vec<u8>) -> Vec<u8> {
    with_header(algorithm, payload)
}
