# Ideas

- rtok `rtok-agent-sdk` `write_atomic`/`link_target`, `store` archive and `transcript_cache` write through this crate.
- ketch `state.rs`, `lockfile.rs`, `manifest.rs`, `listing.rs` and `shell.rs` drop their `NamedTempFile` copies.
- cox `cox-tools` `atomic_write`, `cox-ext` presence, `cox-plugin` pointer and the non-atomic `cox-config` write.
- swarfr `index.rs`, `known.rs` and `daemon/mod.rs` state writes; aulo `tls.rs` `write_private` (with `mode(0o600)`).
