# plugwright — done

Finished tasks, moved out of [plan.md](plan.md). Decisions D1–D24 and the crate layout they refer
to are in [design.md](design.md). Statuses: `[x]` done. Complexity: C1 (trivial) … C5 (hardest).

Order: risk/complexity first, prerequisites pulled forward when a hard task needs them.

| ID | Task | C | Deps | Status |
|---|---|---|---|---|
| T01 | ABI contract crate | C2 | — | [x] |
| T02 | Guest SDK + `export_plugin!` (native + Wasm) | C3 | T01 | [x] |
| T03 | Native in-process loader (shadow copy, decl validation, unload) | C3 | T01 | [x] |
| T04 | Process isolation: protocol, runner, crash/timeout detection, restart policy | C5 | T03 | [x] |
| T05 | Wasm runtime abstraction + generic ABI glue + trap recovery | C4 | T01 | [x] |
| T06 | wasmtime backend (fuel, epoch timeout, memory limiter) | C4 | T05 | [x] |
| T07 | Builder: cargo invocation, artifact discovery, panic=unwind, wasm target | C4 | — | [x] |
| T08 | Cache: fingerprint, key, CAS store, integrity, file lock, atomic writes | C4 | T07 | [x] |
| T09 | Git sources: fetch, SHA resolution, URL policy | C3 | T08 | [x] |
| T10 | Manager + unified `Plugin` trait + version checks + engine registry | C3 | T03–T06, T08 | [x] |
| T11 | Manifest + lockfile (config caching) | C3 | T08, T09 | [x] |
| T12 | wasmi backend | C3 | T05 | [x] |
| T13 | wasmer backend (separate crate, own lockfile) | C4 | T05 | [x] |
| T14 | Logging: spans/events, guest log forwarding | C2 | T10 | [x] |
| T15 | Example plugins: native-hello, wasm-hello | C2 | T02 | [x] |
| T16 | Host binary `plugwright-host` | C2 | T10, T11 | [x] |
| T17 | Integration tests | C3 | T15 | [x] |
| T18 | Docs: README, AGENTS.md, mise tasks | C1 | all | [x] |

### T01 — ABI contract (C2)
`plugwright-abi`, `#![no_std]`, no deps. `PwStr`, `PwBuf`, `PwHostApi`, `PwPluginDecl`
(`abi_version` first, `struct_size`, name/version/sdk_version, `create/call/destroy/free_buf` as
`Option<unsafe extern "C" fn>`), status codes as plain `u32` (unknown → error, no transmute),
log levels, `wasm::{pack, unpack}` and export/import names.
**Done when:** layout documented, pack/unpack round-trip test passes.

### T02 — Guest SDK (C3)
`trait Plugin { fn new(config) ; fn call(method, input) }`. Native: `create/call/destroy`
generic over `T`, each wrapped in `catch_unwind`; buffers returned as `Vec` parts and freed by
the plugin's own `free_buf`; host log API stored in a `OnceLock`. Wasm: `alloc`/`free` via
`Layout`, instance in a `thread_local`, status-prefixed result buffers, panic hook → host log.
**Done when:** the same example crate compiles for host target and `wasm32-unknown-unknown`.

### T03 — Native loader (C3)
Copy artifact to `cache/loaded/<id>-<n>.<ext>`; `Library::new`; resolve `plugwright_entry_v1`;
validate: non-null decl, `abi_version` (read before anything else), `struct_size`, all slots
`Some`, strings UTF-8 ≤ 256 bytes. `NativePlugin { instance, decl, lib: Arc<Library> }`, `Drop`
calls `destroy` then drops the library, then removes the shadow file. Host log ctx is leaked
(one small allocation per load) so late log calls from plugin threads stay memory-safe.
**Done when:** load → call → unload works, ABI mismatch and missing symbol give typed errors.

### T04 — Process isolation (C5)
* Frame: `u32 LE length | u8 tag | fields as (u32 len, bytes)*`; max frame 64 MiB; magic handshake.
* Host → runner: `Hello`, `Load{path, config}`, `Call{method, input}`, `Shutdown`.
  Runner → host: `Ready`, `Loaded{name, version, sdk}`, `Ok{bytes}`, `Err{kind, msg}`, `Log{level, msg}`.
