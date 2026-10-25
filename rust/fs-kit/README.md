# fs-kit

One entry point for file and directory work.

| Module | What |
| --- | --- |
| `path` | `canon`/`canonicalize` without the Windows `\\?\` prefix, `same_path`, `same_file`, `path_starts_with`, `path_ends_with`, `strip_prefix` (ASCII case folded on Windows), `normalize` (lexical join), `clean` (lexical `.`/`..` folding), `is_unwalkable_root` |
| `write` | `atomic` replace through a sibling temp file (fsync, permissions and symlinks kept), `Options` for the knobs, `write_with`, `if_changed`, `sync_dir` |
| `read` | `text_or_empty`, `bytes_if_exists`: a missing file is not an error |
| `tree` | `usage`/`usage_until` (disk usage and file count, links never followed), `copy_tree`/`copy_tree_with` (links kept, followed or skipped; a file is read back only for a `fix`), `clone_tree` (feature `clone`: copy-on-write clones on APFS, btrfs, XFS, bcachefs, ReFS and Dev Drive, a full copy elsewhere, mtimes kept), `remove_any` |
| `backup` | `backup`/`backup_at`: `<name>.bak-<unix-seconds>` beside a file, skipped when an identical copy exists |

Every error is an `io::Error` of the original kind naming its path; `fs_kit::split` and `fs_kit::raw_os_error` reach the original error and its OS code.

```rust,no_run
fs_kit::write::atomic("state.json", b"{}\n")?;
let freed = fs_kit::tree::usage("target".as_ref());
# Ok::<(), std::io::Error>(())
```

Every error names the path it is about and keeps its `io::ErrorKind`.
