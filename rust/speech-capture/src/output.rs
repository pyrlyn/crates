// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! The one place that opens a `cpal` output stream, the mirror of `input`. A
//! caller finds a device (by name or the default), reads its native rate and
//! starts it with a source that fills mono `f32` at that rate. The callback
//! converts the source's samples into whatever format and channel count the
//! driver wants in a small stack buffer: it takes no lock and makes no
//! allocation of its own, so a wait-free source keeps the whole callback
//! wait-free.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use crate::Error;
use crate::stream::StreamThread;

/// Frames rendered per source call. A stack buffer, so the callback never
/// allocates whatever size the driver asks for.
const CHUNK_FRAMES: usize = 512;

fn output_error(e: cpal::Error) -> Error {
    Error::Output(e.to_string())
}

/// An output device that was found and configured but is not streaming yet.
pub struct OutputDevice {
    device: cpal::Device,
    name: String,
    rate: u32,
    channels: usize,
    format: SampleFormat,
    config: cpal::StreamConfig,
}

impl std::fmt::Debug for OutputDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputDevice")
            .field("name", &self.name)
            .field("rate", &self.rate)
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

impl OutputDevice {
    /// The output device called `name`, or the default one for `None`. Opens
    /// no stream, so it makes no sound.
    pub fn find(name: Option<&str>) -> Result<Self, Error> {
        let host = cpal::default_host();
        let device = match name {
            None => host.default_output_device().ok_or(Error::NoOutputDevice)?,
            Some(wanted) => host
                .output_devices()
                .map_err(output_error)?
                .find(|d| d.description().is_ok_and(|d| d.name() == wanted))
                .ok_or_else(|| Error::OutputDeviceNotFound(wanted.to_owned()))?,
        };
        let name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_default();
        let supported = device.default_output_config().map_err(output_error)?;
        Ok(Self {
            device,
            name,
            rate: supported.sample_rate(),
            channels: usize::from(supported.channels()).max(1),
            format: supported.sample_format(),
            config: supported.into(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Rate of the mono samples the source must produce.
    pub fn sample_rate_hz(&self) -> u32 {
        self.rate
    }

    /// Starts playing what `source` writes: it fills the slice with mono
    /// samples at [`sample_rate_hz`](Self::sample_rate_hz) (silence is zeros)
    /// and is called from the audio thread, so it must not block or allocate.
    /// Dropping the returned guard stops the stream.
    pub fn start(
        self,
        source: impl FnMut(&mut [f32]) + Send + 'static,
    ) -> Result<OutputStream, Error> {
        StreamThread::spawn("speech-capture-out", move || self.open(source))
            .map(|_thread| OutputStream { _thread })
    }

    fn open(self, source: impl FnMut(&mut [f32]) + Send + 'static) -> Result<cpal::Stream, Error> {
        let Self {
            device,
            channels,
            format,
            config,
            ..
        } = self;
        let stream = match format {
            SampleFormat::I8 => build::<i8>(&device, config, channels, source),
            SampleFormat::I16 => build::<i16>(&device, config, channels, source),
            SampleFormat::I32 => build::<i32>(&device, config, channels, source),
            SampleFormat::I64 => build::<i64>(&device, config, channels, source),
            SampleFormat::U8 => build::<u8>(&device, config, channels, source),
            SampleFormat::U16 => build::<u16>(&device, config, channels, source),
            SampleFormat::U32 => build::<u32>(&device, config, channels, source),
            SampleFormat::U64 => build::<u64>(&device, config, channels, source),
            SampleFormat::F32 => build::<f32>(&device, config, channels, source),
            SampleFormat::F64 => build::<f64>(&device, config, channels, source),
            other => Err(Error::Output(format!("unsupported sample format {other}"))),
        }?;
        stream.play().map_err(output_error)?;
        Ok(stream)
    }
}

/// A running output stream. Dropping it stops the stream and waits for the
/// stream's thread, so no source call happens after the drop returns.
#[derive(Debug)]
pub struct OutputStream {
    _thread: StreamThread,
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    mut source: impl FnMut(&mut [f32]) + Send + 'static,
) -> Result<cpal::Stream, Error>
where
    T: SizedSample + FromSample<f32>,
{
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| render(data, channels, &mut source),
            |e| tracing::warn!(error = %e, "speaker stream error"),
            None,
        )
        .map_err(output_error)
}

/// The body of the output callback: asks `source` for mono samples in
/// stack-sized chunks and writes each one to every channel of its frame. No
/// lock, no allocation, no syscall.
fn render<T>(data: &mut [T], channels: usize, source: &mut impl FnMut(&mut [f32]))
where
    T: Sample + FromSample<f32>,
{
    let channels = channels.max(1);
    let mut chunk = [0.0f32; CHUNK_FRAMES];
    for block in data.chunks_mut(channels * CHUNK_FRAMES) {
        let frames = block.len().div_ceil(channels);
        let mono = &mut chunk[..frames];
        source(mono);
        for (frame, sample) in block.chunks_mut(channels).zip(mono.iter()) {
            // A source that overshoots must clip, not wrap around in an integer format.
            frame.fill(T::from_sample(sample.clamp(-1.0, 1.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source that counts up from 1 in steps of `step`, and how many calls it took.
    fn run<T>(frames: usize, channels: usize, step: f32) -> (Vec<T>, usize)
    where
        T: Sample + FromSample<f32>,
    {
        let mut next = 0.0f32;
        let mut calls = 0;
        let mut data = vec![T::EQUILIBRIUM; frames * channels];
        render(&mut data, channels, &mut |mono: &mut [f32]| {
            for s in mono {
                next += step;
                *s = next;
            }
            calls += 1;
        });
        (data, calls)
    }

    #[test]
    fn every_channel_of_a_frame_gets_the_same_sample() {
        let (data, _) = run::<f32>(3, 2, 0.25);
        assert_eq!(data, [0.25, 0.25, 0.5, 0.5, 0.75, 0.75]);
    }

    #[test]
    fn a_big_callback_is_split_into_chunks_without_losing_or_moving_samples() {
        let (data, calls) = run::<f32>(CHUNK_FRAMES * 2 + 7, 1, 1e-4);
        assert_eq!(calls, 3);
        assert!((data[CHUNK_FRAMES * 2 + 6] - (CHUNK_FRAMES * 2 + 7) as f32 * 1e-4).abs() < 1e-3);
        assert_eq!(run::<f32>(0, 2, 1.0), (Vec::new(), 0));
    }

    #[test]
    fn integer_formats_convert_and_overshoot_clips() {
        let (data, _) = run::<i16>(2, 1, 4.0);
        assert_eq!(data, [i16::MAX, i16::MAX]);
        let (data, _) = run::<i16>(2, 1, -4.0);
        assert_eq!(data, [i16::MIN, i16::MIN]);
        let (data, _) = run::<u16>(1, 1, 0.0);
        assert_eq!(data, [32_768]);
    }

    #[test]
    fn a_missing_named_device_is_reported_by_name() {
        match OutputDevice::find(Some("no such speaker \u{1F50A}")) {
            Err(Error::OutputDeviceNotFound(name)) => assert!(name.contains("no such")),
            // A machine with no audio host at all fails differently; that is fine.
            Err(_) => {}
            Ok(_) => panic!("found a device that cannot exist"),
        }
    }
}
