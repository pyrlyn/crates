# perm-rules

Decide whether a tool call runs, asks or is refused, from rules a user wrote.

- Rules are Claude Code's strings: `Bash(git commit:*)` (word-boundary prefix),
  `Edit(src/**)` (path glob, `~/` and cwd-relative), `WebFetch(domain:x.com)`,
  `mcp__srv__*`, or a bare tool name. Any other tool kind (`App(...)`,
  `Site(...)`) parses with exact, prefix or domain subjects.
- Order, first hit wins: deny rule, `Bypass`, `Plan`, allow rule, ask rule,
  session grant, `Untrusted` policy, then what the call's `Risk` needs.
- A split shell line is judged by its commands: one deny or ask hit is enough,
  allow needs every command, and an opaque split is never allowed by a prefix.

```rust
use std::path::Path;
use perm_rules::*;

let rules = RuleSet { allow: vec!["Bash(git status:*)".into()], ..RuleSet::default() };
let engine = Engine::compile(&rules, Grammar::default(), None, Path::new("/repo"))?;
let call = Call::new("bash", "git status --short", Risk::Exec);
let outcome = engine.decide(
    &call,
    PermissionMode::Default,
    ApprovalPolicy::OnRequest,
    SandboxMode::WorkspaceWrite,
    &[],
);
assert_eq!(outcome, Outcome::Allow { by: DecidedBy::Rule });
# Ok::<(), RuleError>(())
```

Licence: GPL-3.0-or-later, or the royalty-free licence in
[`LICENSE-ROYALTY-FREE.md`](../../LICENSE-ROYALTY-FREE.md).
