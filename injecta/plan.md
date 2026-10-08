# injecta

Compile-time dependency injection for Rust: one `container!` declaration, `#[derive(Injectable)]`, no runtime registry. Lives in crates-packages (`injecta/`, `injecta-macros/`); not published yet.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T1 | todo | P1 | 3 | 0% | |
| T2 | todo | P1 | 2 | 0% | |
| T3 | todo | P2 | 3 | 0% | |
| T4 | todo | P2 | 2 | 0% | |

### T1. Reusable modules

Let a library crate export a group of entries (`injecta::module!`) that a container includes, so feature crates ship their own wiring. Done when two containers include one module and the compile-fail suite covers a duplicate entry coming from a module.

### T2. Runtime guard against factory cycles

Factories are opaque to the compile-time cycle check; a singleton whose factory resolves itself re-enters `OnceLock::get_or_init`. Add a debug-build re-entrancy guard that reports the provider name instead. Done when a test shows the message and release builds keep the current resolve cost.

### T3. Nested scopes

Allow `scope Session in Request { .. }` so a scope can be opened from another scope. Done when a three-level example resolves values from all levels and the captive-dependency check covers each level.

### T4. Async-ready hook

An optional `ready()` step that runs async initializers of selected singletons up front (the Dart design's `ready(timeout)`), still without async in `resolve`. Done when an example initializes a pool with tokio before serving.
