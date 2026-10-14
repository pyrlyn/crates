# agent-ext

Skills and shell hooks for an agent host.

- `skills`: `discover` scans `<dir>/*/SKILL.md` (YAML frontmatter, the Agent Skills
  format), `index` renders one line per skill for the prompt, and `SkillTool` is
  the deferred `skill` tool that returns a body on demand with its
  `allowed-tools`. A malformed skill is skipped with a notice.
- `hooks`: `ShellHooks` runs the `[[hooks.<Event>]]` commands of a `HooksConfig`
  over Claude Code's protocol (JSON payload on stdin, exit 2 blocks, JSON stdout
  can block, rewrite the input or add context, anything else continues) under a
  timeout that kills the process group. `HookChain` puts shell and plugin hooks
  behind one `Hook`; `chain` is the single rule: the first `Block` or `Failed`
  ends it and a `Modify` feeds the steps after it. A crashing, slow or
  misconfigured hook is a `Failed` outcome the host warns about and skips.
- `frontmatter`: the `---` header split and parse the skill files share.

```rust
use agent_ext::skills::{discover, index, skill_dirs};

let dirs = skill_dirs(".myagent", None, None, Some(std::path::Path::new(".")));
let found = discover(&dirs);
let prompt_part = index(&found.skills);
```

Extracted from cox's `cox-ext`.
