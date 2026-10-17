// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! Mono resampling with `rubato`, in two flavours that the two original
//! callers already depend on: `resample_to_16k` (FFT, fast, for a live
//! recording) and `resample_mono` (windowed sinc, for decoded files, whose
//! output hashes must stay stable). Both take the whole clip from memory.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, Fft, FixedAsync, FixedSync, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};

use crate::Error;

/// The rate speech-to-text engines (whisper.cpp among them) take.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Mono audio at `rate` to [`TARGET_SAMPLE_RATE`], same duration; a clip
/// already at the target rate comes back unchanged.
pub fn resample_to_16k(mono: &[f32], rate: u32) -> Result<Vec<f32>, Error> {
    if rate == TARGET_SAMPLE_RATE || mono.is_empty() {
        return Ok(mono.to_vec());
    }
    let resample = |e: String| Error::Resample(format!("to 16 kHz: {e}"));
    let mut resampler = Fft::<f32>::new(
        rate as usize,
        TARGET_SAMPLE_RATE as usize,
        1024,
        1,
        FixedSync::Both,
    )
    .map_err(|e| resample(e.to_string()))?;
    let input =
        InterleavedSlice::new(mono, 1, mono.len()).map_err(|e| resample(format!("{e:?}")))?;
    let output = resampler
        .process_all(&input, mono.len(), None)
        .map_err(|e| resample(e.to_string()))?;
    Ok(output.take_data())
}

/// Mono audio from `from` Hz to `to` Hz with a windowed-sinc resampler.
pub fn resample_mono(input: &[f32], from: u32, to: u32) -> Result<Vec<f32>, Error> {
    if from == 0 {
        return Err(Error::Resample("source sample rate is 0".into()));
    }
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let ratio = f64::from(to) / f64::from(from);
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: Some(0.95),
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    // The input is fully in memory, so `process_all` handles chunking and
    // delay trimming itself.
    let mut resampler = Async::<f32>::new_sinc(ratio, 2.0, &params, 1024, 1, FixedAsync::Input)
        .map_err(|e| Error::Resample(e.to_string()))?;
    let adapter = InterleavedSlice::new(input, 1, input.len())
        .map_err(|e| Error::Resample(format!("adapter: {e:?}")))?;
    let rendered = resampler
        .process_all(&adapter, input.len(), None)
        .map_err(|e| Error::Resample(e.to_string()))?;
    let mut out = rendered.take_data();
    let expected = (input.len() as f64 * ratio).round() as usize;
    if out.len() > expected + expected / 10 + 64 {
        out.truncate(expected);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32) -> Vec<f32> {
        (0..rate)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn resample_48k_to_16k_keeps_the_duration() {
        let out = resample_to_16k(&tone(48_000), 48_000).expect("resamples");
        assert!(
            (out.len() as i64 - 16_000).abs() <= 16,
            "{} samples",
            out.len()
        );
        assert!(out.iter().all(|s| s.abs() <= 1.0));
        assert_eq!(
            resample_to_16k(&[0.1, 0.2], 16_000).expect("no-op"),
            [0.1, 0.2]
        );
    }

    #[test]
    fn sinc_resample_8k_to_16k_doubles_the_length() {
        let out = resample_mono(&tone(8_000), 8_000, TARGET_SAMPLE_RATE).expect("resamples");
        assert!(out.len() > 14_000 && out.len() < 18_000, "{}", out.len());
    }

    #[test]
    fn sinc_resample_rejects_a_zero_source_rate_and_passes_empty_input() {
        assert!(matches!(
            resample_mono(&[0.1], 0, 16_000),
            Err(Error::Resample(_))
        ));
        assert!(resample_mono(&[], 8_000, 16_000).expect("empty").is_empty());
    }
}
