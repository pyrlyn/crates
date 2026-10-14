# config-schema

Schema-checked TOML config helpers, generic over your config type.

```rust,ignore
// A test: fails when docs/config.schema.json no longer matches the types.
config_schema::check_schema::<Config>(&path, Some("Generated. Do not edit."), "MYAPP_BLESS")?;

// Layers, lowest first, each with a name; every key remembers where it came from.
let env = EnvLayer::new("MYAPP_", &Config::default())?.ignore(&["home"]);
let loaded = Layers::new()
    .defaults("default", &Config::default())
    .file("user", user_path)
    .file("project", project_path)
    .env("env", &env)
    .overrides("flag", &flags)?
    .load::<Config>()?;
loaded.provenance.get("proxy.port"); // Some(Origin { layer: "user", source: Some(path) })

// `config set`: one key changes, comments and layout stay, the result must still load.
let edit = plan_edit::<Config>(&path, &[("proxy.port", parse_value("8791"))])?;
// print a diff of edit.before / edit.after for --dry-run, or:
edit.apply()?; // atomic write
```

| Module | What it does |
| --- | --- |
| `schema` | `schema_text::<T>()` (draft 2020-12, `null` dropped because TOML has none) and `check_schema::<T>()`; the error names the variable that regenerates the file |
| `layers` | `Layers`: named layers (`defaults`, `toml_text`, `file`, `env`, `overrides`, `provider`), `extract`, `load` with `Provenance`, `leaves`, `origin_of`; `git_root`, `find_up` |
| `env` | `EnvLayer`: `PREFIX_*` variables matched against the defaults' key tree, so `APP_PROXY_OPENAI_UPSTREAM` is `proxy.openai_upstream`; array keys take comma lists |
| `edit` | `plan_edit::<T>` (own file: reveals commented defaults, validates), `plan_edit_foreign` (only the named keys change), `read_entry` (check only our entry in a foreign file), `parse_value`, `value_from_json`, atomic `Edit::apply` |

## License

`GPL-3.0-or-later OR LicenseRef-Royalty-Free` (see the repository's `LICENSE` and `LICENSE-ROYALTY-FREE.md`).
