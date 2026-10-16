// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! A dependency-free energy voice-activity detector: cheap enough to run
//! before any model, good enough to trim silence off a clip.

/// One speech span in samples (inclusive start, exclusive end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VadSpan {
    /// First sample of the span.
    pub start: usize,
    /// One past the last sample of the span.
    pub end: usize,
}

/// Energy VAD: 30 ms frames, merge gaps shorter than 300 ms, pad 150 ms.
/// A clip with no frame above the threshold comes back as one span over the
/// whole clip, because tones and quiet speech must not be dropped.
pub fn energy_vad(samples: &[f32], sample_rate: u32) -> Vec<VadSpan> {
    if samples.is_empty() {
        return Vec::new();
    }
    let sr = sample_rate.max(1) as usize;
    let frame = (sr * 30 / 1000).max(1);
    let pad = sr * 150 / 1000;
    let merge = sr * 300 / 1000;
    let n_frames = samples.len().div_ceil(frame);
    let mut rms = Vec::with_capacity(n_frames);
    for i in 0..n_frames {
        let a = i * frame;
        let b = (a + frame).min(samples.len());
        let e = samples[a..b].iter().map(|s| s * s).sum::<f32>() / (b - a) as f32;
        rms.push(e.sqrt());
    }
    let peak = rms.iter().copied().fold(0.0f32, f32::max);
    let thresh = (peak * 0.15).max(0.01);
    let speech: Vec<bool> = rms.iter().map(|&e| e >= thresh).collect();
    let mut spans: Vec<VadSpan> = Vec::new();
    let mut i = 0;
    while i < n_frames {
        if !speech[i] {
            i += 1;
            continue;
        }
        let start_f = i;
        while i < n_frames && speech[i] {
            i += 1;
        }
        let start = (start_f * frame).saturating_sub(pad);
        let end = ((i * frame).min(samples.len()) + pad).min(samples.len());
        if let Some(last) = spans.last_mut()
            && start <= last.end + merge
        {
            last.end = end;
            continue;
        }
        spans.push(VadSpan { start, end });
    }
    if spans.is_empty() {
        vec![VadSpan {
            start: 0,
            end: samples.len(),
        }]
    } else {
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 16_000;

    #[test]
    fn energy_vad_keeps_tone_drops_leading_silence_shape() {
        let sr = SR as usize;
        let mut pcm = vec![0.0f32; sr]; // 1 s silence
        for i in 0..sr / 2 {
            pcm.push((i as f32 * 0.1).sin() * 0.5);
        }
        pcm.extend(std::iter::repeat_n(0.0, sr / 4));
        let spans = energy_vad(&pcm, SR);
        assert!(!spans.is_empty());
        assert!(spans[0].end > spans[0].start);
        assert!(spans[0].start < pcm.len());
    }

    #[test]
    fn energy_vad_trims_the_silence_around_a_tone() {
        let sr = SR as usize;
        let mut pcm = vec![0.0f32; sr];
        pcm.extend((0..sr / 2).map(|i| (i as f32 * 0.1).sin() * 0.5));
        pcm.extend(vec![0.0f32; sr]);
        let spans = energy_vad(&pcm, SR);
        assert_eq!(spans.len(), 1);
        // The 150 ms pad starts the span just before the tone, within one
        // 30 ms frame of the exact tone start, not at 0.
        let padded_start = sr - sr * 150 / 1000;
        assert!((padded_start - sr * 30 / 1000..=padded_start).contains(&spans[0].start));
        assert!(spans[0].end < pcm.len());
    }

    #[test]
    fn energy_vad_merges_gaps_shorter_than_300_ms_and_splits_longer_ones() {
        let sr = SR as usize;
        let burst = |pcm: &mut Vec<f32>| pcm.extend((0..sr / 5).map(|i| (i as f32 * 0.1).sin()));
        let mut near = Vec::new();
        burst(&mut near);
        near.extend(vec![0.0; sr / 10]);
        burst(&mut near);
        assert_eq!(energy_vad(&near, SR).len(), 1);
        let mut far = Vec::new();
        burst(&mut far);
        far.extend(vec![0.0; sr]);
        burst(&mut far);
        assert_eq!(energy_vad(&far, SR).len(), 2);
    }

    #[test]
    fn energy_vad_keeps_a_silent_clip_whole_and_an_empty_one_empty() {
        assert_eq!(
            energy_vad(&[0.0; 1000], SR),
            [VadSpan {
                start: 0,
                end: 1000
            }]
        );
        assert!(energy_vad(&[], SR).is_empty());
    }
}
