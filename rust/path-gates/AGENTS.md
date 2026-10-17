# path-gates — instructions for agents

**What.** Library crate: changed repo-relative paths in, named gates out, by glob rules from a TOML config (`Rules::from_toml`, `Rules::select`).

**Rules.** Unknown config keys are ignored (a host keeps its own). Fail safe: a path no gate claims selects every gate (`Selection::all`) unless the config says `unmatched = "ignore"`. Globs use `literal_separator(true)`; input is normalised to `/`. Errors stay in the crate-local `Error` enum. Keep dependencies minimal and maintained; every one has a row in `toolchain.md`.

**Workflow.** Tasks live in `plan.md`; finished ones move to `done.md`. Gate: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`. English only.

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too; on conflict, ask the creator.
