# plugwright — research

Date: 2026-10-14. Versions are taken from the crates.io API; unchecked facts are marked **unverified**.
Architecture, decisions and the security model: [design.md](design.md). Tasks: [plan.md](plan.md), finished ones in [done.md](done.md).

## 1. Goal

A Rust library that manages dynamic plugins:

1. builds plugin crates into `cdylib` from local sources or git;
2. caches artifacts and configs, and does not rebuild without changes;
3. loads/unloads `.so`/`.dylib`/`.dll` through `libloading`;
4. loads Wasm plugins through an abstract layer (wasmtime / wasmer / wasmi / a custom engine);
5. one trait for all backends;
6. load errors, version mismatches and plugin crashes do not bring the host down;
7. logging of loads and results;
8. security as far as possible.

## 2. Name

`plugwright` ("wright" is a craftsman who builds: the library *builds* and *installs* plugins).
As of 2026-10-14 these names are free on crates.io: `plugwright`, `plugwright-abi`, `plugwright-sdk`,
`plugwright-runner`, `plugwright-wasmtime`, `plugwright-wasmer`, `plugwright-wasmi`, `plugwright-cli`,
`plugwright-core`, `plugwright-macros` (the API answered 404). Checked and rejected: `loadsmith` (taken).
Spare free names: `plugforge`, `plugsmith`, `modloom`, `plugyard`.

## 3. Landscape: what already exists

| crate | version / date | what it does | what it lacks |
|---|---|---|---|
| extism | 1.41.0 / 2026-10-09 | Wasm plugins on wasmtime 49, own ABI, fuel, WASI, host functions | native plugins, building, engine switching |
| hot-lib-reloader | 0.8.2 / 2025-08 | hot reload of a native library in dev (shadow copy + watcher) | isolation, cache, Wasm |
| dynamic_reload | 0.10.0 / 2023-03 | shadow-copy reload | abandoned |
| dlopen2 | 0.9.0 / 2026-07 | typed dlopen wrapper | everything else |
| abi_stable | 0.11.3 / 2023-10 | layout check at load time | abandoned, **no unloading** by design |
| stabby | 72.1.16 / 2026-07 | stable ABI for enum/dyn/futures, rustc/target "canaries" | Wasm, isolation; license EPL-2.0 OR Apache-2.0 |
| plugy | 0.3.1 / 2023-12 | async Wasm plugins | abandoned |
| wasm-bridge, wasm_plugin_host | 2024 / 2021 | Wasm wrappers | abandoned |
| bevy_dynamic_plugin | 0.14.2 | native Bevy plugins | declared unsound, removed in 0.15 |

