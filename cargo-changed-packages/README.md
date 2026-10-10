# cargo-changed-packages

Workspace packages a set of changed paths affects, including reverse dependencies.

Built on `guppy` and `determinator`. The package graph comes from `cargo metadata`
run in the workspace root. Changed paths are relative to that root, as produced by
`git-changed-paths`. A change to `Cargo.toml`, `Cargo.lock`, `.cargo/config*`,
`rust-toolchain*` or `build.rs` returns `all: true`, because the old package graph
would need a checkout of the base ref.

```rust
use cargo_changed_packages::affected;
use std::path::{Path, PathBuf};

let result = affected(Path::new("."), &[PathBuf::from("b/src/lib.rs")])?;
if let Some(filter) = result.nextest_filter() {
    println!("cargo nextest run -E '{filter}'");
}
```

For several calls on one workspace use `Resolver`: it runs `cargo metadata` lazily, once, and
can keep the JSON in a cache directory. The key hashes `Cargo.lock`, every `Cargo.toml`
under the root, `.cargo/config*`, `rust-toolchain*` and `cargo --version`, and re-hashes
path dependencies outside the root; any doubt recomputes. `cache_status()` reports `Hit`,
`Miss`, `MissNotStored` or `Disabled`. An empty change set, a build-altering path or an
unmappable path never starts cargo.

```rust
use cargo_changed_packages::Resolver;
use std::path::{Path, PathBuf};

let resolver = Resolver::new(Path::new(".")).cache_dir("/tmp/cache");
let a = resolver.affected(&[PathBuf::from("a/src/lib.rs")])?;
let b = resolver.affected(&[PathBuf::from("b/src/lib.rs")])?; // same graph, no second run
```
