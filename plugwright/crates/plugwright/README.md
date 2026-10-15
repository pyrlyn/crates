# plugwright

Build, cache, load, isolate and unload Rust plugins: native libraries through `libloading`, and
Wasm modules through a pluggable engine layer (wasmtime and wasmi built in, wasmer in
`plugwright-wasmer`, or your own engine).

* **Build** plugin crates from a path or a git repository with `cargo`, for the host or for
  `wasm32-unknown-unknown`. A `crate-type = ["staticlib"]` crate, or a prebuilt `.a` / `.lib`,
  is relinked into a shared library.
* **Accelerate** builds with sccache, mold / lld, Cranelift and the parallel front end on
  nightly, and profile tuning (`[settings.build]`).
* **Cache** artifacts by a key over sources of every path package, `Cargo.lock`, toolchain,
  target, profile, features, `RUSTFLAGS` and output-changing accelerators; a hit does not run
  cargo at all. Every artifact is verified by sha256 on load. Git branches/tags are pinned to
  SHAs in `plugwright.lock`.
* **Load** through one trait, [`Plugin`], whatever the backend: [`IsolatedPlugin`] (sandboxed
  child process, the default), [`NativePlugin`] (in-process) or [`WasmPlugin`]. Artifact formats
  are detected from their content (ELF, Mach-O incl. fat, PE, `ar`, Wasm), not the file name.
* **Survive** plugin failures: panics are caught, crashes and hangs of isolated plugins kill only
  the child, Wasm guests are bounded by fuel, memory and wall-clock time on every engine; ABI,
  SDK and plugin versions are checked before the first call.
* **Unload** and learn whether the code really left: `unload` returns a [`Release`].
* **Log** builds, cache hits, loads, calls and crashes via `tracing`; guest logs are forwarded
  under the target `plugwright::guest`.

## Host

```toml
# plugwright.toml
[[plugin]]
id = "hello"
source = { path = "plugins/hello" }        # or { git = "https://…", rev = "<sha>" } / { artifact = "libhello.so.1" }
version = "^0.1"
# isolation = "in-process"                 # default "process": sandboxed child, crash-proof
config = { greeting = "Hi" }
[plugin.sandbox]
read = ["data"]                            # extra readable paths; also write = [...], network = true

[[plugin]]
id = "hello-wasm"
kind = "wasm"
engine = "wasmtime"                        # or "wasmi", or any registered engine
source = { path = "plugins/hello" }
limits = { fuel = 50_000_000, max_memory_bytes = 16_777_216, timeout_ms = 1000 }
```

```rust,no_run
use std::path::Path;
use plugwright::PluginManager;

fn main() -> plugwright::Result<()> {
    // Isolated plugins re-exec the current binary as their runner.
    plugwright::maybe_run_runner();

    let (mut mgr, manifest) = PluginManager::from_manifest(Path::new("plugwright.toml"))?;
    for (id, res) in mgr.load_all(manifest.plugins) {
        if let Err(e) = res {
            eprintln!("{id}: {e}"); // one bad plugin does not stop the others
        }
    }
    let out = mgr.call("hello", "greet", b"world")?;
    println!("{}", String::from_utf8_lossy(&out));
    mgr.reload("hello")?; // the old instance keeps serving until the new one is ready
    let release = mgr.unload("hello")?; // unmapped / still-mapped / process-exited / freed
    println!("{release}");
    Ok(())
}
```

