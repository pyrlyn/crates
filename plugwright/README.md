# plugwright

Dynamic plugin manager for Rust: builds plugin crates (path or git) into shared or static
libraries, caches the artifacts, loads them natively through `libloading` (in a sandboxed child
process by default, or in-process) or as Wasm on a pluggable engine (wasmtime, wasmi, wasmer, …),
behind one `Plugin` trait.

Library docs, manifest format and security model: [crates/plugwright/README.md](crates/plugwright/README.md).
Design: [design.md](design.md). Tasks: [plan.md](plan.md), finished in [done.md](done.md).
Research and sources: [research.md](research.md). Tools and crates: [toolchain.md](toolchain.md).

## Layout

| path | what |
|---|---|
| `crates/plugwright-abi` | `#![no_std]` binary contract: native decl table + core-Wasm ABI |
| `crates/plugwright-sdk` | plugin side: `#[plugin]`, `trait Plugin`, `export_plugin!` (native and Wasm from one source), `log` |
| `crates/plugwright-macros` | proc macros: `#[plugin]` (re-exported by the SDK) and `#[interface]` (by `plugwright`), one wire convention |
| `crates/plugwright-core` | host-side `Plugin` trait, `PluginInfo`, `Release`, errors, logging, `Worker` (thread per plugin, two lanes, deadlines) |
| `crates/plugwright-packages` | installed packages on disk: digest, versioned staging with rollback, dev links, discovery |
| `crates/plugwright-build` | cargo builder, content-addressed cache, git sources, static-library relink, build acceleration |
| `crates/plugwright-native` | in-process loader, unload verification, artifact format detection |
| `crates/plugwright-sandbox` | OS sandbox the runner applies to itself (Seatbelt, Landlock + seccomp, low integrity + job) |
| `crates/plugwright-isolate` | child-process isolation: protocol, runner, restarts, timeouts |
| `crates/plugwright-wasm` | Wasm engine traits and the `WasmPlugin` glue |
| `crates/plugwright-wasmtime`, `-wasmi` | engine adapters |
| `crates/plugwright-extism` | extism modules with host functions and opt-in WASI, on a `Worker` |
| `crates/plugwright-wasmer` | wasmer adapter — **outside the workspace**, own `Cargo.lock` |
| `crates/plugwright` | facade: manifest, lockfile, `PluginManager`, re-exports |
| `crates/plugwright-runner` | standalone runner for isolated plugins (when the host cannot re-exec itself) |
| `crates/cargo-plugwright` | `cargo plugwright build / list / call / methods / gc / doctor / lib / wasm / plugin` |
| `examples/native-hello` | native plugin: `greet`, `add`, `panic`, `crash`, `abort`, `sleep`, `print`, sandbox probes |
| `examples/static-hello` | the same contract as a `staticlib`, relinked by the host |
| `examples/wasm-hello` | Wasm plugin: `greet`, `reverse`, `loop`, `oom`, `trap` |
| `examples/host` | `plugwright-host` demo + end-to-end tests |

## Quick start

