// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! SIMD dot products (feature `simd`): NEON on aarch64, AVX2 + FMA on x86_64 when the CPU
//! reports both (checked at runtime), the scalar kernels everywhere else. Apart from
//! `sqlite.rs` this is the only module with `unsafe`: vector loads take raw pointers, and
//! calling an AVX2 function needs the runtime check. Every block keeps a SAFETY note.

#[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
mod arch {
    use std::arch::aarch64::*;

    use crate::distance::{Backend, dot_i8_scalar, dot_scalar};

    pub fn backend() -> Backend {
        Backend::Neon
    }

    #[inline]
    pub fn dot(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        let (ca, ra) = a[..n].as_chunks::<16>();
        let (cb, rb) = b[..n].as_chunks::<16>();
        // SAFETY: NEON is enabled for this target (cfg above). Each load reads four `f32`
        // at offset 0, 4, 8 or 12 of a 16-element array, so it stays inside the chunk.
        let s = unsafe {
            let (mut s0, mut s1, mut s2, mut s3) = (
                vdupq_n_f32(0.0),
                vdupq_n_f32(0.0),
                vdupq_n_f32(0.0),
                vdupq_n_f32(0.0),
            );
            for (x, y) in ca.iter().zip(cb) {
                let (x, y) = (x.as_ptr(), y.as_ptr());
                s0 = vfmaq_f32(s0, vld1q_f32(x), vld1q_f32(y));
                s1 = vfmaq_f32(s1, vld1q_f32(x.add(4)), vld1q_f32(y.add(4)));
                s2 = vfmaq_f32(s2, vld1q_f32(x.add(8)), vld1q_f32(y.add(8)));
                s3 = vfmaq_f32(s3, vld1q_f32(x.add(12)), vld1q_f32(y.add(12)));
            }
            vaddvq_f32(vaddq_f32(vaddq_f32(s0, s1), vaddq_f32(s2, s3)))
        };
        s + dot_scalar(ra, rb)
    }

    #[inline]
    pub fn dot_i8(q: &[f32], v: &[i8]) -> f32 {
        let n = q.len().min(v.len());
        let (cq, rq) = q[..n].as_chunks::<16>();
        let (cv, rv) = v[..n].as_chunks::<16>();
        // SAFETY: as in `dot`; the `i8` load reads exactly the 16 bytes of one chunk.
        let s = unsafe {
            let (mut s0, mut s1, mut s2, mut s3) = (
                vdupq_n_f32(0.0),
                vdupq_n_f32(0.0),
                vdupq_n_f32(0.0),
                vdupq_n_f32(0.0),
            );
            for (x, y) in cq.iter().zip(cv) {
                let x = x.as_ptr();
                let b = vld1q_s8(y.as_ptr());
                let (lo, hi) = (vmovl_s8(vget_low_s8(b)), vmovl_high_s8(b));
                let w0 = vcvtq_f32_s32(vmovl_s16(vget_low_s16(lo)));
                let w1 = vcvtq_f32_s32(vmovl_high_s16(lo));
                let w2 = vcvtq_f32_s32(vmovl_s16(vget_low_s16(hi)));
                let w3 = vcvtq_f32_s32(vmovl_high_s16(hi));
                s0 = vfmaq_f32(s0, vld1q_f32(x), w0);
                s1 = vfmaq_f32(s1, vld1q_f32(x.add(4)), w1);
                s2 = vfmaq_f32(s2, vld1q_f32(x.add(8)), w2);
                s3 = vfmaq_f32(s3, vld1q_f32(x.add(12)), w3);
            }
            vaddvq_f32(vaddq_f32(vaddq_f32(s0, s1), vaddq_f32(s2, s3)))
        };
        s + dot_i8_scalar(rq, rv)
    }
}

#[cfg(target_arch = "x86_64")]
mod arch {
    use std::arch::x86_64::*;

    use crate::distance::{Backend, dot_i8_scalar, dot_scalar};

