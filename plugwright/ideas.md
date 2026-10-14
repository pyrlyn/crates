# plugwright — ideas

Not approved yet. Nothing here moves into [roadmap.md](roadmap.md) or [plan.md](plan.md) without
the creator's approval.

* F1 — artifact signatures (ed25519 / minisign) and a trust policy per source.
* F2 — Windows AppContainer for the runner (created by the parent at `CreateProcess`), closing reads and network.
* F3 — sandboxed builds (container / `bwrap`), vendored deps.
* F4 — component model + WIT typed interfaces for wasmtime.
* F5 — precompiled module cache (`Module::serialize`), signed because `deserialize` is `unsafe`.
* F6 — file watcher → automatic `reload`.
* F7 — async API (`call_async`, wasmtime async + fuel yielding).
* F8 — shared-memory transport for large payloads in isolated mode.
* F9 — `trybuild` UI tests for the macros' diagnostics (now unit tests on `expand`).
