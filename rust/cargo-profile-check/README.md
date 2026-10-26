# cargo-profile-check

Checks that the `[profile.*]` sections of Cargo manifests match a shared profile template,
and prints one plain-text line per difference. The output is meant for agents and scripts:
no markdown, no colors, a stable grammar.

## How it works

1. The template is any TOML file with `[profile.<name>]` and `[profile.<name>.package."*"]`
   tables. Without `--template`, the bundled one is used (`cargo profile-check --print-template`).
2. For each manifest, profiles are resolved like Cargo does: built-in defaults for `dev` and
   `release`, `test` inherits `dev`, `bench` inherits `release`, custom profiles follow
   their `inherits` chain (a cycle or unknown parent is an error). `package."*"` keys are
   inherited along the same chain.
3. Every key of every template profile is compared with the effective value. Equivalent
   spellings are equal: `debug` `true`/`2`/`"full"`, `false`/`0`/`"none"`, `1`/`"limited"`;
   `lto` `true`/`"fat"`; `strip` `true`/`"symbols"`, `false`/`"none"`; `opt-level` `3`/`"3"`.
   `inherits` is compared as written on the profile itself.
4. With `--workspace <dir>` the tree is walked (skipping `target`, `node_modules`, dot
   directories and `--exclude` dirs). Workspace roots and standalone packages are checked.
   Cargo ignores `[profile]` in workspace members, so a member that defines one gets a
   `WARN` line and is not checked.

## Output grammar

```
line    = add | remove | warn
add     = "ADD "    manifest " " section " " key " = " value
remove  = "REMOVE " manifest " " section " " key " = " value
warn    = "WARN "   manifest " " message
section = "profile." name [ ".package.\"*\"" ]
value   = a TOML value (inline), e.g. 1, false, "thin"
```

- `ADD`: the manifest must have `key = value` in `section` (key or whole profile missing,
  or it differs).
- `REMOVE`: the manifest's own `section` has `key = value` that must go. A differing value
  set in that section gives `REMOVE` (actual) followed by `ADD` (expected). A key set in the
  section but absent from the template is a `REMOVE` only. If the wrong value is inherited
  (defaults or a parent profile), only the `ADD` is printed.
- Lines are ordered by manifest, then section, then key. Errors go to stderr as `ERROR ...`.

Example:

```
REMOVE rust/x/Cargo.toml profile.dev debug = true
ADD rust/x/Cargo.toml profile.dev debug = "line-tables-only"
ADD rust/x/Cargo.toml profile.dist inherits = "release"
WARN rust/x/crates/y/Cargo.toml member crate defines [profile], which Cargo ignores; profiles come from rust/x/Cargo.toml
```

`--format json` prints `{"ok": bool, "manifests": [{"manifest", "warnings", "diffs":
[{"op", "section", "key", "value", "toml"}]}]}`.

Exit status: `0` no differences (warnings allowed), `1` differences, `2` usage, read or parse
error (including an `inherits` cycle).

## Usage

```bash
cargo install cargo-profile-check            # or: cargo install --path rust/cargo-profile-check
cargo profile-check --workspace .            # walk and check
cargo profile-check path/to/Cargo.toml crate-dir
cargo profile-check --template my-profiles.toml --workspace . --exclude vendor
cargo profile-check --format json Cargo.toml
```

With no paths and no `--workspace`, the current directory is walked.

## Enable / disable

It is opt-in; nothing runs it by default. To enforce it in CI add a step:

```yaml
- run: cargo install cargo-profile-check --locked
- run: cargo profile-check --workspace .
```

Disable by removing the step (or make it advisory with `continue-on-error: true`). Pass
`--template` to use a repo-specific template.

## Tests

```bash
cargo test -p cargo-profile-check
cargo clippy -p cargo-profile-check --all-targets -- -D warnings
```

Fixtures in `tests/fixtures`: `match`, `mismatch`, `chain` (inherits chain), `cycle`,
`members` (member `[profile]` warning).

## Limitations

- Only `package."*"` overrides are compared; named package overrides and `build-override`
  are ignored.
- Defaults are Cargo's documented ones; platform-dependent `split-debuginfo` has no default,
  so a template key for it must be set explicitly.
- Profiles from `.cargo/config.toml` or `CARGO_PROFILE_*` env vars are not considered.
- Workspace membership is detected from `members`/`exclude` globs (`*`, `?`) and
  `package.workspace`; `default-members` is not needed for this and is ignored.
- `inherits` on `test`/`bench`/`dev`/`release` is not validated (Cargo rejects it).