* Runner: `dup` stdout to a private fd and `dup2(stderr → stdout)` before loading (Unix) so
  `println!` in a plugin cannot corrupt the protocol; `RLIMIT_CORE = 0`; env cleared by host.
* Host: `IsolatedPlugin` spawns the runner (self re-exec with `--plugwright-runner` or an explicit
  binary), reader thread → channel; `call` uses `recv_timeout`; EOF → `Crashed{status}`;
  timeout → kill → `Timeout`; stderr lines forwarded to `tracing`.
* Restart policy: `max_restarts` within a window; the next call transparently respawns and reloads.
**Done when:** a SIGSEGV and an `abort()` in the example plugin return errors, the host keeps
running, and the next call succeeds after an automatic restart.

### T05 — Wasm abstraction (C4)
`WasmEngine::compile(&[u8]) → Box<dyn WasmModule>`; `WasmModule::instantiate(&WasmLimits, HostLog)
→ Box<dyn WasmInstance>`; `WasmInstance::{call, read, write, begin_call}`; `WasmError`
(`Compile/Link/Trap/OutOfFuel/Timeout/MemoryLimit/MissingExport/Abi`). `WasmPlugin` implements the
ABI glue once (validate `pw_abi_version`, read `pw_meta`, `pw_init` with config, `pw_call`), bounds
checks every guest pointer/length, discards the instance after any trap and re-instantiates lazily.
**Done when:** the glue runs unchanged over two engines.

### T06 — wasmtime backend (C4)
`Config::consume_fuel(true)`, `epoch_interruption(true)`, `max_wasm_stack`; one ticker thread per
engine (stopped on drop); `ResourceLimiter` that records a hit flag so traps map to
`MemoryLimit`; `Trap::OutOfFuel` → `OutOfFuel`, `Trap::Interrupt` → `Timeout`; `log` host function
reads guest memory with bounds checks and a length cap.
**Done when:** `loop` → OutOfFuel/Timeout, `oom` → MemoryLimit, `trap` → Trap, plugin usable after.

### T07 — Builder (C4)
`cargo build --lib --release --message-format=json-render-diagnostics --manifest-path …
--target-dir <cache>/target [--target wasm32-unknown-unknown] [--locked] [--offline]
[--features …] --config profile.release.panic="unwind"`; parse `compiler-artifact` messages,
pick the `cdylib` whose `manifest_path` matches; collect diagnostics for `Build` errors (last 40
lines). `CARGO` env respected. Child env: inherits (toolchains need it) — build-time code
execution is a documented trust boundary (see design.md §3).
**Done when:** both example plugins build through the API and the artifact path is returned.

### T08 — Cache (C4)
Key = sha256 of: cache schema, plugwright version, ABI version, `rustc -vV` (remembered in
`meta/` while the toolchain selectors and the compiler binary are unchanged), target, profile,
sorted features, `RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS`/`CARGO_PROFILE_*`/`CARGO_BUILD_*`/
`CARGO_TARGET_*`, source fingerprint (paths + contents of every path package found from the
manifests, skipping `target/`, `.git/`, plus the workspace manifests, `Cargo.lock` and every cargo
config file from the crate up and in `$CARGO_HOME`, absent ones included). Store
`cache/artifacts/<k[0..2]>/<k>/{plugin.<ext>, entry.json, verified.json}` with sha256; verify
sha256 on a hit unless the file's stat matches the one recorded at the last verification once its
times had settled (mismatch → evict + rebuild + `warn!`). Exclusive `File::lock` on
`cache/.lock` while building; writes via temp file + rename. `gc(keep)` removes unreferenced entries.
**Done when:** second load does not spawn cargo; editing a source file or the SDK triggers a rebuild.

