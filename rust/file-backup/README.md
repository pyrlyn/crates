# file-backup

Copy a file to `<name>.bak-<unix-seconds>` beside it before replacing it.

Used by `ketch config reset` and by rtok agent host setup. A missing file, or a
sibling `.bak-*` that already holds the same bytes, is not copied again.

```rust
use file_backup::backup;
let _ = backup(std::path::Path::new("config.toml"))?;
```
