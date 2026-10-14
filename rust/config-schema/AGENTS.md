# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The helpers behind "our configs have a schema and one module owns config I/O",
generic over the caller's config type (`Serialize + Deserialize + JsonSchema`):

- `schema`: the JSON Schema of the types, and the stale-schema check a test calls.
- `layers`, `env`: `figment` layering with named layers, `_`-safe environment
  variables resolved against the defaults' key tree, and per-key provenance.
- `edit`, `reveal`: `toml_edit` edits that keep comments and layout, validated
  against the types for our own files, key-only for foreign ones.

No app-specific key, layer name or env prefix belongs here. The caller owns its
types, its layer order and its guards.

## Rules

- `unwrap`, `expect` and `panic!` are denied outside tests (`clippy.toml` allows
  them in tests). Tests never call `set_var`: use `figment::Jail` or inject
  variables with `EnvLayer::with_vars`.
- Every error that comes from a file names the file.
- `figment`, `toml_edit` and `schemars` stay behind this crate's API.
- Published to crates.io from this repository.

## Commands

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
