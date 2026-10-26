# lockstep-clippy

[dylint](https://github.com/trailofbits/dylint) lints that keep logging and concurrency inside
Lockstep (`rust/lockstep`). Not published (`publish = false`), not a member of the repository
workspace.

| Lint | Default | Flags |
| --- | --- | --- |
| `lockstep_logging_only` | warn | `log`, `tracing` (with `tracing_core`, `tracing_attributes`, `tracing_subscriber`), `env_logger`, `slog` (with `slog_async`, `slog_scope`, `slog_stdlog`, `slog_term`); std's `print!`, `println!`, `eprint!`, `eprintln!`, `dbg!` |
| `lockstep_explicit_concurrency` | warn | `std::thread::{spawn, scope, Builder}`, `std::sync::{Mutex, RwLock, Condvar, Barrier}`, crossbeam's channels, scoped threads and sync types, rayon (including parallel iterators), parking_lot, spin, `tokio::sync::{Mutex, RwLock, Semaphore, Barrier, Notify}` |

## How it works

Both lints are late lint passes, so they see names as the compiler resolved them, not as written:

- **Paths** (expressions, types, patterns, trait bounds, `impl Trait for`) are matched on the crate
  and module the item is *defined* in. Re-exports are covered: `tracing::Level` is defined in
  `tracing_core`, `crossbeam::scope` in `crossbeam_utils`. `use` items are not reported, only the
  places that use what they import.
- **Macros** are found through the expansion data: std's print macros and `dbg!` by diagnostic
  item (a local macro called `println` is not one), the others by the crate that defines them. A
  call is reported once, at the outermost call the user wrote; calls generated inside another
  crate's macro are not.
- **Methods** of banned traits (rayon's `par_iter`, `into_par_iter`, `par_bridge`) are reported
  once per chain. Inherent methods are not: their type is reported where it is named.

`lockstep_logging_only` treats the crates in `allowed_logging_crates` (default `lockstep` and
`logstep`) as the logging facade:

- their own items are fine;
- a banned item named through them is fine, for a facade that re-exports one:
  `logstep::log::info!(..)`, `logstep::Level`, or `use logstep::Level;` then `Level::INFO`;
- the lint does not run inside them.

`lockstep_explicit_concurrency` does not run inside the crate named `lockstep`, which implements
the locks and threads it points to.

`docs/sample-run.txt` is a run against a small crate: default config, a `dylint.toml` that adds
`log` to the facade, and an invalid entry.

## Enable

Install the tools once (same version as `dylint_linting` in `Cargo.toml`):

```sh
cargo install cargo-dylint@6.2.0 dylint-link@6.2.0 --locked
```

In the workspace to lint, name the library in `Cargo.toml` (or `dylint.toml`):

```toml
[workspace.metadata.dylint]
libraries = [{ git = "https://github.com/pyrlyn/crates", pattern = "rust/lockstep-clippy" }]
```

and run:

```sh
cargo dylint --all
```

From a local checkout, without the metadata: `cargo dylint --path <crates>/rust/lockstep-clippy --all`.

## Disable

For a function, statement or module, guarded so builds without dylint see no unknown lint:

```rust
#[cfg_attr(dylint_lib = "lockstep_clippy", allow(lockstep_logging_only))]
fn print_usage() {
    println!("usage: ...");
}
```

`#![cfg_attr(dylint_lib = "lockstep_clippy", allow(lockstep_explicit_concurrency))]` at the top of
`lib.rs` or `main.rs` covers a crate. rustc checks `cfg` names, so declare `dylint_lib` in the
linted package's `Cargo.toml`:

```toml
[lints.rust.unexpected_cfgs]
level = "warn"
check-cfg = ["cfg(dylint_lib, values(any()))"]
```

For one run, turn a lint off (or into an error) from the command line:

```sh
DYLINT_RUSTFLAGS="-A lockstep_logging_only" cargo dylint --all
DYLINT_RUSTFLAGS="-D lockstep_explicit_concurrency" cargo dylint --all
```

## Config

`dylint.toml` in the root of the workspace being linted, table named after the library:

```toml
[lockstep_clippy]
# Crate names that make up the logging facade; `-` and `_` are the same. Default:
allowed_logging_crates = ["lockstep", "logstep"]
```

The list replaces the default, so keep `lockstep` and `logstep` in it if you still want them. An
unknown key or an entry that is not a crate name stops the build with an error.

## Test

Commands run from this directory. `RUSTUP_TOOLCHAIN` overrides `rust-toolchain`, so unset it if
your shell sets it (see Limitations):

```sh
env -u RUSTUP_TOOLCHAIN cargo build
env -u RUSTUP_TOOLCHAIN cargo test
```

`cargo-dylint` and `dylint-link` must be on `PATH` (see Enable). `tests/ui.rs` runs
[dylint_testing](https://crates.io/crates/dylint_testing) UI tests and compares warnings with the
`.stderr` file next to each case:

- `ui/` holds the std-only cases: one file per lint, plus `lockstep.rs` and `logstep.rs`, whose
  crate names (taken from the file name) are on the default allow list;
- `examples/` holds the cases that need the real `log`, `tracing`, `rayon`, ... crates
  (dev-dependencies), so paths resolve to their definitions. `allowed_logging_crates.rs` runs with
  `allowed_logging_crates = ["tracing"]`.

dylint_testing has no bless mode. On a mismatch compiletest prints `Actual stderr saved to ...`;
check that file and copy it over the `.stderr`. `.github/workflows/lockstep-clippy.yml` runs the
tests in CI when this directory changes.

## Limitations

- **Pinned nightly.** dylint libraries link rustc's private crates, so this builds only on the
  nightly `rust-toolchain` names (`nightly-2026-10-01`, with `rustc-dev` and `llvm-tools-preview`),
  the one `clippy_utils =0.1.101` is built against. Moving to a newer nightly means moving both
  together and fixing whatever rustc's internal API changed. The workspace being linted can stay
  on stable: cargo-dylint builds a driver for the library's nightly and checks with it.
- **`RUSTUP_TOOLCHAIN` overrides the pin.** If it is set (for example to the workspace's stable
  release), cargo in this directory uses that toolchain instead, and the build fails (on stable:
  `` `#![feature]` may not be used on the stable release channel ``). Unset it for every command
  here.
- **Not in the workspace.** The repository workspace is on stable and lists this directory under
  `exclude`; `cargo test --workspace` at the root does not build it. It has its own `Cargo.lock`.
- Re-exports through a facade are recognised when the path goes through it
  (`logstep::log::info!`) or is imported from it with `use`, crate-wide. A `use` imports for the
  whole crate: after `use logstep::log::info;`, a direct `log::info!` in the same crate is not
  reported either. A macro reached through a `use` of a re-exported crate (`use logstep::log;` then
  `log::info!`) is still reported, because rustc does not keep the macro call's path.
- Only names are checked. Values whose type is banned but is never named (`let (tx, rx) =
  crossbeam::channel::unbounded()` then `rx.recv()`) are reported once, where the banned item is
  named, not at every method call.
- The crate skips match by crate name: any crate named `lockstep` skips the concurrency lint, any
  crate on the allow list skips the logging lint.
- CI runs on Linux only.
