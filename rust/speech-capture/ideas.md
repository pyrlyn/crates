# Ideas

- One resampler instead of two: `resample_to_16k` (FFT) and `resample_mono` (sinc) differ only in quality and speed, but runa's stored PCM hashes depend on the sinc output.
- A streaming `Recorder` that hands out 16 kHz frames while recording, for live dictation.
