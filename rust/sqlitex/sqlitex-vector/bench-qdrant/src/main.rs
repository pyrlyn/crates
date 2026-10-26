//! `vecbench` with the qdrant-edge engine: the driver source is shared with
//! `sqlitex-vector`'s own `vecbench` binary, only the `qdrant` cfg differs.

#[path = "../../src/bin/vecbench.rs"]
mod vecbench;

fn main() {
    vecbench::main();
}
