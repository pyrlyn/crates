# change-preview

Preview what a command would change before it changes anything.

- A file edit is the unified diff `git diff` prints, or with `Mode::Stat` one
  `path | 7 +++--` line.
- A removal is one `- path  12.1 GB  48 213 files` line: listing every file of
  a build cache would be millions of lines.
- The preview ends with `3 files changed, 12 insertions(+), 4 deletions(-)`
  and `15 paths, 1 204 311 files, -106.2 GB`.

`paint` colours the text for stdout and leaves it plain when stdout is not a
terminal or `NO_COLOR` is set. `Preview` serializes for `--json`.

```rust
use change_preview::{paint, Mode, Preview};
let mut p = Preview::default();
p.edit("config.toml", "a = 1\n", "a = 2\n")
    .remove("target", 13_000_000_000, 48_213);
print!("{}", paint(&p.render(Mode::Stat)));
```
