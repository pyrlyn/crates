# plugwright — plan

GitHub: https://github.com/pyrlyn/crates/tree/main/rust/plugwright

Rust library that builds, caches, loads, isolates and unloads plugins: native libraries in process
or in a sandboxed runner, static libraries relinked by the host, and Wasm modules on wasmtime,
wasmi or wasmer. Architecture, decisions D1–D27 and the security model: [design.md](design.md).
Research and sources: [research.md](research.md). Finished tasks T01–T47: [done.md](done.md).

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T49 | todo | P2 | 4 | 0% | |

### T49. Windows AppContainer for the sandboxed runner

On Windows the runner sandbox is a low integrity level plus a job object
(`crates/plugwright-sandbox/src/windows.rs`). Low integrity cannot open a single
medium-integrity path, so `SandboxPolicy::write` is reported as not enforced and every write is
denied. Reads and network stay open (design.md, security table; formerly idea F2). The e2e test
`sandbox_denies_writes_network_exec_and_foreign_reads` pins that gap today.

Done: the parent starts the runner in an AppContainer (`CreateProcess` with
`SECURITY_CAPABILITIES` and a per-plugin profile). It grants the container SID access to the
read and write allow-listed paths, and adds the `internetClient` capability only when
`network = true`. On Windows the sandbox report lists no read, network or write allow-list gap.
The e2e test asserts the same allow and deny results on Windows as on macOS and Linux: a write
to an allowed directory succeeds; foreign reads, writes, connects and spawns are denied. Unload
removes the profile and the grants. design.md and research.md record the design with sources.
