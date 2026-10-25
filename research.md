# Research

<a id="buildx"></a>
## buildx: keep only the last builds in a Cargo `target/`

Background for T50–T54. Goal: after every build, prune a Cargo target dir so only the units of
the last three builds remain (`deps/` first, then fingerprints, build-script output and
incremental state). All sources checked 2026-10-21 unless a version is given.

### Existing tools

None keeps "the last N builds". Versions and dates are from the crates.io API.

| Tool | Version, release | How it decides | Source |
| --- | --- | --- | --- |
| cargo-sweep | 0.8.0, 2025-10-11 | Per `.fingerprint/<name>-<hash>` entry: atime (`--time`, `--maxsize`, `--file` stamp) or the rustc recorded in the fingerprint (`--installed`, `--toolchains`); deletes files whose `-<hash>` suffix is not kept. README says it is currently unmaintained | https://crates.io/api/v1/crates/cargo-sweep, https://github.com/holmgr/cargo-sweep/blob/master/README.md, https://github.com/holmgr/cargo-sweep/blob/master/src/fingerprint.rs |
| kondo | 0.9.0, 2026-01-23 | Removes whole artifact dirs per detected project | https://crates.io/api/v1/crates/kondo |
| cargo-trim | 0.18.0, 2026-10-15 | `cargo trim target` cleans whole target dirs of stale projects | https://crates.io/api/v1/crates/cargo-trim |
| cargo-wipe | 0.4.0, 2025-11-16 | Finds and wipes whole `target` dirs | https://crates.io/api/v1/crates/cargo-wipe |
| cargo-clean-all | 0.6.5, 2026-08-30 | `--keep-days` / `--keep-size` pick projects, then cleans whole targets | https://crates.io/api/v1/crates/cargo-clean-all |
| cargo-prune | 0.2.0, 2019-02-15 | Duplicate libs in `deps/` by an mtime window; unmaintained | https://crates.io/api/v1/crates/cargo-prune |
| cargo-cache | 0.8.3, 2022-09-11 | `$CARGO_HOME` only; reports on `target/`, does not clean it | https://crates.io/api/v1/crates/cargo-cache |

swarfr (formerly dunnage, https://github.com/pyrlyn/swarfr, v0.1.1) already carries this as
roadmap R1 "Unit-level prune" (`roadmap.md`), with the source of truth being
`--message-format=json`. Its `DESIGN.md` measured age-based sweeps at ~50 MB of 14 GB.

### Cargo itself

- `cargo clean` has `-p`, `--workspace`, `--dry-run`, `--doc`, `--release`, `--profile`,
  `--target-dir`, `--target`; no age, size or keep flag (cargo 0.100 / Rust 1.99):
  https://doc.rust-lang.org/cargo/commands/cargo-clean.html
- `-Zgc` / `cargo clean gc` cover only the global cache in `CARGO_HOME`, not `target/`; automatic
  global-cache GC is stable since 1.88: https://doc.rust-lang.org/cargo/reference/unstable.html#gc,
  https://github.com/rust-lang/cargo/issues/12633. No target-dir GC exists on stable.
- `build.build-dir` is stable since 1.91; intermediate artifacts (`deps/`, `build/`,
  `incremental/`, fingerprints) live there: https://doc.rust-lang.org/cargo/reference/config.html#buildbuild-dir,
  https://doc.rust-lang.org/cargo/reference/build-cache.html
- New build-dir layout: per unit `build/<pkg>/<hash>/{out,fingerprint,run}`, no `deps/` and no
  `.fingerprint/`; `incremental/` unchanged. Described in
  https://github.com/rust-lang/cargo/blob/master/src/compiler/layout.rs; enabled by
  default in https://github.com/rust-lang/cargo/pull/16807 (merged 2026-07-06). Observed: stable
  1.99.1 still uses the legacy layout, nightly 1.101.0 (2026-09-25) the new one. The first stable
  release with the new layout is **unverified**. buildx must handle both.
- No post-build hook: https://github.com/rust-lang/cargo/issues/545 (open since 2014). `[alias]`
  can only name cargo subcommands: https://doc.rust-lang.org/cargo/reference/config.html#alias

### Legacy layout (cargo 1.99.1)

From the `layout.rs` module docs, confirmed on a test crate with a build script:

- `deps/lib<crate>-<META>.{rlib,rmeta,d}`, binaries, test executables and `.o` files carry the
  unit's `-<META>` suffix; dynamic libraries are the exception.
- `.fingerprint/<pkg>-<META>/` uses the same hash as the unit's files in `deps/`.
- A build script has two units: `build/<pkg>-<H1>/` (the compiled script) and `build/<pkg>-<H2>/`
  (its run: `out/`, `output`, `stderr`), matching `.fingerprint/<pkg>-<H2>/run-build-script-*`.
- `incremental/<crate>-<hash>/` uses a hash that cannot be mapped from file names; it can only be
  matched by crate name.
- Final outputs are uplifted into `target/<profile>/` without a hash; they are not swept.

### JSON messages

https://doc.rust-lang.org/cargo/reference/external-tools.html (Artifact messages):

- `compiler-artifact` is emitted for every unit, with `"fresh": true` when rustc did not run, so
  one build's full unit set is known from its message stream. Useful fields: `package_id`,
  `target.{kind,name}`, `profile`, `features`, `filenames`, `executable`.
- `filenames` may list the uplifted path instead of the `deps/` copy and lists no `.d`,
  fingerprint or incremental paths, so the hash is taken from the file name and matched across
  directories.
- `build-script-executed` is emitted even when the script did not run; its `out_dir` gives the run
  unit's hash.

### Name

- crates.io: `buildx` 0.1.1 and `cargo-buildx` 0.1.1 are name placeholders; `buildx-cli` does not
  exist: https://crates.io/api/v1/crates/buildx, https://crates.io/api/v1/crates/cargo-buildx,
  https://crates.io/api/v1/crates/buildx-cli
- Docker ships `docker buildx`, whose `install` subcommand makes it the default builder: name
  clash **unverified** from a primary source (docs.docker.com reference page not fetched). The
  creator accepted the clash.

### sccache

The cache lives outside `target/` (default `~/Library/Caches/Mozilla.sccache` on macOS,
`~/.cache/sccache` on Linux, `SCCACHE_DIR` to override), so pruning `target/` never touches it.
Incrementally compiled crates are not cached. https://github.com/mozilla/sccache/blob/main/docs/Local.md,
https://github.com/mozilla/sccache/blob/main/README.md (v0.18.0, 2026-09-14).
