# git-changed-paths

Paths a git working tree changed relative to a base ref.

```rust
use git_changed_paths::changed_paths;

let changed = changed_paths(std::path::Path::new("."), "origin/main")?;
for path in &changed.paths {
    println!("{}", path.display());
}
```

Includes commits since `git merge-base HEAD <base>`, staged and unstaged edits,
untracked files that are not ignored, deleted files and both sides of a rename.
Paths are relative to the repository top level even when the given directory is a
subdirectory. Requires the `git` CLI on `PATH`. In a shallow clone with no common history
the error is `ShallowNoMergeBase`, which tells the user to fetch more history.
