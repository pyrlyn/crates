# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc 1.99.0 | rustup (`rust-toolchain.toml`) | Build | https://github.com/rust-lang/rust |
| rustc 1.86 | rustup | MSRV check (`just msrv`) | https://github.com/rust-lang/rust |
| cargo-nextest | global | Tests (`just test`) | https://github.com/nextest-rs/nextest |
| just | global | Task runner | https://github.com/casey/just |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| syn | local | https://github.com/dtolnay/syn | Parse macro input (`full`: `impl` blocks, closures) |
| quote | local | https://github.com/dtolnay/quote | Emit generated code |
| proc-macro2 | local | https://github.com/dtolnay/proc-macro2 | Spans and token streams in the macros |
| tracing | local (optional) | https://github.com/tokio-rs/tracing | `TracingHooks` behind the `tracing` feature |
| trybuild | local (dev) | https://github.com/dtolnay/trybuild | Compile-fail tests with pinned messages |
| divan | local (bench) | https://github.com/nvzqz/divan | Comparison benchmarks |
| criterion | local (bench) | https://github.com/bheisler/criterion.rs | Criterion suite: cold start, lazy vs cached, deep chains, threads, scopes, overrides |
| shaku, nject, teloc, dill | local (bench) | crates.io | Competitors in the comparison benchmarks only |
