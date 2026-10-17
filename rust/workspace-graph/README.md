# workspace-graph

Load a Cargo workspace's direct dependency graph and assert rules over it in
tests, so the architecture's dependency direction cannot drift from the
`Cargo.toml` files.

```rust
use workspace_graph::{Graph, Kind};

#[test]
fn dependencies_point_down() {
    let graph = Graph::load(env!("CARGO_MANIFEST_DIR"), &[Kind::Normal]).unwrap();
    // Only the storage crate may link SQLite.
    graph.assert_only_dependents(&["diesel", "libsqlite3-sys"], &["app-store"]);
    // The core stays free of I/O.
    graph.assert_forbidden("app-core", &["tokio", "reqwest"]);
    // Edges between members only: a leaf has no workspace dependencies.
    let inner = graph.workspace_only();
    inner.assert_exact("app-types", &[]);
    // Bottom layer first; a member may depend on its own layer or a lower one.
    inner.assert_layers(&[&["app-types"], &["app-core", "app-store"], &["app-cli"]]);
}
```

`Graph::load` runs `cargo metadata --no-deps` (through `$CARGO` when a test
runs it) and keeps the declared dependencies of the chosen kinds (`Normal`,
`Dev`, `Build`), external packages included. Optional dependencies always
count: a feature-gated dependency is still one a build can pull in.

Every rule has a `check_*` form that returns `Violations` (one message per
broken edge) and an `assert_*` form that panics with them. Build a synthetic
graph with `FromIterator` to prove a rule fails on a violation:

```rust
let graph: workspace_graph::Graph = [("core", vec!["ffi"]), ("ffi", vec![])].into_iter().collect();
assert!(graph.check_only_dependents(&["ffi"], &[]).is_err());
```

Not published: depend on it by path, `{ path = "../../packages/crates/rust/workspace-graph" }`.
