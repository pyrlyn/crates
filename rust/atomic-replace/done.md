# atomic-replace — completed tasks

### T1. The crate: atomic replace with permissions, symlinks and fsync

Workspace task T11. Six projects carried about 18 hand-written temp-file-and-rename writes that disagreed on fsync, permissions, symlinks and temp naming. `write(path, bytes)` and `Options { durable, preserve_permissions, follow_symlinks, create_parent, mode }` with `write`/`write_with`, plus `sync_dir`. Ported from rtok `rtok-agent-sdk` `write_atomic`/`link_target` (permissions, Windows read-only bit, dangling links) and ketch `state.rs` `save_path` (fsync of the file and its directory). Temp files are `create_new` with pid, nanos and a counter in the name, removed on every error path; a symlink cycle is an error instead of replacing one of its links. Check: `cargo test` (11 tests) green on Rust 1.86 and 1.99, clippy `-D warnings` clean for macOS, Linux and Windows targets.
