# path-gates

Map changed paths to named gates by glob rules from a TOML config.

```toml
[[gate]]
name = "rust"
paths = ["**/*.rs", "**/Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]

[[gate]]
name = "docs"
paths = ["**/*.md"]

[[gate]]
name = "lint"
always = true # selected whenever anything changed

unmatched = "all" # default; "ignore" drops paths no gate claims
```

```rust
use path_gates::Rules;

let rules = Rules::from_toml(&std::fs::read_to_string("gates.toml")?)?;
let selection = rules.select(["src/lib.rs", "README.md"]);
// selection.gates == {"docs", "lint", "rust"}
```

An unclaimed path selects every gate (`selection.all`), so a forgotten rule runs
too much instead of too little. Paths are matched as `/`-separated repo-relative
strings; `\` is read as a separator, `.` / empty segments are collapsed, and
`**/x` also matches `x` at the root.

Unknown keys (a host tool's `run = "..."`, `base = "..."`) are ignored. A host
can also deserialize its own struct holding `GateSpec` values and build rules
with `Rules::new(specs, Unmatched::All)`.
