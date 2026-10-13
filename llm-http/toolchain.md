# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| just | mise | Test recipe | https://github.com/casey/just |
| ketch | see its README | Installs swarfr | https://github.com/pyrlyn/ketch |
| swarfr | ketch | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| llm-wire | local (path) | https://github.com/pyrlyn/crates-packages | `ProviderError`, `ProviderEvent` and `Usage` |
| bytes | local, optional (`sse`) | https://github.com/tokio-rs/bytes | Chunk type of the SSE byte stream |
| eventsource-stream | local, optional (`sse`) | https://github.com/jpopesculian/eventsource-stream | SSE parser behind `sse_stream` |
| futures | local, optional (`sse`) | https://github.com/rust-lang/futures-rs | `Stream` and the in-memory `block_on` of `parse_sse_str` |
| http | local | https://github.com/hyperium/http | `HeaderValue` and `StatusCode` for the auth headers and `map_http_error`, with no HTTP client |
| keyring | local, optional (`keyring`) | https://github.com/open-source-cooperative/keyring-rs | Platform credential store lookup |
| reqwest | local, optional (`reqwest`) | https://github.com/seanmonstar/reqwest | Client builder of `client_with_timeout` (`rustls-tls` only) |
| serde_json | local | https://github.com/serde-rs/json | Reads the `error.message` envelope of an error body |
| tokio | local | https://github.com/tokio-rs/tokio | `mpsc`, `sleep`, `select!` and `join!` in the retry loop |
| tokio-util | local | https://github.com/tokio-rs/tokio | `CancellationToken` in the retry loop |
