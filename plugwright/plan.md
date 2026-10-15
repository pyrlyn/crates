# plugwright — plan

GitHub: https://github.com/pyrlyn/crates/tree/main/plugwright

Rust library that builds, caches, loads, isolates and unloads plugins: native libraries in process
or in a sandboxed runner, static libraries relinked by the host, and Wasm modules on wasmtime,
wasmi or wasmer. Architecture, decisions D1–D25 and the security model: [design.md](design.md).
Research and sources: [research.md](research.md). Finished tasks T01–T42: [done.md](done.md).

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T44 | in progress | P2 | 3 | 0% | Claude / claude-opus-5-5 |
| T45 | in progress | P2 | 4 | 0% | Claude / claude-opus-5-5 |
| T46 | in progress | P3 | 2 | 0% | Claude / claude-opus-5-5 |

### T44. File watcher with automatic reload

From idea F6. `PluginManager::reload` exists but has to be called by hand. Add an opt-in
watcher (the `notify` crate) on plugin sources and artifacts that debounces changes and reloads
the affected plugin. A failed rebuild or reload keeps the old instance and reports the error,
never takes the host down. Done: the watcher is off by default, can be stopped, does not
reload twice for one save, and tests cover a change that reloads and a broken change that keeps
the old instance.

### T45. Shared-memory transport for large payloads in isolated mode

From idea F8. Isolated plugins talk to the runner over stdin/stdout frames, so every large
payload is copied through a pipe. Add a shared-memory path for payloads above a threshold,
with the pipe kept as the control channel and the fallback. The runner is untrusted: offsets,
lengths and sizes it reports are validated before use, and a runner that dies mid-call is a
`PluginError`. Done: works on macOS, Linux and Windows (Windows at least cross-linted),
falls back to the pipe when shared memory is unavailable, and a perf case in
`examples/host/benches/perf.rs` shows the gain on a large payload.

### T46. `trybuild` UI tests for macro diagnostics

From idea F9. `#[plugin]` and `#[interface]` diagnostics are checked only by unit tests on
`expand`, which do not prove what a user sees from `rustc`. Add `trybuild` compile-fail tests
with `.stderr` snapshots for the main misuse cases of both macros, plus a compile-pass case.
Done: the tests run in `mise run test` and in CI, and every error path in
`plugwright-macros` that a user can hit has a case.
