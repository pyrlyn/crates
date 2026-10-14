# crates — completed tasks

### T20.2. wasm-plugin-host: package layout

Plugin packages on disk, generic over the application's manifest type and home directory. `discover` finds user plugins under `<home>/plugins/<id>/versions/<digest12>/` (following `current`, with `link` winning for a development tree) and project plugins under `<project>/.<app>/plugins/<id>/`. A user plugin shadows a project plugin of the same id. `package_digest` is SHA-256 over the package tree. `layout` stages through a temp directory, swaps `current`/`previous`, writes `link`, and `remove`s only a path that canonicalizes inside `<home>/plugins/`. The manifest parse is the application's [`Manifest`]; this crate checks the id and refuses a wasm path that leaves the package. Tree copy, directory fsync and confined delete stay local until roadmap T16 `dir-ops`.

### T20.1. wasm-plugin-host: engine and worker

First slice of the shared WASM plugin host that cox and scull both build on (creator, 2026-10-07: extract the shared crate, `MIT OR Apache-2.0`). New crate `wasm-plugin-host`: `PluginHost` loads a module with extism (no WASI, no cache, a memory cap and a per-call deadline clamped to host maxima), runs it on a worker thread of its own with a control lane served before an event lane, cancels a call at its deadline through a watchdog, and maps extism failures to a typed `PluginError`. Host functions come from a `HostEnv` trait the application implements; the required export, thread names and limit defaults are options, so nothing names cox. Ported from cox `crates/cox-plugin/src/host.rs` and `error.rs` with no behaviour change. Done when the crate is in the workspace with the ported tests, registered for CI dry-run publish, bump, README and sonar.

Merged in pyrlyn/crates-packages#33 (`77ba2b1`): 10 tests, clippy, fmt, publish dry-run and `cargo +1.98 check` green. `rust.md` lists the crate (listepo/workspace#5, `0af0a5c`).

### T11. Add the required project files

The project was missing `AGENTS.md`, `done.md`, `roadmap.md`, `ideas.md`, and `toolchain.md`. Done means: all files exist, and `toolchain.md` lists the workspace crates per the rulebook (also update `rust.md` then).

Those files are in the workspace root. `toolchain.md` lists `cargo-changed-packages`, `file-backup`, `git-changed-paths`, `path-gates`, and `scoped-check`. `rust.md` names the same five under shared crates. `file-backup` tests already live in `src/lib.rs`; the other four crates already have `tests/`.

### T11. atomic-replace: one atomic file write for every project

Six projects (rtok, ketch, cox, swarfr, aulo, runa) carry about 18 hand-written temp-file-and-rename writes that disagree on fsync, permissions, symlinks and temp naming. New crate `atomic-replace` (std only, edition 2021, Rust 1.86 so ketch can use it): `write(path, bytes)` and `Options { durable, preserve_permissions, follow_symlinks, create_parent, mode }` with `write`/`write_with`, plus `sync_dir`. Ported from rtok `rtok-agent-sdk` `write_atomic`/`link_target` (permissions, Windows read-only bit, dangling links) and ketch `state.rs` `save_path` (fsync file and directory). Done means: the crate is in the workspace with tests, wired into CI dry-run publish, bump options, README and sonar; consumers migrate in their own tasks.

Plan:
1. `atomic-replace/` with Cargo.toml, src/lib.rs, AGENTS.md, README.md, plan/todo/done/roadmap/ideas/toolchain.md, justfile (copy of change-preview's layout).
2. Temp file beside the target via `create_new`, named `.<name>.tmp-<pid>-<nanos>-<counter>`; removed on every error path.
3. Register: workspace members, `ci.yml` dry-run, `bump.yml` package options, root README crate table and token scope, `sonar-project.properties`.
4. Check: `cargo test -p atomic-replace`, clippy `-D warnings`, fmt, `cargo +1.86 check -p atomic-replace`.

Delivered as the `write` module of `fs-kit` instead of a crate of its own (creator's call, 2026-10-12): `fs_kit::write::{atomic, Options, write_with, if_changed, sync_dir}`, ported from branch `t11-atomic-file` with its tests. Consumers migrate in their own tasks.
