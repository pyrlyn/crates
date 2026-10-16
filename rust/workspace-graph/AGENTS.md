# workspace-graph — instructions for agents

**What.** Test-support library: `Graph::load` reads a Cargo workspace's direct dependencies (`cargo metadata --no-deps`, chosen `Kind`s), and `check_*` / `assert_*` rules assert its shape: only some members may depend on a package, a member must not depend on a set, a member's dependencies are exactly a set, edges point down a layer order.

**Rules.** `check_*` returns `Violations` as a value (one message per broken edge, stable order) so consumers can prove a rule fails on a synthetic graph built with `FromIterator`; `assert_*` only panics with that message. Declared dependencies only, never the resolved graph: no features, no network, no lockfile resolution. Keep dependencies minimal and maintained; every one has a row in `toolchain.md`. `publish = false`: consumers depend on it by path.

**Workflow.** Tasks live in `plan.md`; finished ones move to `done.md`. Gate: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`. English only.

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too; on conflict, ask the creator.
