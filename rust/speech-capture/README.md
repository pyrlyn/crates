# speech-capture

Everything between a sound source and a speech-to-text engine.

- `Recorder::start(max)` records the default microphone into memory (capped at
  `max`); `stop()` returns mono `f32` at 16 kHz. `input_device()` names the
  default device without opening a stream.
- `InputDevice::find(name)` finds an input device by name (`None`: the default);
  `start(sink)` streams mono `f32` at `sample_rate_hz()` into a real-time sink
  and returns an `InputStream` that stops the stream when dropped. `Recorder`
  is built on it.
- `OutputDevice::find(name)` is the same for a speaker: `start(source)` calls a
  real-time source that fills mono `f32` at `sample_rate_hz()`, plays it on every
  channel and returns an `OutputStream` that stops the stream when dropped.
- `decode_audio(path)` reads wav, flac, ogg, mp3, aac or mp4 and returns mono
  16 kHz samples with an `AudioProbe` (codec, source rate, duration, SHA-256 of
  the PCM).
- `resample_to_16k` (FFT) and `resample_mono` (sinc) convert a mono clip.
- `energy_vad(samples, rate)` finds the speech spans of a clip.
- Feature `decode` (on by default) holds `decode_audio` and its helpers; turn
  default features off to capture without symphonia, hound, sha2 and serde.
- Feature `whisper` (off by default): `Transcriber::load(model)` and
  `transcribe(pcm, language)` over whisper.cpp.

```rust
use speech_capture::{TARGET_SAMPLE_RATE, energy_vad, resample_to_16k};

let tone: Vec<f32> = (0..8_000).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
let pcm = resample_to_16k(&tone, 8_000)?;
assert!(!energy_vad(&pcm, TARGET_SAMPLE_RATE).is_empty());
# Ok::<(), speech_capture::Error>(())
```

Merged from cox's `cox-voice` capture and runa's `runa-media` decode, resample
and VAD. On Linux, building
needs the ALSA headers (`libasound2-dev`).
