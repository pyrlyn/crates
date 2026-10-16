# plugwright — plan

GitHub: https://github.com/pyrlyn/crates/tree/main/plugwright

Rust library that builds, caches, loads, isolates and unloads plugins: native libraries in process
or in a sandboxed runner, static libraries relinked by the host, and Wasm modules on wasmtime,
wasmi or wasmer. Architecture, decisions D1–D26 and the security model: [design.md](design.md).
Research and sources: [research.md](research.md). Finished tasks T01–T42: [done.md](done.md).

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T44 | in progress | P2 | 3 | 0% | Claude / claude-opus-5-5 |
| T46 | in progress | P3 | 2 | 0% | Claude / claude-opus-5-5 |

### T44. File watcher with automatic reload

From idea F6. `PluginManager::reload` exists but has to be called by hand. Add an opt-in
watcher (the `notify` crate) on plugin sources and artifacts that debounces changes and reloads
the affected plugin. A failed rebuild or reload keeps the old instance and reports the error,
never takes the host down. Done: the watcher is off by default, can be stopped, does not
reload twice for one save, and tests cover a change that reloads and a broken change that keeps
the old instance.

### T46. `trybuild` UI tests for macro diagnostics

From idea F9. `#[plugin]` and `#[interface]` diagnostics are checked only by unit tests on
`expand`, which do not prove what a user sees from `rustc`. Add `trybuild` compile-fail tests
with `.stderr` snapshots for the main misuse cases of both macros, plus a compile-pass case.
Done: the tests run in `mise run test` and in CI, and every error path in
`plugwright-macros` that a user can hit has a case.
