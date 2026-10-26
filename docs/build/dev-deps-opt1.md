# Dev profile: optimized dependencies

## How it works
`[profile.dev.package."*"] opt-level = 1` compiles every dependency (not workspace
crates) with light optimization. Dependencies are compiled once and then cached, so
incremental builds of our own code are unaffected, while tests and dev binaries run
faster and come out smaller. Workspace crates stay at `opt-level = 0` for fast rebuilds.

## Enable
On for every `dev` build (`cargo build`, `cargo test`, `cargo run`).

## Disable
Remove `opt-level = 1` from `[profile.dev.package."*"]` (there is no environment
variable override for the `"*"` package selector).

## Test
`cargo build` and `cargo test` must succeed.

## Limitations
- A cold build (new toolchain, `Cargo.lock` change, empty cache) is roughly 10-20% slower.
- Stepping into dependency code in a debugger is less precise (inlined, reordered).
- Debug assertions and overflow checks still apply to dependencies (they are separate
  from `opt-level`).
- In this repo `[profile.dev.package.sha2] opt-level = 3` still wins for `sha2`
  (a named package overrides the `"*"` selector).
