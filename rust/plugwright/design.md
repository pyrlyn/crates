# plugwright — design

Rust library that **builds, caches, loads, isolates and unloads plugins**: native `cdylib`
(`.so`/`.dylib`/`.dll`) through `libloading`, and Wasm modules through a pluggable engine layer
(wasmtime, wasmi, wasmer, or a third-party engine). Research and sources: [research.md](research.md).
Tasks: [plan.md](plan.md), finished ones (T01–T41) in [done.md](done.md); F1–F9 in [ideas.md](ideas.md).

Crate name `plugwright` and every `plugwright-*` sub-crate name were free on crates.io on 2026-10-14.

## 1. Architecture

```
            plugwright.toml ──► Manifest ──► Lockfile (plugwright.lock: git SHAs, cache keys, sha256)
                                   │
                                   ▼
 ┌──────────────────────── PluginManager ─────────────────────────┐
 │ load / unload / reload / call / call_json / list               │
 │                                                                │
 │  Source resolver ──► Builder (cargo subprocess) ──► Cache       │
 │  path | git | artifact   --message-format=json   content-addr. │
 │                                                                │
 │  Backend selection → Box<dyn Plugin>                           │
 │   ├─ NativePlugin      (in-process, libloading, catch_unwind)  │
 │   ├─ IsolatedPlugin    (child process runner, crash-proof)     │
 │   └─ WasmPlugin<E>     (any WasmEngine: fuel/memory/timeout)   │
 └────────────────────────────────────────────────────────────────┘
        tracing spans/events for build, cache, load, call, unload, crash
```

Workspace:

| crate | role |
|---|---|
| `plugwright-abi` | `#![no_std]`, zero deps: `#[repr(C)]` native decl table + core-Wasm ABI constants |
| `plugwright-sdk` | plugin side: `trait Plugin` + `export_plugin!` emitting the native **or** Wasm ABI from the same source |
| `plugwright-core` | host-side `Plugin` trait, `PluginInfo`, `Release`, `PluginError`/`WasmError`, logging sink, `Worker` (a thread per plugin: control and event lanes, deadline interrupts) |
| `plugwright-packages` | installed packages on disk: `package_digest`, `versions/<digest12>/` staging, `current` / `previous` / `link`, guarded `remove`, discovery over the application's `Manifest` |
| `plugwright-build` | cargo builder (with a `crate-type` override), content-addressed cache, git sources, static-library relink, build acceleration (`accel`), workspace package lookup (`package`) |
| `plugwright-native` | in-process loader, unload probe, artifact format detection (`format`) |
| `plugwright-sandbox` | runner self-sandbox: Seatbelt / Landlock + seccomp / low integrity + job object |
| `plugwright-trust` | detached minisign signatures for artifacts, `TrustPolicy` (keys, unsigned = deny / warn / allow); signing behind feature `sign` |
| `plugwright-isolate` | child-process isolation: protocol, runner, restart budget, timeouts |
| `plugwright-wasm` | engine traits `WasmEngine → WasmModule → WasmInstance` + `WasmPlugin` ABI glue, `ModuleCache` (compiled modules in memory and on disk) |
| `plugwright-wasmtime`, `plugwright-wasmi` | engine adapters (features `wasmtime`, `wasmi` of the loader and the facade) |
| `plugwright-extism` | extism-ABI modules as a `Plugin`: application host functions (`HostEnv`), memory cap, call budget, cancel at the deadline, opt-in WASI preview 1 (D27) |
| `plugwright-loader` | in-process loading without a manifest: `NativeLoader` (cdylib, staticlib → `NativePlugin`), `WasmLoader` (feature `wasm`: engine, `ModuleCache`, limits → `WasmPlugin`), `Loader` over both (dispatch by magic bytes); `builtin_engines` |
| `plugwright-wasmer` | wasmer adapter; **outside the workspace** with its own lockfile (see T13) |
| `plugwright` | facade: manifest, lockfile, `PluginManager` on top of `plugwright-loader`, re-exports of every layer |
| `plugwright-runner` | tiny binary: out-of-process runner for hosts that cannot re-exec themselves |
| `cargo-plugwright` | `cargo plugwright build / list / call / methods / gc / doctor`; `lib / wasm / plugin` build a named workspace crate without a manifest |
| `examples/native-hello` | native plugin (`greet`, `add`, `panic`, `crash`, `abort`, `sleep`, `print`, sandbox probes) |
| `examples/static-hello` | the same contract as a `staticlib`, relinked by the host |
| `examples/wasm-hello` | Wasm plugin (methods: `greet`, `reverse`, `loop`, `oom`, `trap`) |
| `examples/host` | `plugwright-host` demo CLI; e2e tests in `examples/host/tests/e2e.rs` |

Crate graph: `core ← {sandbox, trust, build, native, wasm, extism} ← {isolate, wasmtime, wasmi, wasmer} ← plugwright ← cargo-plugwright`.

