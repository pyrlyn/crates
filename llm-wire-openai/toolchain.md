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
| llm-http | local (path) | https://github.com/pyrlyn/crates | HTTP client, auth header, error mapping, retry and SSE framing |
| llm-wire | local (path) | https://github.com/pyrlyn/crates | `Provider`, `Request`, `ProviderEvent` and `ProviderError` |
| async-openai | local | https://github.com/64bit/async-openai | Typed Responses request and stream-event shapes (`response-types` only) |
| async-trait | local | https://github.com/dtolnay/async-trait | `Provider` is an async trait |
| futures | local | https://github.com/rust-lang/futures-rs | `StreamExt` over the SSE frame stream |
| reqwest | local | https://github.com/seanmonstar/reqwest | Sends the request (`rustls-tls`, `json`, `stream`) |
| serde | local | https://github.com/serde-rs/serde | `DeserializeOwned` bound when typing a stream event |
| serde_json | local | https://github.com/serde-rs/json | Request bodies and the untyped parts of stream frames |
| tokio | local | https://github.com/tokio-rs/tokio | `mpsc` sink and `select!` in the stream loop |
| tokio-util | local | https://github.com/tokio-rs/tokio | `CancellationToken` |
| insta | local (dev) | https://github.com/mitsuhiko/insta | Snapshots of request bodies and event streams |
| wiremock | local (dev) | https://github.com/LukeMathWalker/wiremock-rs | Local HTTP server for the client tests |
