# file-backup

Copy a file to `<name>.bak-<unix-seconds>` before replacing it.

Used by `ketch config reset` and by rtok agent host setup. A missing file is not
copied. Copies are created with `create_new`, so a planted symlink at the backup
name cannot redirect the write, and a second copy in the same second gets a
`-<n>` suffix. Hard links and symlinks are never treated as backups.

Two targets:

- `backup` copies beside the file. A sibling `<name>.bak-*` that already holds the
  same bytes is not copied again.
- `Folder { name, keep }` copies into `<parent>/<name>/` (`DEFAULT_FOLDER` is
  `_backup`). Any regular file in that folder with the same bytes counts as the
  backup, whatever its name. After a copy, `prune` keeps the newest `keep`
  generations of that file name (`0` keeps all) and never deletes the new copy;
  only files named `<name>.bak-<ts>[-<n>]` in a folder of that exact name are
  candidates. `stale(dir)` lists what a lower cap would delete, without deleting.

```rust
use file_backup::{backup, Folder, DEFAULT_FOLDER};
let path = std::path::Path::new("config.toml");
let _ = backup(path)?;
let _ = Folder { name: DEFAULT_FOLDER, keep: 5 }.backup(path)?;
```
