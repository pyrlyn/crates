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
| async-trait | local | https://github.com/dtolnay/async-trait | `Provider` is an object-safe async trait |
| schemars | local | https://github.com/GREsau/schemars | JSON Schema derive on every wire type |
| serde | local | https://github.com/serde-rs/serde | Serialization of every wire type |
| serde_json | local | https://github.com/serde-rs/json | `Value` payloads and JSON round trips (`preserve_order` keeps cassette hashes stable) |
| thiserror | local | https://github.com/dtolnay/thiserror | `ProviderError` |
| tokio | local | https://github.com/tokio-rs/tokio | `mpsc::Sender` in the `Provider::stream` signature (`sync` feature only) |
| tokio-util | local | https://github.com/tokio-rs/tokio | `CancellationToken` in the `Provider::stream` signature |
| ulid | local | https://github.com/dylanhart/ulid-rs | `CallId` and `ArchiveId` |
| getrandom | local, wasm32-unknown-unknown only | https://github.com/rust-random/getrandom | `wasm_js` backend for ulid's random bits in a browser build |
| figment | local, optional (`test-util`) | https://github.com/SergioBenitez/Figment | Parse a scenario TOML file |
| sha2 | local, optional (`test-util`) | https://github.com/RustCrypto/hashes | Cassette hash key |
| pretty_assertions | local, dev | https://github.com/rust-pretty-assertions/rust-pretty-assertions | Readable test diffs |
| rstest | local, dev | https://github.com/la10736/rstest | Table-driven round-trip tests |
