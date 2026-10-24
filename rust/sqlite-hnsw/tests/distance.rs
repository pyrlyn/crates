//! The dispatched kernels (SIMD with the `simd` feature) agree with the scalar ones.

mod common;

use sqlite_hnsw::distance::{Backend, backend, dot, dot_i8, dot_i8_scalar, dot_scalar};

/// SIMD reorders the sum and fuses multiply-add, so allow a few ulps of the magnitude.
fn close(simd: f32, scalar: f32, magnitude: f32) -> bool {
    (simd - scalar).abs() <= 1e-5 * magnitude + 1e-6
}

#[test]
fn f32_kernel_matches_scalar_for_every_length() {
    for len in 0..=133 {
        let v = common::vecs(2, len, 0x5eed + len as u64);
        let (a, b) = (&v[0], &v[1]);
        let mag: f32 = a.iter().zip(b).map(|(x, y)| (x * y).abs()).sum();
        let (s, r) = (dot(a, b), dot_scalar(a, b));
        assert!(close(s, r, mag), "len {len}: {s} vs {r}");
    }
}

#[test]
fn i8_kernel_matches_scalar_for_every_length() {
    for len in 0..=133 {
        let q = &common::vecs(1, len, 0xabc + len as u64)[0];
        let v: Vec<i8> = (0..len)
            .map(|i| ((i * 37 + len) % 255) as u8 as i8)
            .collect();
        let mag: f32 = q.iter().zip(&v).map(|(x, y)| (x * *y as f32).abs()).sum();
        let (s, r) = (dot_i8(q, &v), dot_i8_scalar(q, &v));
        assert!(close(s, r, mag), "len {len}: {s} vs {r}");
    }
}

#[test]
fn extreme_i8_values_are_widened_exactly() {
    let q = vec![1.0f32; 48];
    let v: Vec<i8> = (0..48)
        .map(|i| if i % 2 == 0 { i8::MIN } else { i8::MAX })
        .collect();
    assert_eq!(dot_i8(&q, &v), -24.0);
    assert_eq!(dot_i8_scalar(&q, &v), -24.0);
}

#[test]
fn integer_valued_inputs_are_exact() {
    // Small integers sum exactly in f32 in any order, so SIMD must equal scalar bit for bit.
    let a: Vec<f32> = (0..100).map(|i| (i % 7) as f32 - 3.0).collect();
    let b: Vec<f32> = (0..100).map(|i| (i % 5) as f32 - 2.0).collect();
    assert_eq!(dot(&a, &b), dot_scalar(&a, &b));
}

#[test]
fn mismatched_lengths_use_the_shorter_slice() {
    let a = vec![1.0f32; 40];
    let b = vec![2.0f32; 25];
    assert_eq!(dot(&a, &b), 50.0);
    assert_eq!(dot_scalar(&a, &b), 50.0);
    assert_eq!(dot_i8(&a, &[3i8; 17]), 51.0);
    assert_eq!(dot_i8_scalar(&a, &[3i8; 17]), 51.0);
}

#[test]
fn backend_reflects_the_feature() {
    let b = backend();
    println!("distance backend: {b:?}");
    if cfg!(not(feature = "simd")) {
        assert_eq!(b, Backend::Scalar);
    } else if cfg!(all(target_arch = "aarch64", target_feature = "neon")) {
        assert_eq!(b, Backend::Neon);
    } else if cfg!(target_arch = "x86_64") {
        assert!(matches!(b, Backend::Avx2 | Backend::Scalar), "{b:?}");
    }
}
