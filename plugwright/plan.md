# plugwright — plan

GitHub: https://github.com/pyrlyn/crates/tree/main/plugwright

Rust library that builds, caches, loads, isolates and unloads plugins: native libraries in process
or in a sandboxed runner, static libraries relinked by the host, and Wasm modules on wasmtime,
wasmi or wasmer. Architecture, decisions D1–D24 and the security model: [design.md](design.md).
Research and sources: [research.md](research.md). Finished tasks T01–T42: [done.md](done.md).

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T43 | in progress | P1 | 4 | 0% | Claude / claude-opus-5-5 |
| T44 | in progress | P2 | 3 | 0% | Claude / claude-opus-5-5 |

### T43. Artifact signatures and a trust policy per source

From idea F1. Today the host trusts any artifact it builds or fetches. Add detached signatures
(ed25519, minisign format) for plugin artifacts and a per-source trust policy in the manifest:
which public keys a source accepts, and whether an unsigned artifact is refused, warned about or
allowed. Verification runs before an artifact is mapped or compiled, and a failed or missing
signature is a `PluginError`, never a panic. Done: signing and verification live in the lowest
crate that can own them, the facade only wires the policy from the manifest, `doctor` reports
the policy, and tests cover a valid signature, a wrong key, a tampered artifact and each policy
mode. The signed precompiled-module cache (idea F5) builds on this later.

### T44. File watcher with automatic reload

From idea F6. `PluginManager::reload` exists but has to be called by hand. Add an opt-in
watcher (the `notify` crate) on plugin sources and artifacts that debounces changes and reloads
the affected plugin. A failed rebuild or reload keeps the old instance and reports the error,
never takes the host down. Done: the watcher is off by default, can be stopped, does not
reload twice for one save, and tests cover a change that reloads and a broken change that keeps
the old instance.
