# Roadmap

Approved work that is not yet in `plan.md`.

The audit tasks (T1–T10) are already in `plan.md`.

## Shared filesystem and daemon crates

Approved by the creator on 2026-10-07: extract the filesystem and daemon logic duplicated across apps into one crate per concern here. Each crate is its own task (500-line cap); each consumer migrates in its own task in that app's plan once the crate is published. ketch's MSRV goes to 1.89 for `lock-file` (approved).

- T14. `path-within`: lexical `normalize`, Windows case-insensitive `within`/`same_path`/`strip_prefix`, `canonical_within`, untrusted relative path parse (zip-slip guard), `confine`. From rtok `src/fs.rs`, ketch `platform/mod.rs`, `extract/mod.rs`, cox-sandbox `path.rs`.
- T15. `lock-file`: exclusive lock (std, Rust 1.89), holder record (pid or JSON), in-process re-entrancy registry, `holder()` probe. From rtok-sys, cox-store `lock.rs`, aulo `instance.rs`.
- T16. `dir-ops`: `remove_any`, `copy_tree` with symlink policy and optional fsync, `swap_dir` with rollback, `probe_writable`. From ketch (three copies each), cox-plugin `install.rs`. `remove_any` and `copy_tree` (with a symlink policy) are in `fs-kit` `tree`; `swap_dir` and `probe_writable` are left and go there too.
- T17. `daemon-core`: shutdown signals (SIGINT/SIGTERM, ctrl_c/ctrl_close) behind a `tokio` feature, process `alive`/`terminate`/`kill`/`parent_pid`/`setsid`/`spawn_detached`/`boot_time`, instance lock plus pidfile. From rtok-sys and aulo `daemon/`.
- T18. `local-endpoint`: endpoint resolution (unix socket with `sun_path` length check, per-user named pipe), listener with owner-only dir, probe-before-unlink stale socket, 0600, unlink on drop, Windows spare-instance pipe loop; blocking client `connect` with timeout. From aulo `listener.rs`/`pipe.rs`, rtok `hooks/resident.rs` and `rtok-hook`.
- T19. `login-service`: launchd, systemd `--user` and schtasks install/uninstall/status behind a fakeable `ServiceManager`, correct XML/systemd/MSVCRT escaping. From aulo `daemon/service/`.

Consumer migrations (move into each app's roadmap when the crates they need are published):

- aulo: `daemon-core`, `local-endpoint`, `login-service`, `atomic-replace`, `app-home`, `lock-file`, `telemetry-setup` (T21; approved as Mailune X3, whose Done-when names this migration; `aulo-telemetry` keeps `AULO_LOG` and its `aulo_<hex>` token pattern as settings).
- cox: `gettext-catalog` (T22) under `cox-i18n`, which keeps its catalogs, `tr!` and the native-catalog export; `sqlite-change-feed` (T23) under `cox-store`'s `changes`, which keeps its mutex and `StoreError`.
- scull: `abi-drift` (T24) for `crates/scull-ffi/tests/bindings.rs` (header and C# file, `SCULL_BLESS`).
- ketch: `abi-drift` (T24) for the `ketch-capi` header test (`KETCH_BLESS`); the payload-schema drift check can use `Drift::check` too.
- Mailune: `telemetry-setup` (T21) for its F5 telemetry task, `gettext-catalog` (T22) for its F10 core strings, `sqlite-change-feed` (T23) for its S7 change feed, `abi-drift` (T24) for its B6 C ABI header test, `keychain-secret` (T25) for its C1 secrets, with `guard::violations` as its no-real-keychain test.
- runa: `local-endpoint` (fixes P15.1 unlink-before-bind race), `daemon-core` (SIGTERM, single instance), `login-service` (real launchctl/systemctl, correct escaping), `app-home` (9 resolvers), `atomic-replace` (runa-memory registry), `keychain-secret` (T25) for `runa-cloud/src/secrets.rs` (`RUNA_NO_KEYRING` as the switch).
- rtok: rtok-sys into `daemon-core`/`lock-file`, resident and rtok-hook onto `local-endpoint`, `atomic-replace`, `file-backup`, `path-within`, `app-home`.
- swarfr: `login-service` (low priority keys), `atomic-replace` (three copies), `lock-file`, `app-home`.
- cox: `atomic-replace` (fixes non-atomic `cox-config` write and lost permissions in `cox-tools`), `lock-file`, `app-home` (two resolvers disagree), `path-within`, `dir-ops`.
- ketch: MSRV 1.89, `atomic-replace` (five copies), `lock-file`, `app-home` (two `config_home` disagree), `path-within`, `dir-ops`.
- cox: `wasm-plugin-host` (T20) — `cox-plugin` keeps its manifest, grants and cox host functions on top of the shared engine, worker, package layout and host-function kit.
- scull: a plugin system on `wasm-plugin-host` (T20); what scull plugins may do is agreed with the creator before its tasks are written.
