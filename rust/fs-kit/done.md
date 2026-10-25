# Done

### One crate for file and directory work

Path identity from the token CLI `tokcli-store/src/paths.rs` and the lexical join from the token CLI `src/fs.rs` (the package manager `lexical_join` and proc-sandbox `lexically_normalize` are the same join); atomic replace from the `atomic-replace` crate of workspace T11 (itself from the token CLI `tokcli-agent-sdk` `write_atomic` and the package manager `state.rs`); tree usage from the token CLI `src/disk_usage.rs`; `copy_tree` from the token CLI `tokcli-agent-sdk` `copy_dir` and the package manager `copy_tree`; `remove_any` from the package manager `platform/unix.rs`; backups from `file-backup`, with its name-probe race and planted-link overwrite (workspace T4) closed by `create_new`. Check: `cargo test -p fs-kit`, clippy `-D warnings`, `cargo +1.86 check -p fs-kit --all-targets`, `cargo publish --dry-run -p fs-kit`.
