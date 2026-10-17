# atomic-replace

Replace a file so a reader sees either the old bytes or the new ones, never a
half-written file.

```rust
atomic_replace::write("state.json", b"{}\n")?;

atomic_replace::Options::new()
    .create_parent(true)
    .durable(false)
    .write_with("cache/index.bin", |file| file.write_all(&bytes))?;
```

- The new bytes go to a sibling temp file (`create_new`, a unique name), which
  is then renamed over the target. Rename is atomic within one filesystem.
- When the target exists its permissions are copied onto the temp file, so a
  0600 file stays 0600. On Windows the target's read-only bit is cleared
  instead, because the rename cannot replace a read-only file.
- When the target is a symlink, even a dangling one, the file it points to is
  written and the link is kept.
- `durable` (on by default) fsyncs the file before the rename and its
  directory after it, so a crash cannot leave an empty file behind.
- Every failed step removes the temp file.
