# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| just | mise | Test recipe | https://github.com/casey/just |
| the package manager | see its README | Installs swarfr | https://github.com/pyrlyn |
| swarfr | the package manager | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| llm-wire | local (path) | https://github.com/pyrlyn/crates | The `Provider` trait and the request/event types the loop drives |
| async-trait | local | https://github.com/dtolnay/async-trait | The seams are object-safe async traits |
| serde | local | https://github.com/serde-rs/serde | `Submission`/`Event` keep the coding agent's serde shape |
| serde_json | local | https://github.com/serde-rs/json | Tool inputs are `Value`s |
| thiserror | local | https://github.com/dtolnay/thiserror | `LoopError` |
| tokio | local | https://github.com/tokio-rs/tokio | Channels, `JoinSet` for parallel calls, `select!`, approval timeout |
| tokio-util | local | https://github.com/tokio-rs/tokio | `CancellationToken` for interrupts |
| llm-wire (`test-util`) | local, dev | https://github.com/pyrlyn/crates | Scenario parsing for the scripted test provider |
| pretty_assertions | local, dev | https://github.com/rust-pretty-assertions/rust-pretty-assertions | Readable test diffs |