## 2. Decisions

| # | Decision | Why |
|---|---|---|
| D1 | Native ABI = `#[repr(C)]` decl table (`plugwright_entry_v1`) with `Option<extern "C" fn>` slots, opaque byte payloads | Rust has no stable ABI; C ABI lets host/plugin use different rustc versions. `abi_stable`/`stabby` were rejected: heavier, Rust-only, and do not help Wasm |
| D2 | One guest SDK, two targets: `export_plugin!` emits the native table or Wasm exports by `cfg(target_arch)` | one plugin source → `.so` *and* `.wasm` |
| D3 | Wasm ABI = core module (`pw_alloc/pw_free/pw_meta/pw_init/pw_call`, import `plugwright.log`), **not** the component model | wasmer and wasmi do not run components; a core ABI keeps engine adapters ~150 LOC each. Component model is F4 |
| D4 | Engine abstraction = 3 object-safe traits `WasmEngine → WasmModule → WasmInstance` with primitive ops (`call`, `read`, `write`, `begin_call`); ABI glue written once in the core | adding an engine = implementing primitives only |
| D5 | Crash isolation = child process (`Isolation::Process`) running the same native loader, framed binary protocol over stdin/stdout, plugin stdout redirected to stderr | SIGSEGV/abort/`exit()` in a plugin cannot be recovered in-process; a process boundary is the only reliable guard |
| D6 | Panic policy: SDK wraps every entry in `catch_unwind`; builder forces `profile.*.panic="unwind"` | a panic escaping `extern "C"` aborts the process (Rust ≥ 1.81) |
| D7 | Build via `cargo` subprocess (`--message-format=json-render-diagnostics`), never via cargo-as-library | stable interface, respects user toolchains/config |
| D8 | Two cache layers: L1 plugwright key (source fingerprint + toolchain + target + profile + features) → content-addressed artifact store; L2 cargo's own incremental cache in a shared target dir | L1 skips cargo entirely and survives `cargo clean`; L2 speeds up genuine rebuilds |
| D9 | Fingerprint covers every **path package** the plugin can reach, read from its manifests (`path`, `workspace = true`, `[patch]`, `[replace]`, config `paths`), not just the plugin dir; no `cargo metadata` | edits in a local path dependency (e.g. the SDK) must invalidate the key; over-approximating costs a miss, running cargo costs 100–200 ms per lookup |
| D10 | Git sources fetched with the `git` CLI into `cache/git/<url>-<sha>`, branch/tag resolved to a SHA and pinned in `plugwright.lock` | deterministic, reproducible; avoids cdylib-symbol re-export problems of a shim crate |
| D11 | Native load from a **shadow copy** with a unique name | Windows locks loaded DLLs; macOS dyld caches by path → reload would return the old image |
| D12 | Unload = `destroy` instance → `dlclose` → probe with `RTLD_NOLOAD` / `GetModuleHandleExW`; the verdict is returned, never assumed | TLS (every Rust cdylib on macOS), ObjC metadata or `RTLD_NODELETE` can make `dlclose` a no-op |
| D13 | Wasm guests get **no WASI** and one import (`log`) | deny-by-default: no FS, network, clock, env |
| D14 | Wasm limits: fuel per call, memory cap, wall-clock timeout on every engine (wasmtime epochs, wasmi fuel slices, wasmer host interrupt); a trapped instance is discarded and re-instantiated on next call | bounded CPU/RAM, no poisoned state |
| D15 | Versioning: ABI must match exactly; SDK semver-compatible with host; plugin version must satisfy manifest `version` req | three distinct, explicit error variants |
| D16 | Logging via `tracing` (spans `plugin{id}`), guest logs forwarded with target `plugwright::guest` | integrates with any subscriber |
| D17 | Sync API first; async is F7 | most plugin calls are short; async complicates every backend |
| D18 | `isolation = "process"` is the default, and the runner sandboxes itself before `dlopen` | in-process native code is fully trusted; that has to be an explicit choice |
| D19 | Artifact kind is decided by magic bytes (ELF / Mach-O / fat / PE / `ar` / Wasm), never by extension | `.so.1`, `.bundle`, renamed files; a wrong kind becomes `NotAPlugin` with a reason instead of a `dlopen` error |
| D20 | Static libraries are relinked with the C driver / `link.exe` found by the `cc` crate: entry forced with `-u` / `/INCLUDE`, the only export; no `--whole-archive` | whole-archive drags all of std and collides with the C runtime; forcing the entry pulls exactly the needed members |
| D21 | Build accelerators: what changes bytes (linker, backend, `-Zthreads`, profile) is in the cache key, sccache is not; explicit requests fail loudly, `auto` skips | correctness of the cache; no silent fallbacks the user asked against |
| D22 | Cranelift only for artifacts that are not loaded in-process | Cranelift-built code aborts on panic (verified on nightly 1.100, aarch64-darwin): in-process that kills the host, isolated it is a survivable crash |
| D23 | Typed calls through proc macros over the existing byte ABI: `#[plugin]` (guest) and `#[interface]` (host) share one wire convention decided **syntactically** from the signature (`&str`/`String` → UTF-8, `&[u8]`/`Vec<u8>` → bytes, ≥ 2 params → JSON array/object, other → JSON) | no ABI change (ABI_VERSION stays 1), raw callers keep working, binary payloads stay copy-free; a trait-based encoding cannot tell `String` from `T: Serialize` without specialization. Cost: aliases of `String` travel as JSON — documented |
| D24 | `#[interface]` keeps the trait as written (methods return `plugwright::Result<T>`) and generates `<Trait>Client<I: Invoke>`; `Invoke` is implemented for `Bound` (via the manager), `dyn Plugin`, `&mut T`, `Box<T>` | an honest signature (every call can fail), mockable trait, calls through the manager keep logging/timeouts/restarts |
| D25 | Artifact signatures are detached minisign files (`<artifact>.minisig`, prehashed ed25519), verified with `minisign-verify` against a per-source `TrustPolicy` before the sha256 pin, relinking, compiling or mapping; signing (`minisign`) is an opt-in feature | the format and tool users already have; zero dependencies on the host's load path, while signing's scrypt and password prompt stay out of hosts; a check before anything runs the artifact's code |
| D26 | Isolated payloads of at least `shm_threshold` bytes travel through one fixed-size shared memory region per runner (POSIX `shm_open` / a pagefile-backed section), mapped by the runner before LOAD applies the sandbox; frames carry `offset, length`, the pipe stays the control channel and the fallback | a 4 MiB call no longer streams through a pipe twice; mapping before the sandbox needs no new sandbox rule, and with the name unlinked and every descriptor closed nothing can resize the region under the host. Raw `libc` / `windows-sys`: `shared_memory` is unmaintained, `memmap2` maps files only, `ipc-channel` is heavy |
| D27 | extism modules run through `plugwright-extism`, beside plugwright's own Wasm ABI: extism's ABI and kernel, host functions the application grants through `HostEnv`, WASI preview 1 only when the application passes `Wasi` directories, exception handling on, no compilation cache; each such plugin runs on a `Worker` whose deadline thread cancels the call through extism's `CancelHandle` | kerf, weft and the coding agent already ship extism guests (Rust, Go, Kotlin PDKs) and need host functions and, for the coding agent, WASI, which D13 rules out for plugwright's own ABI; D13 still holds for `plugwright-wasm`. Ported from `wasm-plugin-host` 0.1, which it replaces. Feature `pulley` compiles modules for wasmtime's Pulley interpreter, for hosts that may not map executable memory (iOS) |