### T09 — Git sources (C3)
`source = { git, rev | branch | tag, subdir }`. Allowed schemes: `https://`, `ssh://`, `git@`
(+ `file://` only with `allow_local_git`). `git init` + `fetch --depth 1 origin <ref>` +
`checkout --detach FETCH_HEAD`, `GIT_TERMINAL_PROMPT=0`, `-c protocol.ext.allow=never`; resolved
SHA recorded in the lockfile; checkout dir completed atomically (marker file).
**Done when:** a local bare repo (test, `allow_local_git`) builds and the SHA lands in the lock.

### T10 — Manager (C3)
`trait Plugin: Send { fn info(&self) -> &PluginInfo; fn call(&mut self, method, input) -> Result<Vec<u8>> }`
plus `call_json`. `PluginManager::{new, register_engine, load, load_all, unload, reload, call,
call_json, get, list}`; `load_all` never aborts on one failure. Version checks (D15).
**Done when:** host demo loads all three backends through one API.

### T11 — Manifest + lockfile (C3)
`plugwright.toml`: `[settings]` (cache dir, locked, offline, runner) and `[[plugin]]` (`id`, `kind`
native|wasm, `source`, `version`, `isolation`, `engine`, `features`, `config` table, `[limits]`,
`timeout_ms`, `sha256`). `plugwright.lock`: resolved source, cache key, sha256 per plugin; written
atomically; reused so git branches stay pinned and prebuilt artifacts are integrity-checked.
Unknown fields rejected (`deny_unknown_fields`).
**Done when:** round-trip tests; bad manifest produces a precise error.

### T12 — wasmi backend (C3)
Same primitives with `wasmi::{Engine, Store, Linker, StoreLimits}`; fuel; no wall-clock timeout
(documented: fuel is the CPU bound).

### T13 — wasmer backend (C4)
Planned as a feature of `plugwright`, but wasmer 7.5 pins `cranelift-codegen =0.136.0` while
wasmtime 49.0.2 needs `^0.136.2`, so both cannot live in one dependency graph. Shipped as the
separate crate `crates/plugwright-wasmer` (excluded from the workspace, own `Cargo.lock`); a host
registers it with `PluginManager::register_engine`.

* fuel — `wasmer-middlewares` metering (1 point per operator), `Exhausted` → `OutOfFuel`;
* memory — custom `Tunables` wrap every guest memory in a `LinearMemory` that refuses to grow past
  the limit and raises a flag → `MemoryLimit` (hard limit, not a post-call check);
* no wall-clock timeout (as wasmi); fuel is the CPU bound;
* wasmer 7 `Store` is not `Send` → each instance lives on its own worker thread, calls go over a
  channel; a panic inside wasmer stays on that thread and becomes a trap.

Test: `crates/plugwright-wasmer/tests/e2e.rs` (greet/reverse/fuel/oom/trap/re-instantiation).

### T14 — Logging (C2)
Spans: `plugin{id, backend}`; events: `cache.hit/miss`, `build.start/finish{duration}`,
`load{abi, version, sdk}`, `call{method, bytes_in, bytes_out, duration, result}`, `unload`,
`crash{status}`, `restart{n}`. Guest log levels mapped 1:1.

### T15 — Examples (C2)
`native-hello` (`greet`, `add` JSON, `panic`, `crash` via null write, `abort`, `sleep`),
`wasm-hello` (`greet`, `reverse`, `loop`, `oom`, `trap`). Both `crate-type = ["cdylib"]`.

### T16 — Host binary (C2)
`plugwright-host --manifest examples/plugwright.toml <demo|list|call <id> <method> [input]|build|gc>`,
`RUST_LOG` filter, runner mode via self re-exec.

### T17 — Integration tests (C3)
`examples/host/tests/e2e.rs` (9 tests, real cargo/dlopen/child processes/engines). Build + cache hit, ABI mismatch, version mismatch, panic in-process, crash isolated + restart,
Wasm fuel/memory/trap recovery, manifest errors, integrity mismatch.

### T18 — Docs (C1)
README (quick start, security model, platform notes), AGENTS.md, `mise run check|test|demo`.

## 3b. Round 2 — closing the known limitations, crates split, library formats

Order: T19–T24 (limitations), then T28 (crates split), then T25–T27 (library formats) and T29–T30 (build acceleration, cargo subcommand) in the new crates.

