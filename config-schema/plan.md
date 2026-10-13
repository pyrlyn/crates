# config-schema

https://github.com/pyrlyn/crates

Schema-checked TOML config helpers: stale-schema check, figment layering with per-key provenance, comment-preserving key edits.

| # | Status | Priority | Complexity | Readiness | Agent |
| --- | --- | --- | --- | --- | --- |
| T2 | todo | P2 | 3 | 0% | |
| T3 | todo | P2 | 3 | 0% | |

### T2. Adopt config-schema in ketch or rtok (needs publication)

Publish the crate through the release pipeline (flip `publish = false`, add it to the token's crate list in the root README), then replace ketch's `toml_file.rs` (`assert_schema_current`, parse and render helpers) or rtok's `config/layers.rs` and `config/validate.rs` (`Named`, `insert_dotted`, `assign`, `reveal_commented_key`, the env leaf table) with this crate. Done when the app's tests pass on the crate and the duplicated code is gone.

### T3. Adopt config-schema in cox

Replace cox-config's `Named`, `env_key`, `set_value_in` and the schema drift test with this crate, keeping the project-config guard list in cox-config (it is cox-specific). Needs T2's publication, or a path dependency while it is unpublished. Done when `cox config set|show --sources` behave as before and cox's tests pass.
