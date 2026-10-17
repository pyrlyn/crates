// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! Decode an audio file (wav, flac, ogg, mp3, aac, mp4) to mono `f32` at
//! 16 kHz with a printable probe of what was decoded. WAV goes through `hound`
//! first, everything else (and a WAV `hound` rejects) through `symphonia`.

use std::fs::File;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::Error;
use crate::resample::{TARGET_SAMPLE_RATE, resample_mono};

/// One decoded clip plus probe metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    /// Mono, [`TARGET_SAMPLE_RATE`] Hz, in `-1.0..1.0`.
    pub samples: Vec<f32>,
    pub probe: AudioProbe,
}

/// Stable, printable summary of a decode (no sample blob).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AudioProbe {
    pub path: String,
    pub codec: String,
    pub src_sample_rate: u32,
    pub src_channels: u16,
    pub duration_secs: f64,
    pub pcm_sample_rate: u32,
    pub pcm_samples: usize,
    pub pcm_sha256: String,
}

impl std::fmt::Display for AudioProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "file: {}", self.path)?;
        writeln!(f, "codec: {}", self.codec)?;
        writeln!(f, "src_rate: {}", self.src_sample_rate)?;
        writeln!(f, "src_channels: {}", self.src_channels)?;
        writeln!(f, "duration_s: {:.6}", self.duration_secs)?;
        writeln!(f, "pcm_rate: {}", self.pcm_sample_rate)?;
        writeln!(f, "pcm_samples: {}", self.pcm_samples)?;
        write!(f, "pcm_sha256: {}", self.pcm_sha256)
    }
}

/// Decode `path` (wav/flac/ogg/mp3/aac) to mono 16 kHz f32 PCM.
pub fn decode_audio(path: &Path) -> Result<DecodedAudio, Error> {
    let (interleaved, spec) = decode_interleaved_f32(path)?;
    if spec.channels == 0 {
        return Err(Error::Decode("zero channels".into()));
    }
    let mono = downmix_mono(&interleaved, spec.channels);
    let samples = if spec.rate == TARGET_SAMPLE_RATE {
        mono
    } else {
        resample_mono(&mono, spec.rate, TARGET_SAMPLE_RATE)?
    };
    let pcm_sha256 = pcm_sha256(&samples);
    let n_src_frames = interleaved.len() / spec.channels as usize;
    let duration_secs = if spec.rate == 0 {
        0.0
    } else {
        n_src_frames as f64 / f64::from(spec.rate)
    };
    Ok(DecodedAudio {
        probe: AudioProbe {
            path: path.display().to_string(),
            codec: spec.codec,
            src_sample_rate: spec.rate,
            src_channels: spec.channels,
            duration_secs,
            pcm_sample_rate: TARGET_SAMPLE_RATE,
            pcm_samples: samples.len(),
            pcm_sha256,
        },
        samples,
    })
}

/// SHA-256 of little-endian f32 bytes (stable across platforms).
pub fn pcm_sha256(samples: &[f32]) -> String {
    let mut hasher = Sha256::new();
    for s in samples {
        hasher.update(s.to_le_bytes());
    }
    hex(&hasher.finalize())
}

/// 16-bit mono WAV of already-resampled PCM (OpenAI `input_audio`).
pub fn pcm_to_wav_bytes(samples: &[f32], sample_rate: u32) -> Result<Vec<u8>, Error> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Vec::new();
    {
        let mut cursor = std::io::Cursor::new(&mut buf);
        let mut w =
            hound::WavWriter::new(&mut cursor, spec).map_err(|e| Error::Decode(e.to_string()))?;
        for s in samples {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            w.write_sample(v)
                .map_err(|e| Error::Decode(e.to_string()))?;
        }
        w.finalize().map_err(|e| Error::Decode(e.to_string()))?;
    }
    Ok(buf)
}

struct RawSpec {
    rate: u32,
    channels: u16,
    codec: String,
}

fn decode_interleaved_f32(path: &Path) -> Result<(Vec<f32>, RawSpec), Error> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "wav"
        && let Ok(v) = decode_wav_hound(path)
    {
        return Ok(v);
    }
    decode_symphonia(path)
}

fn decode_wav_hound(path: &Path) -> Result<(Vec<f32>, RawSpec), Error> {
    let reader = hound::WavReader::open(path).map_err(|e| Error::Decode(e.to_string()))?;
    let spec = reader.spec();
    let codec = format!(
        "pcm_{}_{}",
        match spec.sample_format {
            hound::SampleFormat::Int => "s",
            hound::SampleFormat::Float => "f",
        },
        spec.bits_per_sample
    );
    let channels = spec.channels;
    let rate = spec.sample_rate;
    let samples: Result<Vec<f32>, _> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .into_samples::<i32>()
            .map(|s| s.map(|v| int_sample_to_f32(v, spec.bits_per_sample)))
            .collect(),
        hound::SampleFormat::Float => reader.into_samples::<f32>().collect(),
    };
    let interleaved = samples.map_err(|e| Error::Decode(e.to_string()))?;
    Ok((
        interleaved,
        RawSpec {
            rate,
            channels,
            codec,
        },
    ))
}

fn int_sample_to_f32(s: i32, bits: u16) -> f32 {
    let bits = bits.clamp(1, 31);
    let max = (1u32 << (bits - 1)) as f32;
    (s as f32 / max).clamp(-1.0, 1.0)
}

