# perm-rules — completed tasks

### T1. Extract the permission rule engine from cox

Moved out of cox-permission (`rules.rs`, `policy.rs`, `lib.rs`) with its tests so cox and aulo share one rule grammar and decision order (aulo plan T1.8). Behaviour is unchanged; cox's own types are replaced by neutral ones: `Call { tool, subject, risk, segments }`, `RuleSet { allow, ask, deny }`, `RuleError` (was `CoreError::Config`) and plain `Risk`, `PermissionMode`, `ApprovalPolicy`, `SandboxMode`, `DecidedBy`, `Why`, `Segments`. Tool kinds are open strings and `Grammar` registers aliases and path tools, so `App(...)` and `Site(domain:...)` rules parse. Left in cox: the default deny list, `Why::SandboxDenied`, `DecidedBy::{User, Hook}`, the danger-full-access banner and the bash parser. Check: `cargo test -p perm-rules` (68 tests incl. the 37-row decision table and the deny-never-weakens property) and `cargo clippy --all-targets -- -D warnings` green on Rust 1.99.
