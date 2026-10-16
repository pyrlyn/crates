# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The extension points cox and aulo load from files: `SKILL.md` skills (discovery,
the index line for the prompt, the deferred `skill` tool) and Claude-Code-style
shell hooks (`[[hooks.<Event>]]` commands, JSON payload on stdin, exit 2 blocks,
`HookChain` over shell and plugin hooks). It carries no host types: `HookEvent`,
`HookOutcome`, `HookConfig`, `HooksConfig` and the `Hook` trait are defined here
with the host's serde shape, and the host wraps or maps them.

Skill files are untrusted input. A skill that does not parse, has no
`name`/`description`, is not lowercase-hyphen or does not match its directory is
skipped with a notice, never fatal and never half-loaded. A change that loads
more than this needs a regression test in `tests/skills.rs`.

Hooks fail open and their output is untrusted too: a spawn error, a bad exit, a
timeout (the whole process group is killed) or a broken matcher regex is a
`HookOutcome::Failed`, never a panic or an error to the caller; only exit 2 or an
explicit block verdict blocks, and plain non-JSON stdout means continue. The
runner is Unix-only (`sh -c`, process groups). A change that makes it block or
fail harder needs a regression test in `tests/hooks.rs`.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
