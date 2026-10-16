# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| just | mise | Test recipe | https://github.com/casey/just |
| cmake | brew / system package | Builds whisper.cpp (feature `whisper` only) | https://github.com/Kitware/CMake |
| libasound2-dev | apt (Linux only) | ALSA headers for cpal | https://github.com/alsa-project/alsa-lib |
| ketch | see its README | Installs swarfr | https://github.com/pyrlyn/ketch |
| swarfr | ketch | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| cpal | local | https://github.com/RustAudio/cpal | Microphone capture from a named or the default input device (`InputDevice`, `Recorder`) |
| ringbuf | local | https://github.com/agerasev/ringbuf | Fixed-size lock-free ring buffer between the audio callback and the recorder |
| rubato | local | https://github.com/HEnquist/rubato | Resampling to 16 kHz |
| symphonia | local (feature `decode`) | https://github.com/pdeljanov/Symphonia | Decoding mp3, aac, flac, ogg, mp4 and wav files |
| hound | local (feature `decode`) | https://github.com/ruuda/hound | WAV read and write |
| sha2 | local (feature `decode`) | https://github.com/RustCrypto/hashes | SHA-256 of decoded PCM for the probe |
| serde | local (feature `decode`) | https://github.com/serde-rs/serde | `AudioProbe` serialization |
| thiserror | local | https://github.com/dtolnay/thiserror | Error enums |
| tracing | local | https://github.com/tokio-rs/tracing | Microphone stream errors from the audio callback |
| whisper-rs | local (feature `whisper`) | https://github.com/tazz4843/whisper-rs | whisper.cpp bindings for `Transcriber` |
| tempfile | local (dev) | https://github.com/Stebalien/tempfile | Scratch files in tests |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
