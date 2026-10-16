# speech-capture

Microphone capture, audio file decode, resampling to 16 kHz mono and an energy VAD for speech-to-text, with optional whisper.cpp transcription.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 2 | 0% | |
| T3 | todo | P2 | 2 | 0% | |

### T2. Adopt speech-capture in cox (needs publication)

Replace the `Recorder` in `crates/cox-voice/src/capture.rs` and its `Transcriber` in cox with this crate (feature `whisper` for the transcriber), mapping `speech_capture::Error` and `TranscribeError` to cox's `VoiceError` and `DictationError`. Blocked until the crate is published: it is `publish = false` today, so it must first be published (or the release flow set up for it).

### T3. Adopt speech-capture in runa (needs publication)

Replace `audio.rs` (decode, `resample_mono`, `pcm_sha256`, `pcm_to_wav_bytes`) and `energy_vad` from `asr.rs` in `crates/runa-media` with this crate. runa keeps `wav_base64`, the whisper model download, the ASR engine and the video code. Blocked until the crate is published, as in T2.