Reloading on change is opt-in. `watch` watches the plugins loaded at that moment (an artifact
file, or a path source's crate minus `target/`); a save is debounced into one reload, an output
with the same bytes reloads nothing, and a broken change keeps the old instance:

```rust,no_run
use std::sync::{Arc, Mutex};
use std::time::Duration;
use plugwright::{PluginManager, WatchOptions};

fn run(mut mgr: PluginManager) -> plugwright::Result<()> {
    let mut watcher = mgr.watch(&WatchOptions::default())?;
    // Either pump it from the host's own loop…
    if watcher.wait(Duration::from_millis(100)) {
        for (id, res) in mgr.reload_changed(&mut watcher) {
            if let Err(e) = res {
                eprintln!("{id}: {e}"); // still serving the old instance
            }
        }
    }
    // …or let a thread do it; stopping (or dropping) the reloader stops watching.
    let mgr = Arc::new(Mutex::new(mgr));
    let reloader = watcher.spawn(mgr.clone(), |id, res| eprintln!("{id} reloaded: {res:?}"))?;
    reloader.stop();
    Ok(())
}
```

## Plugin

One source builds both a native library and a Wasm module:

```toml
[lib]
crate-type = ["cdylib"]                    # or ["staticlib"]: the host relinks it

[dependencies]
plugwright-sdk = "0.1"
```

```rust,ignore
use plugwright_sdk::{log, plugin};

#[derive(serde::Deserialize)]
struct Config { greeting: String }      // the manifest's `config` table

struct Hello { greeting: String }

#[plugin]
impl Hello {
    fn new(config: Config) -> Self {    // or `-> Result<Self, E>`; no `new` → `Default`
        Hello { greeting: config.greeting }
    }

    fn greet(&self, name: &str) -> String {
        log::info!("greeting {name}");
        format!("{}, {name}!", self.greeting)
    }

    fn add(&self, a: i64, b: i64) -> Result<i64, &'static str> {
        a.checked_add(b).ok_or("overflow")
    }

    #[plugwright(skip)]
    fn helper(&self) {}                 // not exported
}
```

`#[plugin]` exports every `&self`/`&mut self` method (`#[plugwright(name = "...")]` renames
one), implements `Plugin`, calls `export_plugin!` and answers `plugwright.describe` with the
method list (`cargo plugwright methods ID`). Arguments and results are encoded by one rule,
shared with the host's `#[interface]`:

| signature | wire |
|---|---|
| no parameters | input ignored |
| one `&str` / `String` | raw UTF-8 |
| one `&[u8]` / `Vec<u8>` | raw bytes |
| one parameter of any other type | JSON |
| two or more parameters | JSON array `[2, 40]` or object `{"a": 2, "b": 40}` |
| returns `()` / `String`/`&str` / `Vec<u8>`/`&[u8]` / other | empty / UTF-8 / bytes / JSON |
| returns `Result<T, E>` | `T` as above; `E: Display` becomes the plugin error |

The rule is syntactic — the macro sees tokens, so an alias of `String` travels as JSON. A
missing argument reads as `null` (so `Option<T>` may be omitted); unknown named arguments are
errors. The `macros` feature (default) pulls in `serde_json`, about 85 KB in a release Wasm
module; with `default-features = false` implement `plugwright_sdk::Plugin` by hand and call
`export_plugin!(Type)`.

## Typed calls

```rust,ignore
#[plugwright::interface]
trait Hello {
    fn greet(&mut self, name: &str) -> plugwright::Result<String>;
    fn add(&mut self, a: i64, b: i64) -> plugwright::Result<i64>;
    fn shout(&mut self, name: &str) -> plugwright::Result<String> {   // has a body: runs on the host
        Ok(self.greet(name)?.to_uppercase())
    }
}

let mut hello = HelloClient::new(manager.bind("hello")?);
assert_eq!(hello.add(2, 40)?, 42);
```

The trait is kept as written; `HelloClient<I>` implements it for any `I: Invoke` — a
`Bound` from `PluginManager::bind` (logging, timeouts and restarts included), a
`&mut dyn Plugin` or a `Box<dyn Plugin>`. Malformed output is a `PluginError::Protocol`.

## Artifact formats

| file | accepted as | notes |
|---|---|---|
| `.so`, `.so.N`, any name | native | ELF shared object for the host architecture (Linux, BSD) |
| `.dylib`, `.bundle` | native | Mach-O dylib/bundle, or a fat binary containing the host slice |
| `.dll` | native | PE image with the DLL flag for the host machine |
| `.a`, `.lib` | native | `ar` archive, relinked into a shared library (below) |
| `.wasm` | wasm | core module; components are reported as unsupported |

The kind in the manifest must match the content: a Wasm module given as `kind = "native"`, an
executable, a foreign architecture or a text file are `NotAPlugin` errors that name the
reason.

**Static libraries** are linked with the platform linker found the way build scripts find it
(`CC`, `PATH`, Visual Studio discovery): the entry symbol is forced in (`-u` / `/INCLUDE`) and
is the only export (version script / `-exported_symbol` / `/EXPORT`), so only the archive
members the plugin needs are pulled. The Rust standard library's native dependencies come from
`rustc --print native-static-libs`; add others per plugin with `link_args = ["-lz"]`. The
result is cached under a key over the archive's sha256, the triple, the linker and its
arguments. MinGW (`*-windows-gnu`) hosts are not supported.

## Build acceleration

```toml
[settings.build]
toolchain = "nightly"     # rustup toolchain for plugin builds (cargo +nightly)
sccache = "auto"          # auto | on | off — RUSTC_WRAPPER; never part of the cache key
linker = "auto"           # auto (mold on Linux if installed) | default | mold | lld
codegen = "auto"          # llvm (default) | cranelift | auto (Cranelift for dev builds on nightly)
threads = 8               # -Zthreads, nightly only
profile = { opt_level = "2", codegen_units = 16, lto = "off", debug = 0, strip = "symbols", incremental = false }
```

* Accelerators that can change the bytes (linker, backend, threads, profile) are part of the
  cache key; sccache only replays rustc output and is not.
* An accelerator asked for explicitly (`on`, `mold`, `lld`, `cranelift`, `threads`) and not
  available is a build error; `auto` skips what is missing.
* **Cranelift does not unwind**: a panic aborts the process. plugwright therefore builds
  in-process plugins with LLVM even when Cranelift is requested, and uses Cranelift only for
  isolated plugins, where a panic becomes a crash the host survives and restarts. It does not
  target Wasm either.
* `panic` cannot be tuned: plugins are always built with `panic = "unwind"`.
* Extra rustflags are appended to `RUSTFLAGS` / `CARGO_ENCODED_RUSTFLAGS` when set, otherwise to
  `build.rustflags` via `--config` (a `target.<triple>.rustflags` in your cargo config would
  take precedence over it).

## `cargo plugwright`

```text
cargo plugwright [--manifest PATH] [-v] build [ID...]
cargo plugwright [--manifest PATH] [-v] list
cargo plugwright [--manifest PATH] [-v] call ID METHOD [INPUT|-]
cargo plugwright [--manifest PATH] [-v] gc
cargo plugwright [--manifest PATH] [-v] doctor
```

`doctor` checks the toolchain, the Wasm target, the relink linker, Wasm engines, what the
sandbox enforces on this OS, sccache / mold / Cranelift, and what `[settings.build]` resolves to.

## Security model

| threat | mitigation | residual risk |
|---|---|---|
| malicious plugin source (`build.rs`, proc-macros) | build only manifests you trust; `--locked` / `--offline`; SHAs pinned in the lockfile | build-time code runs with your privileges |
| tampered cached artifact | sha256 recorded at build and checked on every load; `sha256` pin for prebuilt artifacts (checked before relinking) | attacker who can write both cache and lockfile |
| native crash, abort, hang | default `isolation = "process"`: runner with cleared env, no core dumps, call timeout, restart budget | — |
| native plugin reaching out | runner sandboxes itself before `dlopen`: no writes, network or new processes, reads allow-listed (macOS Seatbelt; Linux `no_new_privs` + Landlock + seccomp) | Windows: low integrity + job object only, reads and network stay open; Linux without Landlock: file access open (reported, `require = true` makes it an error) |
| native panic | SDK wraps every entry in `catch_unwind`; builder forces `panic = "unwind"`; Cranelift never used in-process | plugins not built with the SDK |
| ABI confusion | `abi_version` read first, `struct_size`, every slot checked, unknown status → error | — |
| Wasm escape / DoS | no WASI, one import (`log`), bounds-checked reads, fuel, memory cap, wall-clock timeout (wasmtime epochs, wasmi fuel slices, wasmer host interrupt), trapped instances discarded | engine bugs; wasmer on non-unix abandons a hung worker thread |
| git URL tricks | scheme allowlist, `protocol.ext.allow=never`, no hooks, no prompts, no submodules | — |
| wrong file handed in | format detected from magic bytes; kind, architecture and image type checked before loading | — |

**In-process native plugins are trusted code.** Loading a library runs its initialisers in your
process; keep the default `isolation = "process"` or use Wasm for anything you do not fully
trust.

## Platform notes

* **Unloading in-process plugins is verified, not guaranteed.** After `dlclose`, plugwright
  probes whether the image is still mapped (`RTLD_NOLOAD` / `GetModuleHandleExW`) and `unload`
  returns `Release::Unmapped` or `Release::StillMapped`; `PluginManager::leaked_images` counts
  the latter. macOS never unmaps images with thread-local variables, which includes every Rust
  `cdylib`; glibc keeps libraries with TLS destructors. Every load uses a fresh shadow copy, so
  `reload` always gets the new code. Isolated plugins return `ProcessExited`, Wasm `Freed`.
* **Windows** locks loaded DLLs; the shadow copy keeps the cached artifact replaceable. The
  runner moves plugin stdout (Win32 handle and CRT fd 1) to stderr, so stray output cannot
  corrupt the protocol.
* **wasmer** cannot share a dependency graph with wasmtime 49 (they pin different cranelift
  versions), so it ships as the separate crate `plugwright-wasmer`. Use it with
  `plugwright = { default-features = false, features = ["wasmi"] }` and
  `PluginManager::register_engine`. Its wall-clock timeout uses `SIGUSR1` on unix (feature
  `host-interrupt`, on by default).