Telling example: **Zellij** moved from wasmer to wasmtime (0.41, 2024-11), and then from wasmtime to
**wasmi** (0.44, 2026-03, PR #4449). This supports the argument that the engine must be swappable.

**Gap:** no crate found combines building through cargo (path/git) + a content-addressed
cache + native and multi-engine Wasm loading + process isolation (the search is not exhaustive).

## 4. Native plugins

### 4.1 ABI
* Rust has **no stable ABI**; `repr(Rust)` gives no layout guarantees. Options: `#[repr(C)]` + a table of
  `extern "C"` functions; abi_stable (abandoned); stabby (alive, but complex and Rust-only); serialized
  messages over the C ABI.
* **Decision (D1):** a `#[repr(C)]` table `PwPluginDecl` (first field `abi_version`, then `struct_size`,
  pointers as `Option<extern "C" fn>`), the payload is opaque bytes (JSON/bincode is up to the
  plugin). This is the only option that works the same in-process, over IPC and in Wasm.
  The host and the plugin may be built with different rustc versions.
* Build compatibility key: `rustc -vV` (version, commit hash, host, LLVM) + target + profile + features —
  the same idea as the stabby "canaries"; for us it goes into the cache key.

### 4.2 libloading 0.9.0 (ISC, MSRV 1.88)
* `unsafe fn Library::new(..)`, `unsafe fn get<T>(..) -> Symbol<'_, T>` — the symbol borrows the library.
* `Drop` calls `dlclose`/`FreeLibrary` and **ignores errors**; there is `close() -> Result`.
* `new()` ≡ `RTLD_LAZY | RTLD_LOCAL`; library init/fini run on load/unload — hence `unsafe`.

### 4.3 Unloading pitfalls
* **glibc + Rust TLS:** a `thread_local!` with `Drop` registers its destructor through
  `__cxa_thread_atexit_impl`, which increments `l_tls_dtor_count`; `dlclose` is then a no-op.
  rust-lang/rust#88737 (open): dlclose is "fundamentally broken" with TLS destructors; #91979 — a crash
  on thread exit if the unload did happen.
* **macOS:** an image is not unloaded if it contains ObjC/Swift classes, C++ `thread_local`, if it
  is linked with main or lives in the shared cache (Apple DTS). A repeated `dlopen` of the same path may return
  the old image.
* **Windows:** a loaded DLL is locked — cargo cannot overwrite it.
* **Conclusion (D11, D12):** load from a **shadow copy** with a unique name (`<id>-<n>.<ext>`), like
  hot-lib-reloader/dynamic_reload; unloading is best effort, documented. For repeatable
  reload a new copy is enough; the old image may stay in memory.
* `tracing` inside a reloadable library may fail with "DefaultCallsite already exists" →
  the plugin does **not** link its own subscriber, it logs through a host callback (D16).

### 4.4 Panics and crashes
* `extern "C-unwind"` is stable since 1.71. **Since Rust 1.81 a panic that escapes `extern "C"`
  aborts the process.** `catch_unwind` catches only unwinding panics and is useless with `panic=abort`.
* SIGSEGV / stack overflow / `abort()` in a native plugin **cannot be reliably survived in-process**:
  returning from a SIGSEGV handler for a real fault is UB per POSIX (this wording is **unverified**).
* **Conclusion:** (D6) the SDK wraps every entry point in `catch_unwind`, the builder forces
  `panic="unwind"`; (D5) for untrusted plugins — a **child runner process**, whose crash
  becomes `PluginError::Crashed`, with automatic restart.

## 5. Wasm

### 5.1 Engines

| engine | version | license | fuel | memory limit | timeout | component model |
|---|---|---|---|---|---|---|
| wasmtime | 49.0.2 (50.0.0-rc.1), MSRV 1.96 | Apache-2.0 WITH LLVM-exception | `consume_fuel`, `Store::set_fuel` | `ResourceLimiter` / `StoreLimitsBuilder` | epoch interruption | yes |
| wasmi | 2.0.0, MSRV 1.86 | MIT/Apache-2.0 | `consume_fuel`, `Store::set_fuel` | `Store::limiter` | no (fuel only) | no |
| wasmer | 7.5.0, MSRV 1.96 | MIT | `wasmer-middlewares::metering` | Tunables / check after the call | `experimental-host-interrupt` | **no** |

* wasmtime 49: default features include `async`, `component-model`, `cranelift`, `pooling-allocator`, `cache`,
  `gc`, `threads`, `wat`; `Config::async_support` is marked deprecated ("no longer has any effect").
  API: `Config::{consume_fuel, epoch_interruption}`, `Store::{set_fuel, get_fuel, set_epoch_deadline,
  epoch_deadline_trap, limiter}`, `Engine::increment_epoch()`; `ResourceLimiter::memory_growing(current,
  desired, maximum) -> Result<bool>`.
* wasmer 7: backends `cranelift` (default), `llvm`, `singlepass`, `v8`, `js`; the WAMR/wasmi backends
  were removed in 7.2. The component model is not supported and not planned (the focus is WASIX).
* WASI: `wasm32-wasip2` — Tier 2 since Rust 1.82; WASI 0.3.0 was released on 2026-06-11 (wasmtime ≥ 46);
  `wasm32-wasip3` — Tier 3.

### 5.2 ABI choice
* The component model + WIT give typed interfaces, but work **only in wasmtime** → an abstraction
  over engines would be a fiction.
* **Decision (D3, D4):** a core module with a minimal ABI (like extism): exports `pw_abi_version`,
  `pw_alloc`, `pw_free`, `pw_meta`, `pw_init`, `pw_call`; the only import is `plugwright.log`.
  The engine adapter implements only primitives: compile, instantiate, `call(name, i32/i64 args)`,
  memory `read/write`, `begin_call` (fuel refill, deadline). All ABI logic and bounds checks live
  once in the core. The component model is a future option for wasmtime only (F4).
* No WASI (D13): no filesystem, network, clocks, env — deny by default.
* After a trap the instance is discarded and recreated lazily (D14) — no "poisoned" state.

## 6. Build and cache

* `cargo build --message-format=json-render-diagnostics`: `compiler-artifact` messages contain
  `manifest_path`, `target.{kind, crate_types}`, `filenames`, `fresh`; the last one is `build-finished`.
  `--locked`, `--offline`, `--frozen`; `--config KEY=VALUE` is repeatable; `--target-dir`;
  `--artifact-dir` is nightly only.
* Git dependencies are pinned by `rev`/`tag`/`branch`; `Cargo.lock` fixes the commit.
* Cargo has its own fingerprinting (`fresh: true`) — our cache sits **above** it (D8):
  * **L1** — the plugwright key (sha256: cache schema, plugwright version, ABI, `rustc -vV`, target, profile,
    features, RUSTFLAGS, a fingerprint of the sources of **all path packages** from `cargo metadata` + Cargo.lock) →
    content-addressed storage `artifacts/<k>/plugin.<ext>` + `entry.json` with sha256;
    a hit = cargo is not run at all, and it survives `cargo clean`.
  * **L2** — a shared `--target-dir` in the cache, incremental cargo build on real changes.
* Git sources (D10): the `git` CLI, branch/tag → SHA, the SHA is written to `plugwright.lock` — this is the
  "config cache": a repeated run does not touch the network and builds exactly the same commit.
* Cache locking: `std::fs::File::lock` (stable since Rust 1.89) — the external `fs4` is not needed.
* **Supply chain:** `build.rs` and proc-macros run arbitrary code at build time with the user's
  rights. Building a plugin = trusting its sources. Mitigation: `--locked`, lockfile, sha256 pins,
  and in the future a build sandbox (F3).

## 7. Security — the resulting model

| threat | protection |
|---|---|
| malicious source | trusted manifests only, `--locked`/`--offline`, SHA pin; F3 — sandbox |
| artifact substitution | sha256 at build time, check on every load, a `sha256` pin for prebuilt artifacts |
| native plugin crash | `Isolation::Process`, cleared env, core dumps disabled; F2 — OS sandbox |
| panic | `catch_unwind` in the SDK + forced `panic=unwind` |
| ABI confusion | `abi_version` as the first field, `struct_size`, check of all slots, unknown status → error |
| Wasm escape | engine sandbox, no WASI, one import, bounds checks when reading guest memory |
| Wasm DoS | fuel, memory limit, epoch timeout, stack limit, recreation after a trap |
| runner protocol corruption | length-limited frames, handshake, plugin stdout → stderr |
| git URL tricks | scheme allowlist, `protocol.ext.allow=never`, no interactive prompts, no submodules |

## 8. Rejected options

* **abi_stable** — abandoned since 2023, does not support unloading.
* **stabby** — powerful, but Rust-only, heavy, EPL; our ABI is simpler and fits IPC/Wasm.
* **A shim crate with a git dependency** instead of cloning — `#[no_mangle]` symbols of dependencies in a
  `cdylib` are exported unreliably, plus symbol conflicts.
* **A SIGSEGV handler** for "recovery" — UB.
* **cargo as a library** — unstable API, pulls in all of cargo.
* **The component model as the only ABI** — ties us to wasmtime.
* **wasmtime precompiled cache (`Module::deserialize`)** — `unsafe`, needs a signature; deferred (F5).

## 9. Environment limits

The machine has ~3–5 GiB of free disk, so debuginfo is off in `[profile.dev]`.

### 9.1 What turned out during implementation

* **wasmer and wasmtime do not coexist in one dependency graph.** wasmer 7.5.0 pins
  `cranelift-codegen = "=0.136.0"`, while wasmtime 49.0.2 requires `^0.136.2`, and cargo cannot
  reconcile them. So wasmer is not a core feature but a separate crate `crates/plugwright-wasmer` outside the workspace
  (`exclude`) with its own `Cargo.lock`. A host that needs wasmer depends on `plugwright` without
  `wasmtime` and registers the engine through `register_engine`.
* **wasmer 7 `Store` is not `Send`** (it holds `NonNull` pointers to VM objects and a non-`Send` `on_called` callback).
  The manager needs the trait `WasmInstance: Send`, so every wasmer instance lives in its own
  owner thread and calls go through a channel. A panic inside wasmer stays in that thread and
  becomes a trap.
* **Memory limit in wasmer:** it has no `ResourceLimiter`. It is done through custom `Tunables`:
  every guest memory is wrapped in a `LinearMemory` that refuses `grow` beyond the limit and
  sets a flag. As a result the limit is hard, not a check after the call. Tunables belong to the `Engine`;
  a clone of `Engine` shares compiled artifacts but not tunables, so each instance
  has its own limit.
* **wasmi 2.0** does not re-export `LimiterError` from the crate root; `wasmi_core` is needed.
* **Fuel in wasmtime is also spent on `memory.fill`/memset:** the `oom` example, which filled memory,
  hit the fuel limit before the memory limit. The test was rewritten to use `Vec::with_capacity`, so that
  it checks the memory limit itself.
* **`cargo metadata` cost 3–12 s on every `prepare`** (a lock on the package cache with parallel
  tests). A memo keyed by `(len, mtime)` of all manifests and `Cargo.lock` was added. A cache hit now
  does not run cargo at all.

### 9.2 Round 2: sandbox, timeouts, unloading

* **macOS does not unload images that have thread-local variables.** Check after `dlclose` via
  `dlopen(path, RTLD_NOLOAD)`: a C dylib without `__thread` is unloaded, the same dylib with a single
  `__thread` variable stays in memory. Rust std uses TLS, so **any** Rust cdylib on
  macOS stays mapped after `dlclose`. So in-process unload on macOS always leaks the
  image; `unload` now honestly returns `StillMapped`, and the manager counts such images.
  Guaranteed unloading is only process isolation (it is now the default).
* **wasmi: lazy translation charges fuel in a way that makes the call impossible to resume.** When a
  function is compiled on its first call, wasmi charges fuel for the translation and, if there is not enough,
  returns `ResumableOutOfFuelError` as an *error*, not as `ResumableCall::OutOfFuel`. With fuel sliced
  into 100k pieces the very first call failed. The fix is `CompilationMode::Eager`: the compilation cost is paid
  once in `compile`, and fuel use during a call becomes predictable.
* **wasmer `experimental-host-interrupt`** works only on unix and is implemented through `SIGUSR1`:
  wasmer installs its own handler and passes foreign signals to the previous one. The signal can arrive before
  guest code is entered and be lost, so the host repeats `interrupt()` during a grace period
  (200 ms). During a host import the interrupt is postponed until the return to Wasm. If the worker still
  does not answer (Windows, a long import), its thread is detached, and fuel limits the CPU.
* **Seatbelt: the last matching rule wins**, so the profile is built as
  `(allow default)`, then `deny` for file-read-data/write/exec/fork/network/mach-lookup, then
  targeted `allow` rules for system paths and the allowlist.
* **Linux in Docker (OrbStack kernel 7.0)**: Landlock ABI v8 is available even under Docker's default
  seccomp profile. `clone3` has to return `ENOSYS`, not `EPERM`: glibc falls back to `clone`
  only on `ENOSYS`, otherwise `std::thread::spawn` breaks.
* **Environment:** the image `rust:1.97.1-slim` (~2.9 GB) for Linux tests filled the disk to ENOSPC so
  that even the agent's commands would not start. Run Linux checks with `target` on tmpfs and
  delete the image right away.

### 9.3 Round 3: formats, static libraries, faster builds

* **Format by content, not by extension.** Magic numbers: ELF `\x7fELF` (e_type/e_machine),
  Mach-O `0xFEEDFACF`/`0xFEEDFACE`, fat `0xCAFEBABE` — Java class files have the same magic,
  so fat is accepted only with 1–32 slices; PE `MZ` → `e_lfanew` → `PE\0\0`
  (DLL flag `0x2000`), `ar` `!<arch>\n`, Wasm `\0asm` (version 1 — core module, otherwise a component).
  On Linux the test binary is a PIE (`ET_DYN`), so an "executable instead of a library"
  cannot be told apart there by the header; the check is skipped.
* **Relinking `.a` → shared.** `--whole-archive` / `-force_load` / `/WHOLEARCHIVE`
  were rejected: a Rust staticlib contains all of std, everything is pulled into the image and conflicts with the CRT.
  Instead the entry symbol is declared undefined (`-u` / `/INCLUDE`) and made
  the only export — the linker takes only the needed archive members. Native dependencies of std
  come from `rustc --print native-static-libs`. On macOS the resulting `static-hello` dylib is 1.2 MB and
  has exactly one export, `_plugwright_entry_v1`.
* **Cranelift does not unwind the stack.** Checked on nightly 1.100 (aarch64-apple-darwin):
  `catch_unwind(|| panic!())` in code built by Cranelift terminates the process. For a plugin
  in the host process this is a host crash, so Cranelift is used only for isolated
  plugins (there a panic becomes `Crashed` and a restart) and is not used for Wasm.
* **sccache** does not cache crates that call the linker (`cdylib`, `bin`, `proc-macro`) and
  incremental builds, but it does cache rlib dependencies — the bulk of the work. It only reproduces rustc output, so it is not
  part of the cache key; the linker, backend, `-Zthreads` and profile settings are.
* **Linker.** On x86_64 Linux rustc has used rust-lld by default since 1.90, so `auto`
  enables only mold (if installed); on macOS ld-prime is already fast.
* **Plugin toolchain.** cargo now runs in the crate directory, so that the plugin's `rust-toolchain`
  applies the same way as to `rustc -vV` in the cache key; the `--manifest-path` and
  `--target-dir` paths must then be absolute (otherwise relative manifests break —
  caught by the demo, not by e2e).
* **Observation:** at a load average of 140–190 the first start of the freshly linked
  `plugwright-host` as a runner took more than 10 s (handshake timeout), the next one 16 ms.
  This is the delay of the first exec of a new binary under load, not a protocol error.
* The names `plugwright` and `cargo-plugwright` are free on crates.io (checked 2026-10-14).

### 9.4 Round 4: macros and checks on Linux

* **Encoding by syntax, not by trait.** The trait `IntoOutput` cannot tell
  `String` from `T: Serialize` without specialization (String is also Serialize), so `#[plugin]` and
  `#[interface]` decide by the tokens of the signature: `&str`/`String` → UTF-8, `&[u8]`/`Vec<u8>` →
  bytes, otherwise JSON; ≥ 2 parameters → a JSON array or object. Cost: an alias of `String` goes as
  JSON — documented (D23). The ABI does not change, "raw" calls `call(id, method, bytes)`
  keep working.
* **`#[interface]` does not rewrite the trait.** Methods must return
  `plugwright::Result<T>`: an honest signature (any call can fail) and the trait is usable for
  mocks. The client is `<Trait>Client<I: Invoke>`; `Invoke` is implemented for `Bound` (through the manager —
  logging, timeouts, restarts), `dyn Plugin`, `&mut T`, `Box<T>`. A blanket impl for all
  `P: Plugin` was rejected: it overlaps with `&mut T`/`Box<T>` (fundamental types).
* **`#[macro_export]` macros cannot be generated by another macro** and then re-exported by
  path (`macro_expanded_macro_exports_accessed_by_absolute_paths`), so `log::info!` and the others
  are written explicitly, and the level functions go through a local `macro_rules!`.
* **Cost of `macros` in Wasm:** serde_json grows the release wasm-hello module from 48 to 133 KB.
  For minimal modules use `default-features = false` and a manual `impl Plugin`.
* **Clippy in someone else's crate.** The generated `Ok(f()?)` triggers `needless_question_mark` in the
  user's crate — replaced with `.map_err(Into::into)`.
* **Deduplication.** A code review found only two places where `macro_rules!` is really
  needed: the `WasmVal` conversion in three engines (the orphan rule forbids `From`) →
  `wasm_val_bridge!`, and `assert!(matches!(..))` in tests → `assert_err!` with a clear
  message. The rest (13× `PluginError::Build{..}`, `BuildRequest` literals, `lto`/`strip`
  checks, magic format tables) is better solved by a constructor, a serde enum, or
  stays a `match` — a macro would only hide the logic.
* **Linux (colima, Debian bookworm, aarch64, kernel 6.8):** all tests, `doctor` (sandbox
  `no_new_privs+landlock-v4+seccomp`), the demo and the wasmer adapter passed. A bug was found: the probe
  `rustc --print native-static-libs` received a relative cache path, while rustc runs in the
  crate directory — the same class of bug as with `--manifest-path`. On macOS an already
  written memo file hid it. Fixed with `std::path::absolute`.

## 10. Sources

* crates.io API `/api/v1/crates/<name>` — versions and licenses (2026-10-14).
* docs.rs: wasmtime 49.0.2 (`Store`, `ResourceLimiter`, `StoreLimitsBuilder`), wasmi 2.0.0 (`Store`), libloading 0.9.0.
* https://blog.rust-lang.org/2024/09/05/Rust-1.81.0/ — abort on a panic through `extern "C"`.
* https://doc.rust-lang.org/std/panic/fn.catch_unwind.html
* https://github.com/rust-lang/rust/issues/88737, https://github.com/rust-lang/rust/issues/91979
* https://developer.apple.com/forums/thread/122591, https://developer.apple.com/forums/thread/670849
* https://github.com/rksm/hot-lib-reloader-rs
* https://github.com/wasmerio/wasmer/blob/main/CHANGELOG.md
* https://doc.rust-lang.org/rustc/platform-support/wasm32-wasip2.html, https://wasi.dev/releases/wasi-p3
* https://doc.rust-lang.org/cargo/reference/external-tools.html, https://doc.rust-lang.org/cargo/commands/cargo-build.html
* https://doc.rust-lang.org/cargo/reference/build-scripts.html
* https://github.com/ZettaScaleLabs/stabby, https://docs.rs/extism/1.41.0
* https://github.com/rust-lang/rustc_codegen_cranelift, https://doc.rust-lang.org/nightly/cargo/reference/unstable.html#codegen-backend
* https://github.com/mozilla/sccache, https://github.com/rui314/mold
* https://doc.rust-lang.org/rustc/command-line-arguments.html#--print-print-compiler-information (native-static-libs)
* https://docs.rs/cc/1.8.0/cc/windows_registry/fn.find.html
