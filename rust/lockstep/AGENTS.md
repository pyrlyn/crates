# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

Locks that cannot deadlock: one global acquisition order, or lock-free
structures. Not published (`publish = false`). Tasks live in [plan.md](plan.md) /
[todo.md](todo.md), finished ones in [done.md](done.md); the chosen approach and its
sources in [docs/research.md](docs/research.md).

## Rules

- Read `docs/research.md` before changing the design; a change of approach is
  recorded there first.
- The guarantee holds only if all locking goes through this crate. Never take a
  `std` or `parking_lot` lock inside it except as the primitive a wrapper owns.
- `unsafe_code` is forbidden. Lock-free structures reuse a maintained crate
  rather than unsafe code written here.
- Prefer a compile-time check; fall back to a debug-build runtime check only
  where the type system cannot express the order, and say why in a comment.

## Commands

```bash
cargo test -p lockstep
cargo clippy -p lockstep --all-targets -- -D warnings
cargo fmt --all -- --check
```
