# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc, cargo | rustup (the root `rust-toolchain.toml` of `pyrlyn/crates`, plus `wasm32-unknown-unknown`) | Build; plugwright also runs cargo to build plugins | https://github.com/rust-lang/rust |
| mise | see its docs | Task runner (`mise run check/test/demo/perf/...`) | https://github.com/jdx/mise |
| git | system | Git plugin sources are fetched with the `git` CLI | https://git-scm.com |
| C compiler driver (cc/clang) or `link.exe` | system / Xcode CLT / Visual Studio | Relinking static-library plugins | https://github.com/rust-lang/cc-rs |
| sccache | optional, mise | Build acceleration (`sccache = "auto"`) | https://github.com/mozilla/sccache |
| mold | optional, mise (Linux) | Build acceleration (`linker = "auto"`) | https://github.com/rui314/mold |
| rustc_codegen_cranelift | optional, `rustup component add rustc-codegen-cranelift-preview` on nightly | Faster dev builds of isolated plugins (`codegen = "cranelift"`) | https://github.com/rust-lang/rustc_codegen_cranelift |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| cargo_metadata | local | https://github.com/oli-obk/cargo_metadata | Finds a workspace crate by name for `cargo plugwright lib / wasm / plugin` |
| fs-kit | local | https://github.com/pyrlyn/crates/tree/main/rust/fs-kit | Canonical source paths without the Windows `\\?\` prefix (plugwright-build) |
| cc | local | https://github.com/rust-lang/cc-rs | Finds the linker for static-library relinking |
| libc | local | https://github.com/rust-lang/libc | dlopen probes, rlimits, Seatbelt and Landlock/seccomp calls, POSIX shared memory for isolated calls |
| libloading | local | https://github.com/nagisa/rust_libloading | Loads native plugin libraries |
| notify | local | https://github.com/notify-rs/notify | Opt-in watcher that reloads plugins when their files change |
| minisign-verify | local | https://github.com/jedisct1/rust-minisign-verify | Verifies artifact signatures before loading (zero dependencies) |
| minisign | local (optional, feature `sign` of `plugwright-trust`) | https://github.com/jedisct1/rust-minisign | Signs artifacts and generates keys |
| proc-macro2 | local | https://github.com/dtolnay/proc-macro2 | `#[plugin]` / `#[interface]` macros |
| quote | local | https://github.com/dtolnay/quote | `#[plugin]` / `#[interface]` macros |
| syn | local | https://github.com/dtolnay/syn | `#[plugin]` / `#[interface]` macros |
| semver | local | https://github.com/dtolnay/semver | ABI, SDK and plugin version checks |
| serde | local | https://github.com/serde-rs/serde | Manifest, lockfile and cache metadata |
| serde_json | local | https://github.com/serde-rs/json | JSON calls and cargo's JSON messages |
| sha2 | local | https://github.com/RustCrypto/hashes | Cache keys and artifact integrity |
| thiserror | local | https://github.com/dtolnay/thiserror | `PluginError`, `WasmError` |
| toml | local | https://github.com/toml-rs/toml | Manifests and Cargo.toml fingerprinting without `cargo metadata` |
| tracing | local | https://github.com/tokio-rs/tracing | Spans and events for build, load, call |
| tracing-subscriber | local | https://github.com/tokio-rs/tracing | Logging in the demo host and the CLI |
| wasmtime | local | https://github.com/bytecodealliance/wasmtime | Wasm engine (feature `wasmtime`) |
| wasmi, wasmi_core | local | https://github.com/wasmi-labs/wasmi | Wasm interpreter (feature `wasmi`) |
| extism | local (`plugwright-extism`) | https://github.com/extism/extism | Runs extism-ABI plugins with host functions (exceptions proposal on) |
| wasmer, wasmer-middlewares | local (`crates/plugwright-wasmer`, own lockfile) | https://github.com/wasmerio/wasmer | Wasm engine adapter outside the workspace |
| windows-sys | local | https://github.com/microsoft/windows-rs | Job objects, integrity levels, module probes, shared memory sections on Windows |
| tempfile | local (dev) | https://github.com/Stebalien/tempfile | Test directories |
| trybuild | local (dev) | https://github.com/dtolnay/trybuild | UI tests of `#[plugin]` / `#[interface]` diagnostics |
