# cargo-changed-packages — completed tasks

### T1. Affected workspace packages via determinator

`affected(workspace_root, changed)` returns the workspace packages whose files changed
plus their reverse dependencies, from a `guppy` package graph and `determinator`. Paths
that alter the build or the graph (`Cargo.toml`, `Cargo.lock`, `.cargo/config*`,
`rust-toolchain*`, `build.rs`) or cannot be mapped select everything (`all: true`).
`Affected::nextest_filter` renders a nextest filterset.

### T2. `Resolver`: one graph load, optional on-disk cache

`Resolver` loads the package graph at most once and only when a call needs it (an empty
change set never does). The `cargo metadata` JSON can be cached by a content hash of the
lock file, manifests, cargo config, toolchain files and `cargo --version`. `Error::Cargo`
replaces the guppy command error for a failed or missing `cargo`.
