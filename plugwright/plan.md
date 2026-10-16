# plugwright — plan

GitHub: https://github.com/pyrlyn/crates/tree/main/plugwright

Rust library that builds, caches, loads, isolates and unloads plugins: native libraries in process
or in a sandboxed runner, static libraries relinked by the host, and Wasm modules on wasmtime,
wasmi or wasmer. Architecture, decisions D1–D24 and the security model: [design.md](design.md).
Research and sources: [research.md](research.md). Finished tasks T01–T42: [done.md](done.md).

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T43 | in progress | P1 | 4 | 0% | Claude / claude-opus-5-5 |
| T46 | in progress | P3 | 2 | 0% | Claude / claude-opus-5-5 |

### T43. Artifact signatures and a trust policy per source

From idea F1. Today the host trusts any artifact it builds or fetches. Add detached signatures
(ed25519, minisign format) for plugin artifacts and a per-source trust policy in the manifest:
which public keys a source accepts, and whether an unsigned artifact is refused, warned about or
allowed. Verification runs before an artifact is mapped or compiled, and a failed or missing
signature is a `PluginError`, never a panic. Done: signing and verification live in the lowest
crate that can own them, the facade only wires the policy from the manifest, `doctor` reports
the policy, and tests cover a valid signature, a wrong key, a tampered artifact and each policy
mode. The signed precompiled-module cache (idea F5) builds on this later.


### T46. `trybuild` UI tests for macro diagnostics

From idea F9. `#[plugin]` and `#[interface]` diagnostics are checked only by unit tests on
`expand`, which do not prove what a user sees from `rustc`. Add `trybuild` compile-fail tests
with `.stderr` snapshots for the main misuse cases of both macros, plus a compile-pass case.
Done: the tests run in `mise run test` and in CI, and every error path in
`plugwright-macros` that a user can hit has a case.
