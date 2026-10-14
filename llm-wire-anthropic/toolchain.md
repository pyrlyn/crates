# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| just | mise | Test recipe | https://github.com/casey/just |
| cargo-insta | cargo install | Review request and event snapshots | https://github.com/mitsuhiko/insta |
| ketch | see its README | Installs swarfr | https://github.com/pyrlyn/ketch |
| swarfr | ketch | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| llm-wire | local (path) | https://github.com/pyrlyn/crates | `Provider`, `Request`, `ProviderEvent`, `ProviderError` and `Usage` |
| llm-http | local (path) | https://github.com/pyrlyn/crates | Client builder, key lookup, auth headers, error mapping, retry and SSE framing |
| async-trait | local | https://github.com/dtolnay/async-trait | `Provider` is an async trait |
| futures | local | https://github.com/rust-lang/futures-rs | `StreamExt` over the SSE frame stream |
| reqwest | local | https://github.com/seanmonstar/reqwest | Sends the request; `json` for the body, `stream` for the response (`rustls-tls` only) |
| serde | local | https://github.com/serde-rs/serde | Derives in the generated wire types |
| serde_json | local | https://github.com/serde-rs/json | Request body as `Value`, frame parsing; `preserve_order` keeps the request bytes stable |
| tokio | local | https://github.com/tokio-rs/tokio | `mpsc` sink and `select!` in the stream loop |
| tokio-util | local | https://github.com/tokio-rs/tokio | `CancellationToken` |
| typify | local (build) | https://github.com/oxidecomputer/typify | `build.rs` generates the wire types from the OpenAPI spec |
| insta | local (dev) | https://github.com/mitsuhiko/insta | Snapshot tests of request bodies and event streams |
| wiremock | local (dev) | https://github.com/LukeMathWalker/wiremock-rs | Loopback HTTP server for the client tests |
