# shell-classify

Rate a bash command line before anything runs it.

- `classify(line)` returns the riskiest thing the line can do: `ReadOnly`,
  `Write`, `Exec` or `Destructive`. A line that does not parse is `Exec`.
- `segments(line)` returns every simple command the line runs, split on `;`,
  `&&`, `||`, pipes, `&` and newlines, plus an `opaque` flag set when the split
  cannot vouch for the whole line (a substitution, `eval`, `sh -c`, an
  assignment, an output redirect to a path, a parse error). Wrappers such as
  `nohup` and `timeout 5` and the script of `sh -c '...'` are looked through, so a
  deny rule sees the command they hide.

```rust
use shell_classify::{classify, segments, Risk};

assert_eq!(classify("git status && git diff --stat"), Risk::ReadOnly);
assert_eq!(classify("rm -rf build"), Risk::Destructive);
assert_eq!(segments("cd x && ls").commands, ["cd x", "ls"]);
```

Extracted from cox's `bash` tool.
