# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The audio front end for speech-to-text, shared by cox, runa and aulo: the default
microphone (`Recorder`, and `InputDevice` for a streaming sink), the default or a named speaker (`OutputDevice`, for a streaming source), audio file decode (`decode_audio`), resampling to 16 kHz
mono, an energy VAD (`energy_vad`) and, behind the `whisper` cargo feature
(off by default), a whisper.cpp `Transcriber`. Audio stays in memory; nothing
here opens a socket or writes a file.

Audio callbacks never block or allocate, and audio buffers are bounded. `src/input.rs`
and `src/output.rs` are the only places that open a `cpal` stream (`InputDevice::find`,
`OutputDevice::find`, `start`, the `InputStream` and `OutputStream` guards), both on the
stream thread in `src/stream.rs`; `Recorder` is built on `input.rs`. The output callback,
`render`, asks the caller's source for mono samples in a stack buffer and writes them to
every channel, so that source must be wait-free too. Its callback, `deliver`, converts and
downmixes into a stack buffer and calls the caller's sink, so the sink must be
wait-free too. `Recorder`'s sink is `feed` in `src/capture.rs`: it pushes into the
producer half of a fixed-size `ringbuf` ring that `start` allocates before the stream
starts, and counts the samples a full ring drops in an `AtomicU64`
(`Recorder::overflowed_samples`). Keep it that way: no `Mutex`, no `Vec`, no
logging and no `?` that allocates in a callback or a sink, and a test for any change.

The `decode` feature (default) gates file decode and its dependencies; a caller that
only captures can turn default features off.

Tests must not need a microphone or a model: device-free steps are pure
functions, and the model test is `#[ignore]`d.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo test --no-default-features   # capture and resample only
cargo test --features whisper   # builds whisper.cpp: needs cmake and a C++ compiler
cargo fmt
```

On Linux, `cpal` needs the ALSA headers (`libasound2-dev`).

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
