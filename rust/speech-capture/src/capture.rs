// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! `Recorder`: the default microphone as mono `f32` at 16 kHz, held in
//! memory only and capped in length. The input stream's sink never blocks or
//! allocates: it writes straight into the producer half of a fixed-size ring
//! buffer allocated before the stream starts, and counts the samples the full
//! buffer had no room for. `feed` is a plain function so it tests without a
//! microphone; the device itself is opened by [`InputDevice`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapRb};

use crate::Error;
use crate::input::{InputDevice, InputStream};
use crate::resample::resample_to_16k;

/// A press of the key in progress. Dropping the recorder (cancel) stops the
/// stream and discards the audio.
pub struct Recorder {
    samples: HeapCons<f32>,
    overflow: Arc<AtomicU64>,
    rate: u32,
    stream: InputStream,
}

impl Recorder {
    /// Opens the default input device and keeps at most `max` of audio.
    pub fn start(max: Duration) -> Result<Self, Error> {
        let device = InputDevice::find(None)?;
        let rate = device.sample_rate_hz();
        // Sized once, here, so the sink never allocates. `HeapRb` rejects a
        // zero capacity.
        let cap = ((f64::from(rate) * max.as_secs_f64()) as usize).max(1);
        let (mut producer, samples) = HeapRb::<f32>::new(cap).split();
        let overflow = Arc::new(AtomicU64::new(0));
        let dropped = Arc::clone(&overflow);
        let stream = device.start(move |mono| feed(mono, &mut producer, &dropped))?;
        Ok(Self {
            samples,
            overflow,
            rate,
            stream,
        })
    }

    /// Samples at the device rate dropped so far because the recording was
    /// already `max` long.
    pub fn overflowed_samples(&self) -> u64 {
        self.overflow.load(Ordering::Relaxed)
    }

    /// Stops the stream and returns the audio resampled to 16 kHz.
    pub fn stop(self) -> Result<Vec<f32>, Error> {
        let Self {
            mut samples,
            overflow,
            rate,
            stream,
        } = self;
        // Dropping the guard waits for the stream thread, so nothing writes to
        // the ring while it is drained below.
        drop(stream);
        let dropped = overflow.load(Ordering::Relaxed);
        if dropped > 0 {
            tracing::warn!(
                dropped,
                "recording hit its length cap; the tail was dropped"
            );
        }
        let mono: Vec<f32> = samples.pop_iter().collect();
        resample_to_16k(&mono, rate)
    }
}

/// The recorder's sink: pushes each sample, counting in `overflow` the ones the
/// ring has no room for and dropping them, so the recording keeps its first
/// samples, as a cap does. Wait-free: no lock, no allocation, no syscall.
fn feed(mono: &[f32], producer: &mut impl Producer<Item = f32>, overflow: &AtomicU64) {
    for &sample in mono {
        if producer.try_push(sample).is_err() {
            overflow.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::HeapProd;

    /// A ring of `cap` samples and its two halves, as `start` builds it.
    fn ring(cap: usize) -> (HeapProd<f32>, HeapCons<f32>) {
        HeapRb::<f32>::new(cap).split()
    }

    fn drain(consumer: &mut HeapCons<f32>) -> Vec<f32> {
        consumer.pop_iter().collect()
    }

    #[test]
    fn feed_keeps_what_was_pushed_in_order() {
        let overflow = AtomicU64::new(0);
        let (mut tx, mut rx) = ring(8);
        feed(&[0.5, 0.5, 0.0], &mut tx, &overflow);
        assert_eq!(drain(&mut rx), [0.5, 0.5, 0.0]);
        feed(&[0.25, -0.25], &mut tx, &overflow);
        assert_eq!(drain(&mut rx), [0.25, -0.25]);
        assert_eq!(overflow.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_full_ring_counts_dropped_samples_keeps_the_first_ones_and_never_panics() {
        let overflow = AtomicU64::new(0);
        let (mut tx, mut rx) = ring(5);
        feed(&[0.1; 3], &mut tx, &overflow);
        feed(&[0.2; 3], &mut tx, &overflow);
        assert_eq!(overflow.load(Ordering::Relaxed), 1);
        feed(&[0.3; 10], &mut tx, &overflow);
        assert_eq!(overflow.load(Ordering::Relaxed), 11);
        assert_eq!(drain(&mut rx), [0.1, 0.1, 0.1, 0.2, 0.2]);
        // Draining frees the room again.
        feed(&[0.4; 2], &mut tx, &overflow);
        assert_eq!(drain(&mut rx), [0.4, 0.4]);
        assert_eq!(overflow.load(Ordering::Relaxed), 11);
    }
}
