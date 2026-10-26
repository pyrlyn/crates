// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Dot-product kernels behind the cosine distance.
//!
//! [`dot`] and [`dot_i8`] use SIMD when the `simd` feature is on and the CPU has it (NEON on
//! aarch64, AVX2 + FMA on x86_64, detected at runtime), and the scalar [`dot_scalar`] /
//! [`dot_i8_scalar`] otherwise. SIMD sums in another order and fuses multiply-add, so its
//! result can differ from the scalar one in the last bits.

/// The kernel [`dot`] and [`dot_i8`] use on this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Portable Rust (left to the compiler's auto-vectorization).
    Scalar,
    /// aarch64 NEON.
    Neon,
    /// x86_64 AVX2 with FMA.
    Avx2,
}

#[cfg(feature = "simd")]
use crate::simd as imp;

#[cfg(not(feature = "simd"))]
mod imp {
    pub use super::{dot_i8_scalar as dot_i8, dot_scalar as dot};
    pub fn backend() -> super::Backend {
        super::Backend::Scalar
    }
}

/// The kernel in use: [`Backend::Scalar`] without the `simd` feature or a capable CPU.
pub fn backend() -> Backend {
    imp::backend()
}

/// `sum(a[i] * b[i])` over the shorter of the two slices.
#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    imp::dot(a, b)
}

/// `sum(q[i] * v[i])` with `i8` components widened to `f32`, over the shorter slice.
#[inline]
pub fn dot_i8(q: &[f32], v: &[i8]) -> f32 {
    imp::dot_i8(q, v)
}

/// Scalar [`dot`]: eight independent partial sums, then the tail.
#[inline]
pub fn dot_scalar(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    let mut acc = [0f32; 8];
    let (ca, ra) = a.as_chunks::<8>();
    let (cb, rb) = b.as_chunks::<8>();
    for (x, y) in ca.iter().zip(cb) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

/// Scalar [`dot_i8`]: eight independent partial sums, then the tail.
#[inline]
pub fn dot_i8_scalar(q: &[f32], v: &[i8]) -> f32 {
    let n = q.len().min(v.len());
    let (q, v) = (&q[..n], &v[..n]);
    let mut acc = [0f32; 8];
    let (cq, rq) = q.as_chunks::<8>();
    let (cv, rv) = v.as_chunks::<8>();
    for (x, y) in cq.iter().zip(cv) {
        for k in 0..8 {
            acc[k] += x[k] * y[k] as f32;
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in rq.iter().zip(rv) {
        s += x * *y as f32;
    }
    s
}