fn decode_symphonia(path: &Path) -> Result<(Vec<f32>, RawSpec), Error> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error as SError;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::formats::TrackType;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| Error::Decode(e.to_string()))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| Error::NoAudio(path.display().to_string()))?
        .clone();
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|c| c.audio())
        .ok_or_else(|| Error::NoAudio(path.display().to_string()))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .map_err(|e| Error::Decode(e.to_string()))?;

    let mut pcm = Vec::new();
    let mut rate = audio_params.sample_rate.unwrap_or(0);
    let mut channels = audio_params
        .channels
        .as_ref()
        .map(|c| c.count() as u16)
        .unwrap_or(0);
    let codec = format!("{:?}", audio_params.codec);

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SError::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("end of stream") || msg.contains("eof") {
                    break;
                }
                return Err(Error::Decode(msg));
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                rate = decoded.spec().rate();
                channels = decoded.spec().channels().count() as u16;
                let start = pcm.len();
                pcm.resize(start + decoded.samples_interleaved(), 0.0);
                decoded.copy_to_slice_interleaved(&mut pcm[start..]);
            }
            Err(SError::DecodeError(_)) => continue,
            Err(e) => return Err(Error::Decode(e.to_string())),
        }
    }
    if pcm.is_empty() {
        return Err(Error::Decode(format!(
            "{}: no PCM frames decoded",
            path.display()
        )));
    }
    Ok((
        pcm,
        RawSpec {
            rate,
            channels,
            codec,
        },
    ))
}

fn downmix_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels as usize;
    if ch <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_wav(path: &Path, rate: u32, channels: u16, hz: f32) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..rate {
            let s = (i as f32 * hz * std::f32::consts::TAU / rate as f32).sin();
            for _ in 0..channels {
                w.write_sample((s * 16_000.0) as i16).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    #[test]
    fn tone_wavs_decode_to_stable_distinct_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let mut hashes = Vec::new();
        for (n, hz) in [220.0, 440.0, 880.0].into_iter().enumerate() {
            let path = dir.path().join(format!("clip-{n}.wav"));
            write_wav(&path, 16_000, 1, hz);
            let a = decode_audio(&path).unwrap();
            let b = decode_audio(&path).unwrap();
            assert_eq!(a.probe.pcm_sample_rate, TARGET_SAMPLE_RATE);
            assert_eq!(a.probe.src_channels, 1);
            assert_eq!(a.probe.src_sample_rate, 16_000);
            assert_eq!(a.samples.len(), 16_000, "1 s at 16 kHz");
            assert_eq!(a.probe.pcm_sha256, b.probe.pcm_sha256);
            assert_eq!(a.probe.pcm_sha256, pcm_sha256(&a.samples));
            assert_eq!(a.probe.pcm_sha256.len(), 64);
            hashes.push(a.probe.pcm_sha256);
        }
        hashes.sort();
        hashes.dedup();
        assert_eq!(hashes.len(), 3, "distinct tones give distinct PCM");
    }

    #[test]
    fn stereo_8k_resamples_to_mono_16k() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stereo8k.wav");
        write_wav(&path, 8_000, 2, 440.0);
        let decoded = decode_audio(&path).unwrap();
        assert_eq!(decoded.probe.src_channels, 2);
        assert_eq!(decoded.probe.src_sample_rate, 8_000);
        assert_eq!(decoded.probe.pcm_sample_rate, 16_000);
        // About 1 s at 16 kHz; the sinc padding may add a few samples.
        assert!(
            decoded.samples.len() > 14_000 && decoded.samples.len() < 18_000,
            "got {} samples",
            decoded.samples.len()
        );
    }

    #[test]
    fn a_wav_without_the_wav_extension_decodes_through_symphonia() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.dat");
        write_wav(&path, 16_000, 1, 440.0);
        let decoded = decode_audio(&path).unwrap();
        assert_eq!(decoded.samples.len(), 16_000);
        assert_eq!(decoded.probe.src_sample_rate, 16_000);
        // hound names a WAV codec `pcm_s_16`; symphonia's name differs, which
        // proves the second decoder ran.
        assert!(
            !decoded.probe.codec.starts_with("pcm_s_"),
            "{}",
            decoded.probe.codec
        );
    }

    #[test]
    fn missing_file_errors() {
        let err = decode_audio(Path::new("/no/such/clip.wav")).unwrap_err();
        assert!(err.to_string().contains("io") || err.to_string().contains("No such"));
    }

    #[test]
    fn a_file_that_is_not_audio_is_a_decode_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, b"not audio at all").unwrap();
        assert!(matches!(decode_audio(&path), Err(Error::Decode(_))));
    }

    #[test]
    fn pcm_to_wav_bytes_round_trips_through_the_decoder() {
        let samples: Vec<f32> = (0..1600).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let wav = pcm_to_wav_bytes(&samples, 16_000).unwrap();
        assert!(wav.starts_with(b"RIFF"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.wav");
        std::fs::write(&path, &wav).unwrap();
        let back = decode_audio(&path).unwrap();
        assert_eq!(back.samples.len(), samples.len());
        assert!(
            back.samples
                .iter()
                .zip(&samples)
                .all(|(a, b)| (a - b).abs() < 1e-3)
        );
    }
}
