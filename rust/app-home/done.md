# app-home — completed tasks

### T1. The crate: user home, app home, XDG dirs and tilde

Workspace task T13. About 30 hand-written resolvers across rtok, cox, aulo, ketch, runa and swarfr disagreed on empty variables, the Windows `USERPROFILE` fallback, relative XDG values and the no-home fallback. `Dirs` runs one set of rules over an injectable variable lookup (`from_env`, `with`); free functions cover the process env. `user_home` is non-empty `HOME`, else `USERPROFILE`, else `std::env::home_dir`; `app_home(var, dot_name)` expands a leading `~` in `$var`; `config_home`/`data_home`/`cache_home`/`state_home` take the XDG variable only when absolute and fall back to the `~/.config` layout on every OS; `expand_tilde` handles `~`, `~/` and `~\` by components. From rtok-hook, ketch `platform`/`config`/`shell`, cox `cox-config`, aulo `aulo-config` and runa. Check: `cargo test` (9 tests) green on Rust 1.86 and 1.99, clippy `-D warnings` clean for macOS, Linux and Windows targets.