Requires rustup (`rust-toolchain.toml` pins Rust 1.97.1 with `wasm32-unknown-unknown`),
[mise](https://mise.jdx.dev) for the tasks below, `git` and a C compiler driver (`cc`) for
relinking static libraries.

```bash
mise run demo
```

The demo builds the example plugins through plugwright, loads them in-process, isolated, on
wasmtime and on wasmi, then makes them panic, segfault, abort, hang, loop, run out of memory and
trap — and the host prints `survived everything`. A second run is all cache hits.

```bash
cargo install --path crates/cargo-plugwright
```

```bash
cd examples && cargo plugwright doctor
```

```bash
cd examples && cargo plugwright call hello-isolated greet world
```

`lib`, `wasm` and `plugin` need no manifest. They take a crate of the cargo workspace around the
current directory by name, build it through the plugwright cache in `<target-dir>/plugwright` and
copy the result into `--out-dir` (default `./dist`). `lib` overrides the crate's own `crate-type`,
so any library crate comes out as a shared or a static library; `plugin` keeps it, relinks a
static library and loads the result once to check it.

```bash
cargo plugwright lib my-crate             # libmy_crate.so / .dylib / my_crate.dll
cargo plugwright lib my-crate --static    # libmy_crate.a / my_crate.lib
cargo plugwright wasm my-crate            # my_crate.wasm
cargo plugwright plugin my-plugin --wasm  # built, loaded, checked
```

## Without a manifest

The crate `plugwright-loader` (re-exported by `plugwright`) compiles and opens plugins with only
the checks that keep the host sound: the file format from magic bytes, the ABI version, the
declaration table and the SDK version. It skips hashing, copying, the lockfile, the runner process
and the sandbox. It has three loaders:

* `NativeLoader` builds and maps cdylib and staticlib plugins and returns a `NativePlugin`.
* `WasmLoader` builds, compiles and runs core Wasm modules and returns a `WasmPlugin`. It has
  `with_engine` and `with_limits`.
* `Loader` puts both over one cache. `open` picks the loader by magic bytes and returns
  `Box<dyn Plugin>`.

Without default features the crate loads only native code and pulls in no Wasm crates (54
dependencies instead of 151). The feature `wasm` adds `WasmLoader` without an engine, and
`wasmtime` and `wasmi` add the engines.

```rust
let loader = plugwright::Loader::new(".plugwright")?;
let dynamic = loader.open("plugins/libhello.so")?; // mapped in place
let stat = loader.open("plugins/libhello.a")?;     // relinked once, then cached
let wasm = loader.open("plugins/hello.wasm")?;     // compiled once, then cached
let built = loader.load("plugins/hello")?;         // cargo build (cdylib or staticlib), cached
let module = loader.load_wasm("plugins/hello")?;   // cargo build for wasm32, cached

let native = plugwright::NativeLoader::new(".plugwright")?.load("plugins/hello")?; // NativePlugin
```

The cache key reads manifests instead of running `cargo metadata`, `rustc -vV` is remembered on
disk, and a cached artifact is not rehashed while its stat stays the same, so a warm cache serves
a fresh process in milliseconds. The first-ever `dlopen` of a new file costs ~90 ms of macOS
code-signature validation, once per file. The numbers are under [Performance](#performance).

A library is mapped in place, so you must not rewrite it while it is loaded. Each live instance
needs an image of its own, so a second load of a still-mapped file maps a copy from a reusable
pool (`<tag>.<ext>`, `<tag>-<n>.<ext>`). `PluginManager::gc_cache` prunes unmapped copies. The
code runs in your process, so use `Loader` for trusted code only.

Wasm modules are compiled once per engine and content. Plugins loaded from the same bytes in one
process share the compiled module, and wasmtime keeps the machine code in
`compiled/wasmtime/<sha256>-<engine hash>.cwasm` for the next process. wasmi has nothing to keep
on disk. Stored code is checked against its sha256 before
use, so a damaged file is recompiled. The check does not stop someone who can write to the cache
directory, because they can run code in the host. `gc_cache` drops compiled code of modules the
lockfile no longer names.

## Performance

```bash
mise run perf
```

`examples/host/benches/perf.rs` builds the example plugins and then measures two kinds of case.
Hot cases repeat in one process. Fresh cases each start a new process with a warm cache and time
the load inside it, so spawning the process is not counted. The run fails when a median exceeds
its budget, set at about ten times the medians below. `PLUGWRIGHT_PERF_SCALE=3` widens every
budget for a slow or busy machine, and `-- <substring>` runs only the matching cases. Each run
prints its change against the previous one, which is kept in `target/tmp/perf-last.json`.

Medians with a release host; `wasm-hello` is 130 KB. Linux runs in a colima VM on the same Mac.

| Case | macOS | Linux aarch64 |
|---|---|---|
| call, native in-process | 0.11 µs | 0.23 µs |
| call, native in an isolated process | 9 µs | 45 µs |
| call, wasmtime / wasmi | 1.3 / 2.4 µs | 1.2 / 2.3 µs |
| cache hit for an unchanged crate (key + lookup) | 1.5 ms | 0.3 ms |
| Wasm open while the module is in use (instantiate only) | 0.13 ms | 0.08 ms |
| Wasm open with a new loader, then drop everything | 0.6 ms | 0.6 ms |
| fresh process: open `.so` / `.wasm` on wasmtime / `.wasm` on wasmi | 0.9 / 0.8 / 2.1 ms | 0.08 / 0.8 / 2.0 ms |
| fresh process: `load` of a native / static / Wasm crate | 2.6 / 3.5 / 2.7 ms | 0.5 / 0.6 / 1.3 ms |
| fresh process: `PluginManager` load, native / Wasm | 2.9 / 2.8 ms | 0.55 / 1.3 ms |

On aarch64 Linux, the first code wasmtime publishes in a process registers a `membarrier`. That
is cheap while the process has one thread and costs an RCU grace period (~10 ms) once other
threads exist. The wasmtime epoch thread therefore starts at the first instantiation, not when
the engine is created. A host that already runs threads of its own pays the ~10 ms once per
process.

## Checks

```bash
mise run check
```

```bash
mise run test
```

```bash
mise run test-wasmer
```

`test-wasmer` builds wasmer with Cranelift (~2.3 GiB in its own `target/`). The e2e test
`cranelift_is_used_for_isolated_plugins_only` runs only when a nightly toolchain with
`rustc-codegen-cranelift-preview` is installed.

## License

MIT OR Apache-2.0.
