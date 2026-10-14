# app-home

Resolve where a command-line app keeps its files, the same way on every OS.

```rust
let home = app_home::app_home("AULO_HOME", ".aulo");   // $AULO_HOME, else ~/.aulo
let cfg = app_home::config_home().map(|d| d.join("runa")); // $XDG_CONFIG_HOME, else ~/.config
let p = app_home::Dirs::from_env().expand_tilde("~\\.ketch\\bin");
```

- `user_home`: `HOME`, else `USERPROFILE`, else the OS account's home. An
  empty variable counts as unset: native Windows shells often export an empty
  `HOME`.
- `app_home(var, ".name")`: `$var` with a leading `~` expanded (a value from a
  JSON `env` block is never shell-expanded), else `<home>/.name`.
- `config_home`, `data_home`, `cache_home`, `state_home`: the XDG variable when
  it is an absolute path (the spec calls relative values invalid), else
  `~/.config`, `~/.local/share`, `~/.cache`, `~/.local/state` on every OS.
- `expand_tilde`: `~`, `~/x` and `~\x`, joined by components so `~\.app\bin`
  is two segments on Unix too.

`Dirs::with(lookup)` runs the same rules over any variable lookup, for tests.
