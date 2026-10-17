# Ideas

- aulo's `aulo-speech-cloud` frames SSE with `sse-core`, cox with `eventsource-stream`. One backend for both would drop a dependency; `sse-core` is zero-I/O and could replace `eventsource-stream` here if its behaviour matches on the existing tests.
