# Done

### One crate for file and directory work

Path identity from rtok `rtok-store/src/paths.rs` and the lexical join from rtok `src/fs.rs` (ketch `lexical_join` and proc-sandbox `lexically_normalize` are the same join); atomic replace from the `atomic-replace` crate of workspace T11 (itself from rtok `rtok-agent-sdk` `write_atomic` and ketch `state.rs`); tree usage from rtok `src/disk_usage.rs`; `copy_tree` from rtok `rtok-agent-sdk` `copy_dir` and ketch `copy_tree`; `remove_any` from ketch `platform/unix.rs`; backups from `file-backup`, with its name-probe race and planted-link overwrite (workspace T4) closed by `create_new`. Check: `cargo test -p fs-kit`, clippy `-D warnings`, `cargo +1.86 check -p fs-kit --all-targets`, `cargo publish --dry-run -p fs-kit`.
