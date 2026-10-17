# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The process sandbox and the path guard, extracted from cox's `cox-sandbox`:
`sandbox::command` and `sandbox::argv` turn a `SandboxPolicy` into a confined
`Command` or argv (Seatbelt on macOS, bubblewrap or Landlock plus seccomp on
Linux), and `path::confine` keeps a path from an untrusted caller inside the
workspace roots. Both are trust guards: a change that weakens either needs a
test that fails without it.

## Rules specific to this crate

- `unsafe_code` is denied. The only allows are the `pre_exec` hook in
  `sandbox::command` and the Landlock ABI probe in `sandbox::landlock`; each
  carries a `// why:` comment. Do not add a third.
- No `unwrap`, `expect` or `panic!` outside tests.
- The Linux backends are `cfg(target_os = "linux")`; keep the argv and profile
  builders pure so they are tested on every platform.
- No dependency on cox or aulo crates. The policy types live in `policy`.

## Commands

```bash
cargo test -p proc-sandbox
cargo clippy -p proc-sandbox --all-targets -- -D warnings
cargo clippy -p proc-sandbox --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
cargo fmt
```

`just test` runs the same tests and finishes with a lossless `swarfr` cleanup of the target dir.
