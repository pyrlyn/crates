# proc-sandbox — completed tasks

### T1. Extract the process sandbox from cox

Moved out of cox's `crates/cox-sandbox` (aulo T1.9): `sandbox::command`, `sandbox::argv`, `sandbox::backend`, the Seatbelt profile builder, the bubblewrap argv builder (with `expose_under_private_tmp`), the Landlock plus seccomp guard, and `path::confine`, with their tests. No behaviour change. cox's `SandboxMode`, `LinuxBackend` and `SandboxPolicy` became the crate's own plain types in `policy` (no serde), and `ToolError::{Confined, Io}` became `path::ConfineError`. `unsafe_code` is denied with the same two targeted allows cox had (the `pre_exec` hook and the Landlock ABI probe); `unwrap`, `expect` and `panic` are denied outside tests. Check: `cargo test -p proc-sandbox` (19 tests, macOS) and `cargo clippy -p proc-sandbox --all-targets -- -D warnings` green on Rust 1.99, also for `x86_64-unknown-linux-gnu`.
