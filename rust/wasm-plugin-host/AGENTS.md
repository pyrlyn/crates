# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The WebAssembly plugin host shared by cox and scull: extism under a memory cap,
one worker thread per plugin, per-call deadlines, and a control lane served
before an event lane. It knows nothing about what plugins are for; each
application keeps its own manifest, exports and host functions and plugs them in
through `HostEnv` and `Options`. The on-disk package layout (discovery,
staging, `current`/`previous`, `link`, and `remove` confined to the plugins
root) is generic over the application's manifest type and home directory.

## Rules

- The only crate here that links extism and wasmtime. Their versions follow cox's
  pins (`extism 1.30.0`, `wasmtime 43`); bump them only with the creator, and both
  together, or a second wasmtime resolves that extism never sees.
- WASI stays off until extism ships wasmtime >= 48 (RUSTSEC-2026-0269).
- Licence `MIT OR Apache-2.0`, so a third party can embed it in a closed host.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
