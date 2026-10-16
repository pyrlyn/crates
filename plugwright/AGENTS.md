# AGENTS.md

If an AGENTS.md or CLAUDE.md exists higher in the tree, follow it too; on conflict, ask the creator.

Rust plugin manager, a separate workspace inside `pyrlyn/crates` (excluded from the root one:
own `Cargo.lock`, `rust-toolchain.toml` (1.97.1) and `mise.toml` tasks; run commands from this
directory). CI: the `plugwright` job in the root `.github/workflows/ci.yml`.
Read [design.md](design.md) (architecture, decisions D1–D26, security model) before changing
design; [research.md](research.md) records why alternatives were rejected. Tasks live in
[plan.md](plan.md) / [todo.md](todo.md), finished ones in [done.md](done.md).

## Commands

* `mise run check` — `cargo fmt --check` + `clippy -D warnings` for the workspace.
* `mise run test` — unit tests + `examples/host/tests/e2e.rs` (real cargo builds; first run is slow).
* `mise run demo` — end-to-end demo through `examples/plugwright.toml`.
* `mise run test-wasmer` — the wasmer adapter, which has its own lockfile.
* `mise run perf` — performance tests with budgets (`examples/host/benches/perf.rs`, release host).
  Run them after touching a load, cache or call path; a new hot path gets a case.
* `mise run doctor` — `cargo plugwright doctor` on the demo manifest.
* Wasm side: `cargo clippy -p plugwright-sdk -p wasm-hello --target wasm32-unknown-unknown -- -D warnings`.
* Cross lint: `--target x86_64-pc-windows-msvc` for the workspace minus `wasm-hello`,
  `plugwright-wasmtime`, `plugwright-loader`, `plugwright`, `plugwright-host`, `cargo-plugwright`;
  `--target aarch64-unknown-linux-gnu` for the crates without wasmtime (no cross C compiler here).

## Layout

`plugwright-macros` (proc macros, re-exported by `-sdk` and the facade).
`plugwright-core` ← `-sandbox`, `-trust`, `-build`, `-native`, `-wasm` ← `-isolate`, `-wasmtime`, `-wasmi`,
`-wasmer`; `-build`, `-native`, `-wasm`, `-wasmtime`, `-wasmi` ← `plugwright-loader`
(`NativeLoader`, `WasmLoader` behind feature `wasm`, `Loader` over both; no manifest) ← `plugwright`
(facade: manifest + manager, re-exports the loader) ← `cargo-plugwright`. Check
`plugwright-loader` with no features, `wasm`, `wasmi` and `wasmtime`: native-only must not pull Wasm
crates. Put code in the lowest
crate that can own it; the facade only wires layers together.

## Rules

* Any change to a layout or signature in `plugwright-abi` bumps `ABI_VERSION`.
* Never let a panic cross `extern "C"`; host callbacks wrap their bodies in `catch_unwind`.
* Plugin failures are `PluginError` values, never host panics. Values from a plugin (lengths,
  statuses, strings) are validated before use.
* `crates/plugwright-wasmer` stays out of the workspace: wasmer 7.5 and wasmtime 49 pin
  incompatible `cranelift-codegen` versions.
* Disk is tight on the dev machine: profiles use `debug = 0`; run `cargo clean` in
  `crates/plugwright-wasmer` after testing it.
* Artifact kinds come from magic bytes (`plugwright_native::format`), never from file extensions.
* Anything that can change build output belongs in the cache key; sccache does not.
* Never build an in-process plugin with Cranelift: its code aborts on panic.
* `#[plugin]` and `#[interface]` share `plugwright-macros/src/wire.rs`; change the wire
  convention there and in both `__private` runtimes together. Generated code uses absolute paths
  (`::core::…`, `#krate::__private`) and must stay clippy-clean in user crates.
* Every live native instance needs its own image (the SDK keeps per-image globals): map native
  libraries only through `NativePlugin::load`, which picks the file under the `MAPPING` lock.
* Paths handed to a subprocess that runs in another directory must be absolute.
* Docs and code in English, `research.md` included.
