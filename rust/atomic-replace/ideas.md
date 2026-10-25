# Ideas

- The token CLI `tokcli-agent-sdk` `write_atomic`/`link_target`, `store` archive and `transcript_cache` write through this crate.
- The package manager `state.rs`, `lockfile.rs`, `manifest.rs`, `listing.rs` and `shell.rs` drop their `NamedTempFile` copies.
- The coding agent `agent-tools` `atomic_write`, `agent-ext` presence, `agent-plugin` pointer and the non-atomic `agent-config` write.
- swarfr `index.rs`, `known.rs` and `daemon/mod.rs` state writes; aulo `tls.rs` `write_private` (with `mode(0o600)`).
