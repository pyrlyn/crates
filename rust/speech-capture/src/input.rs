// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! The one place that opens a `cpal` input stream. A caller finds a device (by
//! name or the default), reads its native rate, and starts it with a sink that
//! receives mono `f32` at that rate. The callback converts and downmixes into a
//! small stack buffer and hands it to the sink: it takes no lock and makes no
//! allocation of its own, so a sink that is wait-free keeps the whole callback
//! wait-free. The stream lives on its own thread (see `stream`), because a
//! `cpal` stream is not `Send` on every platform; the guard that stops it is.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

use crate::Error;
use crate::stream::StreamThread;

/// Frames converted per sink call. A stack buffer, so the callback never
/// allocates whatever size the driver delivers.
const CHUNK_FRAMES: usize = 512;

fn stream_error(e: cpal::Error) -> Error {
    Error::Stream(e.to_string())
}

/// The default input device's name, `None` without one; opens no stream,
/// so it records nothing and raises no permission prompt (a doctor/health check).
pub fn input_device() -> Option<String> {
    let device = cpal::default_host().default_input_device()?;
    device.description().ok().map(|d| d.name().to_string())
}

/// An input device that was found and configured but is not streaming yet.
pub struct InputDevice {
    device: cpal::Device,
    name: String,
    rate: u32,
    channels: usize,
    format: SampleFormat,
    config: cpal::StreamConfig,
}

impl std::fmt::Debug for InputDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputDevice")
            .field("name", &self.name)
            .field("rate", &self.rate)
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

impl InputDevice {
    /// The input device called `name`, or the default one for `None`. Opens
    /// no stream, so it raises no permission prompt.
    pub fn find(name: Option<&str>) -> Result<Self, Error> {
        let host = cpal::default_host();
        let device = match name {
            None => host.default_input_device().ok_or(Error::NoInputDevice)?,
            Some(wanted) => host
                .input_devices()
                .map_err(stream_error)?
                .find(|d| d.description().is_ok_and(|d| d.name() == wanted))
                .ok_or_else(|| Error::DeviceNotFound(wanted.to_owned()))?,
        };
        let name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_default();
        let supported = device.default_input_config().map_err(stream_error)?;
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

    /// Rate of the mono samples the sink will receive.
    pub fn sample_rate_hz(&self) -> u32 {
        self.rate
    }

    /// Starts streaming mono samples at [`sample_rate_hz`](Self::sample_rate_hz)
    /// into `sink`, called from the audio thread: it must not block or
    /// allocate. Dropping the returned guard stops the stream.
    pub fn start(self, sink: impl FnMut(&[f32]) + Send + 'static) -> Result<InputStream, Error> {
        StreamThread::spawn("speech-capture", move || self.open(sink))
            .map(|_thread| InputStream { _thread })
    }

    fn open(self, sink: impl FnMut(&[f32]) + Send + 'static) -> Result<cpal::Stream, Error> {
        let Self {
            device,
            channels,
            format,
            config,
            ..
        } = self;
        let stream = match format {
            SampleFormat::I8 => build::<i8>(&device, config, channels, sink),
            SampleFormat::I16 => build::<i16>(&device, config, channels, sink),
            SampleFormat::I32 => build::<i32>(&device, config, channels, sink),
            SampleFormat::I64 => build::<i64>(&device, config, channels, sink),
            SampleFormat::U8 => build::<u8>(&device, config, channels, sink),
            SampleFormat::U16 => build::<u16>(&device, config, channels, sink),
            SampleFormat::U32 => build::<u32>(&device, config, channels, sink),
            SampleFormat::U64 => build::<u64>(&device, config, channels, sink),
            SampleFormat::F32 => build::<f32>(&device, config, channels, sink),
            SampleFormat::F64 => build::<f64>(&device, config, channels, sink),
            other => Err(Error::Stream(format!("unsupported sample format {other}"))),
        }?;
        stream.play().map_err(stream_error)?;
        Ok(stream)
    }
}

/// A running input stream. Dropping it stops the stream and waits for the
/// stream's thread, so no sink call happens after the drop returns.
#[derive(Debug)]
pub struct InputStream {
    _thread: StreamThread,
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    mut sink: impl FnMut(&[f32]) + Send + 'static,
) -> Result<cpal::Stream, Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| deliver(data, channels, &mut sink),
            |e| tracing::warn!(error = %e, "microphone stream error"),
            None,
        )
        .map_err(stream_error)
}