Limitations left after T01–T18: (L1) in-process unload is best effort and silent about it;
(L2) the isolated runner has the host user's full privileges, and in-process is the default;
(L3) wasmi/wasmer have no wall-clock timeout; (L4) Windows runner does not move plugin stdout
off the protocol pipe.

| ID | Task | C | Closes | Status |
|---|---|---|---|---|
| T19 | OS sandbox for the runner: Seatbelt (macOS), Landlock + seccomp + `no_new_privs` (Linux), low integrity + job object (Windows); runner loads the artifact read-only, no shadow copy | C5 | L2 | [x] |
| T20 | Wall-clock timeout for wasmi (resumable out-of-fuel slices) and wasmer (host interrupt + worker abandonment) | C4 | L3 | [x] |
| T21 | Unload verification: probe whether the image is still mapped after `dlclose`, report it from `unload`, count leaked images | C3 | L1 | [x] |
| T22 | Windows runner: duplicate the stdout handle for the protocol, point `STD_OUTPUT_HANDLE` and CRT fd 1 at stderr | C3 | L4 | [x] |
| T23 | Secure defaults: `isolation = "process"` is the default; in-process must be explicit | C2 | L2 | [x] |
| T24 | Tests (sandbox denies write/network/exec, wasm timeouts on all engines, unload report), docs, cross-target clippy | C2 | all | [x] |
| T28 | Decompose into crates: `plugwright-core` (trait, errors, logging), `-build` (builder, cache, git), `-native` (loader, formats, static relink), `-isolate` (runner, protocol, sandbox), `-wasm` (engine traits + glue), `-wasmtime`, `-wasmi`, `-wasmer`; `plugwright` = manifest + manager + re-exports | C4 | new | [x] |
| T25 | Artifact formats: detect by magic bytes (ELF / Mach-O / PE / `ar` / Wasm), accept `.so` `.so.N` `.dylib` `.bundle` `.dll` `.a` `.lib` `.wasm` | C3 | new | [x] |
| T26 | Static libraries: relink `.a` / `.lib` (prebuilt or `crate-type = ["staticlib"]`) into a cached shared library (entry forced with `-u` / `/INCLUDE`, D20) | C4 | new | [x] |
| T27 | Tests + docs for T25–T26 | C2 | new | [x] |
| T29 | Build acceleration (`[settings.build]`): auto-detect and apply `sccache` (`RUSTC_WRAPPER`), fast linker (mold / lld), Cranelift codegen backend + `-Zthreads` on nightly, tuned plugin profiles; accelerators that change output go into the cache key, `sccache` does not | C4 | new | [x] |
| T30 | `cargo plugwright` subcommand (`cargo-plugwright`): `build`, `list`, `call`, `gc`, `doctor` (toolchain, targets, accelerators, sandbox support) | C3 | new | [x] |
| T31 | `plugwright-macros`: `#[plugin]` on an inherent impl (constructor `new`/`#[plugwright(init)]`/`Default`, `skip`, `name`, `plugwright.describe`, compile errors for async/generic/by-value self/`&mut` params) re-exported by the SDK (feature `macros`) | C4 | new | [x] |
| T32 | `#[interface]` + `Invoke`, `Bound`, `PluginManager::{bind, describe}`, `cargo plugwright methods` | C3 | new | [x] |
| T33 | Examples migrated to `#[plugin]`; e2e for typed calls (array/object args, unknown args, describe, native + wasm); SDK expansion tests; macro error tests | C2 | new | [x] |
| T34 | `macro_rules!` dedup: SDK log levels + `log::info!`-style macros, `wasm_val_bridge!` for the three engines, `assert_err!` in e2e; `PluginError::build` constructor instead of a macro | C2 | new | [x] |
| T35 | Test coverage pass: wire classification, native ABI through `plugwright_entry_v1`, mock Wasm engine, isolate framing + fake runner, CLI usage, manifest/lockfile/cache/git/accel/relink, format sniffing; fixes: pre-release SDKs, Wasm version/layer, `e_lfanew`/field-length overflow, truncated frame prefix, CLI usage before manifest lookup, sccache out of the cache key | C3 | new | [x] |
| T36 | Lockfile pins only what loaded (`resolve` + `pin`; `prepare` still pins for explicit builds; a failed `reload` keeps the old pin and cache entry); `sha256` format checked at parse; native artifacts sniffed before a runner is spawned; runner loop `serve(reader, out)` unit-tested; engine `read` bounds-checks before allocating | C2 | new | [x] |
| T37 | Low-overhead path: `Loader` (`open`/`load` for cdylib, staticlib, `.so`/`.dylib`/`.dll`/`.a`/`.lib`; no manifest, hashing or lockfile); `Placement::{InPlace, Copy}` with a reusable copy pool, a `MAPPING` lock and `prune_copies`; memoised `sha256_file` by file identity; linker resolution memoised per process and in `meta/`; `sha2` optimised in dev. Warm: open 1 ms, load 2–4 ms (was 90–1200 ms) | C3 | new | [x] |
| T38 | Manager warm path without subprocesses: no `cargo metadata` (manifests read with `toml`, `sources.rs`); `rustc -vV` remembered on disk; cache hits trust a settled recorded stat (`verified.json`); workspace manifest, ancestor and `$CARGO_HOME` cargo configs and cargo output env now in the key. Fresh process, warm cache: native 4–7 ms, static 5–8 ms (was 100–300 ms) | C3 | new | [x] |
| T39 | Wasm load path and crate split: `ModuleCache` (one compiled module per engine and sha256 while any plugin uses it; `WasmEngine::compile_cached`, wasmtime `.cwasm` under `compiled/` framed with sha256, written atomically), `gc_cache` prunes it; `plugwright-loader` crate (`Loader::{open, load, load_wasm, build_wasm}`, `with_engine`, `with_limits`, lazily created engine, `builtin_engines`), `plugwright` depends on it and re-exports it. wasmtime load: 1.1 s → ~2 ms in a fresh process, 0.3 ms within one | C3 | new | [x] |
| T40 | Native and Wasm loading split: `NativeLoader` and `WasmLoader` with typed results and a shared cargo-build helper; `Loader` composes them over one cache and dispatches `open` by magic bytes; feature `wasm` (implied by `wasmtime`/`wasmi`) gates every Wasm crate, so a native-only build has 54 dependencies instead of 151 | C3 | new | [x] |
| T41 | Performance tests: `examples/host/benches/perf.rs` (`harness = false`, no new dependencies), 16 hot and fresh-process cases with budgets about 10× the medians, `PLUGWRIGHT_PERF_SCALE`, change against the last run; smoke mode under `cargo test --benches`. Found and fixed: dropping a `WasmtimeEngine` waited out a 10 ms epoch tick (fresh `WasmLoader` open 12.8 → 0.9 ms), and a plugin outliving its engine lost its wall-clock timeout (the ticker is now shared with compiled modules and instances, parked rather than asleep, and started at the first instantiation; on aarch64 Linux an earlier thread turned wasmtime's first `membarrier` registration into a ~10 ms RCU wait: fresh wasm load 9.1 → 0.8 ms) | C3 | new | [x] |

### T19 — Runner sandbox (C5)
Applied by the runner to itself after the handshake and before `dlopen`, so library initialisers
already run confined. Policy (`[plugin.sandbox]`, default on): no file writes, no network, no new
processes; reads limited to system library paths, the artifact and `read = [...]`; `write = [...]`
and `network = true` opt in.
* macOS: `sandbox_init` with an SBPL profile (`deny default`-style for writes/network/exec,
  reads allow-listed).
* Linux: `PR_SET_NO_NEW_PRIVS`; Landlock (best available ABI, file read/write allow-list);
  seccomp-bpf: architecture check (x32 denied), `execve`/`execveat`/`fork`/`vfork`/`ptrace`/
  `process_vm_*`/`pidfd_getfd`/`io_uring_*` denied, `clone3` → `ENOSYS` (libc falls back to
  `clone`), `clone` only with `CLONE_THREAD`, `kill`/`tgkill` only to itself, `socket` denied
  unless `network = true`. Missing Landlock is reported, not silently ignored (`require = true`
  turns it into a load error).
* Windows: the runner lowers its own token to Low integrity (no writes to medium-integrity
  paths) and puts itself in a job object with `ActiveProcessLimit = 1`, kill-on-close,
  die-on-unhandled-exception and UI limits. Reads and network stay open (needs an AppContainer
  created by the parent — F2) and are reported as gaps.
* The runner reports what was enforced in `LOADED`; the host logs it.
**Done when:** e2e plugin methods `write_file`, `connect`, `spawn` fail under the sandbox and
succeed with it disabled (macOS locally, Linux in a container).
**Result:** `crates/plugwright/tests/sandbox.rs` + e2e `sandbox_denies_…` pass on macOS
(`seatbelt`) and Linux 7.0 in Docker (`no_new_privs+landlock-v8+seccomp`).

### T20 — Wasm wall-clock timeouts (C4)
* wasmi: `begin_call` stores the deadline and the fuel budget; calls run in 100k-fuel slices
  via `call_resumable`; on `OutOfFuel` the slice is refilled while budget and time remain.
  Compilation is eager: lazy translation charges fuel in a way that cannot be resumed.
* wasmer: feature `experimental-host-interrupt` (unix) — the host side waits with
  `recv_timeout`, then interrupts the store; if the worker still does not answer (non-unix, or a
  host call in progress) it is abandoned and fuel bounds its CPU. The instance is discarded.
  The interrupt is retried for a 200 ms grace (a signal can land just before guest entry).
  Feature `host-interrupt` (default) — wasmer then owns `SIGUSR1`.
**Done when:** `loop` with huge fuel returns `Timeout` within `timeout + 100 ms` on every engine.
**Result:** wasmtime/wasmi in `wasm_wall_clock_timeout_on_every_engine`, wasmer in its own e2e.

### T21 — Unload verification (C3)
After `dlclose`: unix `dlopen(path, RTLD_NOLOAD)` (balanced by `dlclose`), Windows
`GetModuleHandleExW(UNCHANGED_REFCOUNT)`. `Plugin::close(self: Box<Self>) -> Release`
(`Unmapped | StillMapped | ProcessExited | Freed`); `PluginManager::unload` returns it, and
`leaked_images()` counts images that stayed mapped (unload and reload). Isolated plugins report
`ProcessExited` only after the runner has exited. Guaranteed unload = process isolation.
**Finding:** on macOS dyld never unmaps an image with thread-local variables, and Rust std
gives every cdylib some, so in-process unload there always reports `StillMapped`.

### T22 — Windows stdout (C3)
`DuplicateHandle` (non-inheritable) for the protocol, then `_dup2(2, 1)` and
`SetStdHandle(STD_OUTPUT_HANDLE, stderr)`. Rust `println!` re-reads the std handle on each write,
so plugin output lands on stderr. Compile + clippy for `x86_64-pc-windows-msvc`.

### T23 — Secure defaults (C2)
`Isolation::default() = Process`; demo manifest marks the in-process plugin explicitly.

### T24 — Tests and docs (C2)
New example methods and e2e tests; README security table and platform notes rewritten; clippy for
`aarch64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`.

### T28 — Crates split (C4)
Done as listed in design.md §1; `WasmError` moved to `plugwright-core` so engine crates do not depend on
each other; the facade keeps the old module paths (`plugwright::builder`, `::native`, …) as
re-exports. wasmer depends on the split crates instead of the whole facade.

### T25 — Artifact formats (C3)
`plugwright_native::format`: `detect(&[u8])` → `Elf{kind,arch} | MachO{kind,arch} |
MachOFat{arches} | Pe{dll,arch} | Archive | Wasm | WasmComponent | Unknown`. Fat headers accept
1–32 slices (rules out Java class files, same magic). `check_loadable` runs before the shadow copy;
the shadow copy keeps the platform's extension. The manager checks Wasm specs too: a native
image as `kind = "wasm"` is `NotAPlugin`, a component is `Unsupported`.

### T26 — Static relink (C4)
`Builder::relink(RelinkRequest{id, archive, archive_sha256, link_args})`: triple from `rustc -vV`,
linker via `cc` (`try_get_compiler`, `windows_registry::find(…, "link.exe")`), native libs from
`rustc --print native-static-libs` (probe crate, memoised per triple in `meta/`), ELF version
script / `-exported_symbol` / `/EXPORT`. Key = schema + archive sha + triple + linker + args.
The builder returns the `.a` / `.lib` of a staticlib-only crate; the manager pins `sha256`
against the archive and then relinks. `*-windows-gnu` is `Unsupported`.
**Result:** the relinked `static-hello` exports only `_plugwright_entry_v1` (1.2 MB on macOS).

### T27 — Tests and docs for T25–T26 (C2)
Unit tests for every format and foreign images; e2e `static_library_is_relinked_and_loaded`
(in-process + isolated, cache hit) and `prebuilt_formats_are_detected_by_content` (bare `.a`,
`libhello.so.1`-style name, Wasm as native, native as Wasm, text as `.dll`).

### T29 — Build acceleration (C4)
`plugwright_build::accel::Accel` = `[settings.build]`: `toolchain` (`cargo +tc`, `rustc +tc`),
`sccache` (auto/on/off → `RUSTC_WRAPPER`, empty string disables a configured wrapper), `linker`
(auto = mold on Linux / mold / lld), `codegen` (llvm / cranelift / auto = dev builds on nightly;
`-Zcodegen-backend` + `profile.*.codegen-backend`), `threads` (`-Zthreads`), `profile`
(opt-level, codegen-units, lto, debug, strip, incremental; no `panic`). `Accel::plan` → `Plan`
(`wrapper`, `rustflags`, `cargo_args`, `applied`); `key_material()` excludes the wrapper.
Rustflags extend `CARGO_ENCODED_RUSTFLAGS` / `RUSTFLAGS` when set, else `--config
build.rustflags`. `BuildRequest::in_process` keeps Cranelift away from in-process plugins (D22).
cargo now runs in the crate directory (its `rust-toolchain` applies), with absolute
`--manifest-path` / `--target-dir`.
**Result:** e2e `cranelift_is_used_for_isolated_plugins_only` with `nightly` + Cranelift: the
isolated plugin is built by Cranelift and a panic there is a `Crashed` + restart; the in-process
one is LLVM and its panic is caught.

### T30 — `cargo plugwright` (C3)
`crates/cargo-plugwright`, stdlib argument parsing (accepts the `plugwright` argv cargo inserts),
manifest searched upwards. `build [ID...]`, `list` (no loading: lockfile + cache lookup), `call`
(binary-safe output, `-` = stdin), `gc`, `doctor` (rustc/cargo, wasm target, relink linker,
engines, `sandbox::capabilities()`, sccache/mold/Cranelift, the resolved plan for isolated and
in-process builds). Re-execs itself as the runner. `prepare` now persists the lockfile, so
`build` updates it. Name free on crates.io (checked 2026-10-14, like `plugwright`).

### T31–T34 — Macros (C4)
`crates/plugwright-macros` (syn 2 / quote): `wire.rs` classifies parameter and return types once
for both macros (D23). `#[plugin]` emits the impl with helper attributes stripped,
`impl Plugin` (constructor + `match` dispatch + `plugwright.describe` + an unknown-method error
that lists the available names) and `export_plugin!` unless `no_export`. Runtime helpers live in
`plugwright_sdk::__private` (`utf8`, `json`, `to_json`, `Args::{parse, take, finish}`) and
`plugwright::__private` (`encode`, `encode_args`, `utf8`, `decode` → `PluginError::Protocol`), so
generated code stays small. The `macros` feature adds serde_json to plugins: wasm-hello grows from
48 KB to 133 KB in release. `crate = path` handles renamed dependencies. Generated host code avoids
`Ok(x?)` (clippy `needless_question_mark` fires in user crates).
**Result:** e2e `typed_interface_over_native_and_wasm`; `plugwright-sdk/tests/plugin_macro.rs`;
macro unit tests for every compile error. Dedup candidates not taken (closures/constructors/serde
enums fit better) are listed in research.md §9.4.

