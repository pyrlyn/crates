# bless-check

Check that a committed generated file matches what the code renders now, or
rewrite it when blessed.

```rust
use bless_check::{Mode, assert_fresh};

#[test]
fn the_committed_schema_is_current() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema.json");
    assert_fresh(
        &path,
        render_schema(),
        Mode::from_env("APP_BLESS"),
        "regenerate it with APP_BLESS=1 cargo test",
    );
}
```

| Mode | Fresh file | Different file | Missing file |
| --- | --- | --- | --- |
| `Check` | `Ok(Fresh)` | `Err(Differs)` | `Err(Missing)` |
| `CreateMissing` | `Ok(Fresh)` | `Err(Differs)` | writes, `Ok(Written)` |
| `Bless` | `Ok(Fresh)` | writes, `Ok(Written)` | writes, `Ok(Written)` |

`Mode::from_env(var)` is `Bless` when `var` is set to anything, else `Check`.
CRLF and LF compare equal, so a Windows checkout of an LF file is fresh. A
fresh file is never rewritten. `Error::Differs` names the first differing
line with an excerpt of each side. A binary with a `--check` flag calls
`check_or_bless` directly and turns `Error::is_stale` into its exit code.

Std only. Not published: depend on it by path,
`{ path = "../../packages/crates/rust/bless-check" }`.