## 3. Security model

| threat | mitigation | residual risk |
|---|---|---|
| malicious plugin **source** (build.rs, proc-macros) | build only manifests you trust; `--locked`, `--offline`; lockfile pins SHAs | arbitrary code runs at build time — F3 (sandboxed builds) |
| tampered **artifact** in cache | sha256 recorded at build, verified on every load; `sha256` pin for prebuilt artifacts | attacker with write access to both cache and lock |
| forged or swapped prebuilt **artifact** | minisign signature against the source's trusted keys, before mapping / relinking / compiling; unsigned = deny / warn / allow per source (D25) | stolen signing key; a swap between the check and the load by someone with write access |
| native plugin memory corruption / crash | `Isolation::Process` (default); env cleared; core dumps off | — |
| native plugin reaching the system | runner sandbox: Seatbelt / Landlock + seccomp / low integrity + job | Windows: reads and network open (T49: AppContainer); Linux without Landlock: files open (reported) |
| in-process native plugin | explicit opt-in only; unload verified and leaks counted | full trust: same address space |
| native plugin panic | `catch_unwind` in SDK, forced `panic=unwind`, Cranelift never in-process (D22) | plugin built without the SDK |
| wrong or foreign file | magic-byte detection, kind / architecture / image type checked before `dlopen` (D19) | — |
| static library relink | archive sha256 pinned before linking; only the entry symbol exported | linker and `link_args` run with your privileges |
| ABI confusion | `abi_version` first field, `struct_size`, every slot checked, unknown status → error | — |
| Wasm guest escape | engine sandbox, no WASI, single `log` import, bounds-checked host reads | engine bugs |
| Wasm DoS | fuel, memory limiter, wall-clock timeout on all engines, stack limit, instance discarded on trap | wasmer on Windows: a timed-out worker is abandoned and stopped by fuel |
| protocol confusion (isolated) | length-limited frames, magic handshake, stdout redirect (fd 1 on unix; std handle + CRT fd 1 on Windows); shared memory ranges from the runner checked against the host's own region size and copied out, a bad one kills the runner (D26) | — |
| git URL tricks | scheme allowlist, `protocol.ext.allow=never`, no terminal prompts, no submodules | — |

