# bless-check — instructions for agents

**What.** Library: a committed generated file (schema, header, bindings, table, fixture) must be what the code renders now. `check_or_bless(path, rendered, mode)` compares them and, in `Mode::Bless` (or `Mode::CreateMissing` for an absent file), writes the file instead of failing; `assert_fresh` is the test form; `Mode::from_env("FOO_BLESS")` reads the usual switch.

**Rules.** Std only, no dependencies. CRLF and LF compare equal (Windows checkouts). An unchanged file is never rewritten. Errors are values (`Error::Missing`, `Error::Differs` with the first differing line, `Error::Io`), so a binary's `--check` mode can report without panicking; only `assert_fresh` panics. `publish = false`: consumers depend on it by path.

**Workflow.** Tasks live in `plan.md`; finished ones move to `done.md`. Gate: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`. English only.

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too; on conflict, ask the creator.
