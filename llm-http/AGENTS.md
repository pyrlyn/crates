# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The HTTP plumbing every LLM provider wire needs on top of `reqwest`: client
construction, credential lookup (env var, then platform keyring), auth
headers, non-2xx to `ProviderError` mapping, retry with backoff around one
provider stream, and generic SSE framing. Extracted from cox's
`cox-provider-http` so cox and aulo share one copy. Errors and events are
`llm-wire` types.

## Rules

- **Behaviour-neutral with cox.** Which statuses map to which `ProviderError`,
  which errors retry, the backoff curve and the env-first key lookup match
  cox's. Change one only together with cox, never alone.
- **Secrets stay out of logs and errors.** Auth headers are marked sensitive,
  a bad credential is `ProviderError::Auth` with no text, and nothing here
  formats a key. Keep the tests that prove it.
- **Tests never touch the real OS keyring.** Inject the lookup
  (`resolve_key_with`) or switch the keyring off; only a binary calls
  `resolve_key`.
- **No `unwrap`, `expect` or `panic!` outside tests** (clippy denies them), and
  no `unsafe` (forbidden), so tests inject the environment instead of setting it.
- **Heavy dependencies stay behind features.** `keyring`, `reqwest` and
  `eventsource-stream` are optional (features `keyring`, `reqwest`, `sse`, all
  default). The header helpers, `map_http_error` and `retry` build without
  them and for wasm32-unknown-unknown; CI tests that build, checks its
  `cargo tree` and builds it for wasm32. Use `http` types, not reqwest's, in
  always-available code, and inject anything that needs a native runtime.
- **Doc comments on every public item** (`missing_docs` is on).
- **Published to crates.io** from this repository: a dependency on a sibling crate
  carries both `path` and `version`, so a release publishes the siblings first.

## Commands

```bash
cargo test -p llm-http
cargo clippy -p llm-http --all-targets -- -D warnings
cargo fmt
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
