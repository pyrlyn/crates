# Ideas

- TOML hosts (Codex `config.toml` `[mcp_servers]`) with `toml_edit`, the way `rtok-mcp` already does it, so one crate covers every host's server map.
- A generic ownership check for removal (`unregister_owned`): remove an entry only when a caller-supplied predicate says it is ours, instead of each caller reading `entry_at` first.
- Backup pruning (keep the newest N generations), which rtok's own backup had; it belongs in `file-backup`.
- Lossless edits of other keys a host reads (hooks in `settings.json`), on the same syntax tree.
