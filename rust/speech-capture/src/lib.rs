// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! Everything between a sound source and a speech-to-text engine: the default
//! microphone (`Recorder`, or `InputDevice` for a streaming sink), the default
//! or a named speaker (`OutputDevice`, for a streaming source), audio files
//! (`decode_audio`, feature `decode`, on by default), resampling to 16 kHz
//! mono (`resample_to_16k`, `resample_mono`) and an energy voice-activity
//! detector (`energy_vad`). The optional `whisper` feature adds `Transcriber`,
//! whisper.cpp over a 16 kHz mono buffer. Audio stays in memory: nothing here
//! opens a socket or writes a file.

// Unit tests assert with `expect`/`unwrap`; the deny list is for shipped code.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod capture;
#[cfg(feature = "decode")]
mod decode;
mod error;
mod input;
mod output;
mod resample;
mod stream;
#[cfg(feature = "whisper")]
mod transcribe;
mod vad;

pub use capture::Recorder;
#[cfg(feature = "decode")]
pub use decode::{AudioProbe, DecodedAudio, decode_audio, pcm_sha256, pcm_to_wav_bytes};
pub use error::Error;
pub use input::{InputDevice, InputStream, input_device};
pub use output::{OutputDevice, OutputStream};
pub use resample::{TARGET_SAMPLE_RATE, resample_mono, resample_to_16k};
#[cfg(feature = "whisper")]
pub use transcribe::{TranscribeError, Transcriber};
pub use vad::{VadSpan, energy_vad};
