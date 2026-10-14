// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! The one error type for capture, decode and resampling. Transcription has
//! its own (`TranscribeError`) because it exists only behind the `whisper`
//! feature.

/// What can go wrong between a sound source and 16 kHz mono samples.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The host has no default input device.
    #[error(
        "no microphone found; check that one is connected and that this program may use it (macOS: System Settings > Privacy & Security > Microphone)"
    )]
    NoInputDevice,
    /// No input device has the requested name.
    #[error("no input device named {0:?}")]
    DeviceNotFound(String),
    /// The input stream could not be opened, started or run.
    #[error(
        "microphone: {0}; if the OS denied access, allow this program to use the microphone (macOS: System Settings > Privacy & Security > Microphone)"
    )]
    Stream(String),
    /// The host has no default output device.
    #[error("no speaker found; check that an output device is connected and selected")]
    NoOutputDevice,
    /// No output device has the requested name.
    #[error("no output device named {0:?}")]
    OutputDeviceNotFound(String),
    /// The output stream could not be opened, started or run.
    #[error("speaker: {0}")]
    Output(String),
    /// An audio file could not be opened or read.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// An audio file is damaged or in a format the decoders do not handle.
    #[error("decode: {0}")]
    Decode(String),
    /// A sample-rate conversion failed.
    #[error("resample: {0}")]
    Resample(String),
    /// A media file holds no audio track.
    #[error("no audio track in {0}")]
    NoAudio(String),
}