/// The body of the input callback: converts `data` to `f32`, averages each
/// interleaved frame to mono and passes it on in stack-sized chunks. No lock,
/// no allocation, no syscall.
fn deliver<T>(data: &[T], channels: usize, sink: &mut impl FnMut(&[f32]))
where
    T: Sample,
    f32: FromSample<T>,
{
    let channels = channels.max(1);
    let mut chunk = [0.0f32; CHUNK_FRAMES];
    for block in data.chunks(channels * CHUNK_FRAMES) {
        let mut filled = 0;
        for frame in block.chunks(channels) {
            let sum: f32 = frame.iter().map(|s| s.to_sample::<f32>()).sum();
            chunk[filled] = sum / channels as f32;
            filled += 1;
        }
        sink(&chunk[..filled]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything `deliver` hands the sink, and how many calls it took.
    fn run<T>(data: &[T], channels: usize) -> (Vec<f32>, usize)
    where
        T: Sample,
        f32: FromSample<T>,
    {
        let mut out = Vec::new();
        let mut calls = 0;
        deliver(data, channels, &mut |mono: &[f32]| {
            out.extend_from_slice(mono);
            calls += 1;
        });
        (out, calls)
    }

    #[test]
    fn deliver_averages_the_channels() {
        assert_eq!(
            run(&[1.0f32, 0.0, 0.5, 0.5, -1.0, 1.0], 2).0,
            [0.5, 0.5, 0.0]
        );
        assert_eq!(run(&[0.25f32, -0.25], 1).0, [0.25, -0.25]);
    }

    #[test]
    fn deliver_converts_integer_samples() {
        let first = [0i16, i16::MIN, 0, i16::MIN];
        assert_eq!(run(&first, 2).0, [-0.5, -0.5]);
        assert_eq!(run(&[0.5f32; 6], 3).0, [0.5, 0.5]);
    }

    #[test]
    fn a_partial_last_frame_is_still_one_sample_and_zero_channels_count_as_one() {
        assert_eq!(run(&[1.0f32, 1.0, 1.0], 2).0, [1.0, 0.5]);
        assert_eq!(run(&[0.5f32], 0).0, [0.5]);
    }

    #[test]
    fn a_big_callback_is_split_into_chunks_without_losing_or_moving_samples() {
        let data: Vec<f32> = (0..CHUNK_FRAMES * 2 + 7).map(|i| i as f32).collect();
        let (out, calls) = run(&data, 1);
        assert_eq!(out, data);
        assert_eq!(calls, 3);
        assert_eq!(run::<f32>(&[], 2), (Vec::new(), 0));
    }

    #[test]
    fn i16_and_u16_samples_convert_to_f32() {
        assert_eq!(0i16.to_sample::<f32>(), 0.0);
        assert_eq!(i16::MIN.to_sample::<f32>(), -1.0);
        assert!((i16::MAX.to_sample::<f32>() - 1.0).abs() < 1e-4);
        assert_eq!(32_768u16.to_sample::<f32>(), 0.0);
        assert_eq!(0u16.to_sample::<f32>(), -1.0);
        assert!((u16::MAX.to_sample::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_missing_named_device_is_reported_by_name() {
        match InputDevice::find(Some("no such microphone \u{1F3A4}")) {
            Err(Error::DeviceNotFound(name)) => assert!(name.contains("no such")),
            // A machine with no audio host at all fails differently; that is fine.
            Err(_) => {}
            Ok(_) => panic!("found a device that cannot exist"),
        }
    }
}