    #[inline]
    fn avx2() -> bool {
        // std caches the CPUID result; this is an atomic load per call.
        std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma")
    }

    pub fn backend() -> Backend {
        if avx2() {
            Backend::Avx2
        } else {
            Backend::Scalar
        }
    }

    #[inline]
    pub fn dot(a: &[f32], b: &[f32]) -> f32 {
        if avx2() {
            // SAFETY: the CPU supports AVX2 and FMA (checked just above).
            unsafe { dot_avx2(a, b) }
        } else {
            dot_scalar(a, b)
        }
    }

    #[inline]
    pub fn dot_i8(q: &[f32], v: &[i8]) -> f32 {
        if avx2() {
            // SAFETY: the CPU supports AVX2 and FMA (checked just above).
            unsafe { dot_i8_avx2(q, v) }
        } else {
            dot_i8_scalar(q, v)
        }
    }

    #[target_feature(enable = "avx2,fma")]
    fn hsum(v: __m256) -> f32 {
        let s = _mm_add_ps(_mm256_castps256_ps128(v), _mm256_extractf128_ps::<1>(v));
        let s = _mm_add_ps(s, _mm_movehl_ps(s, s));
        _mm_cvtss_f32(_mm_add_ss(s, _mm_shuffle_ps::<0b01>(s, s)))
    }

    #[target_feature(enable = "avx2,fma")]
    fn dot_avx2(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        let (ca, ra) = a[..n].as_chunks::<16>();
        let (cb, rb) = b[..n].as_chunks::<16>();
        let (mut s0, mut s1) = (_mm256_setzero_ps(), _mm256_setzero_ps());
        for (x, y) in ca.iter().zip(cb) {
            let (x, y) = (x.as_ptr(), y.as_ptr());
            // SAFETY: each unaligned load reads eight `f32` at offset 0 or 8 of a
            // 16-element array, so it stays inside the chunk.
            unsafe {
                s0 = _mm256_fmadd_ps(_mm256_loadu_ps(x), _mm256_loadu_ps(y), s0);
                s1 = _mm256_fmadd_ps(_mm256_loadu_ps(x.add(8)), _mm256_loadu_ps(y.add(8)), s1);
            }
        }
        hsum(_mm256_add_ps(s0, s1)) + dot_scalar(ra, rb)
    }

    #[target_feature(enable = "avx2,fma")]
    fn dot_i8_avx2(q: &[f32], v: &[i8]) -> f32 {
        let n = q.len().min(v.len());
        let (cq, rq) = q[..n].as_chunks::<16>();
        let (cv, rv) = v[..n].as_chunks::<16>();
        let (mut s0, mut s1) = (_mm256_setzero_ps(), _mm256_setzero_ps());
        for (x, y) in cq.iter().zip(cv) {
            let x = x.as_ptr();
            // SAFETY: the `i8` load reads exactly the 16 bytes of one chunk; each `f32`
            // load reads eight values at offset 0 or 8 of a 16-element array.
            unsafe {
                let b = _mm_loadu_si128(y.as_ptr().cast());
                let lo = _mm256_cvtepi32_ps(_mm256_cvtepi8_epi32(b));
                let hi = _mm256_cvtepi32_ps(_mm256_cvtepi8_epi32(_mm_srli_si128::<8>(b)));
                s0 = _mm256_fmadd_ps(_mm256_loadu_ps(x), lo, s0);
                s1 = _mm256_fmadd_ps(_mm256_loadu_ps(x.add(8)), hi, s1);
            }
        }
        hsum(_mm256_add_ps(s0, s1)) + dot_i8_scalar(rq, rv)
    }
}

#[cfg(not(any(
    all(target_arch = "aarch64", target_feature = "neon"),
    target_arch = "x86_64"
)))]
mod arch {
    pub use crate::distance::{dot_i8_scalar as dot_i8, dot_scalar as dot};

    pub fn backend() -> crate::distance::Backend {
        crate::distance::Backend::Scalar
    }
}

pub use arch::{backend, dot, dot_i8};
