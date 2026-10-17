# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The Anthropic Messages wire for LLM agent loops: the translation of an
`llm-wire` `Request` into a Messages body (cache breakpoints, thinking,
effort), the SSE to `ProviderEvent` state machine, and a streaming client that
implements `llm_wire::Provider` over `llm-http`. Extracted from cox's
`cox-provider-anthropic` so cox and aulo share one copy.

## Rules

- **Behaviour-neutral with cox.** Request bytes, breakpoint placement, which
  frames become which events and which usage counters saturate match cox's.
  The request snapshots and the recorded SSE fixtures prove it; change one
  only together with cox, never alone.
- **Provider output is untrusted input.** Keep the guards in `stream`: an
  unknown block, delta or event kind is skipped before the typed parse, a
  malformed frame is `ProviderError::Parse`, a counter past `u32` saturates
  and a negative one is dropped. Keep the caps in `request`: at most four
  cache breakpoints, stale indices skipped, thinking replayed only on the
  model that produced it.
- **The types are generated, not written.** `build.rs` runs typify over
  `schema/anthropic-openapi.json` (see `schema/README.md`); never edit the
  spec by hand. A field the spec renamed fails to compile in `request.rs`.
- **Secrets stay out of logs and errors.** The key goes out as a sensitive
  `x-api-key` header through `llm_http::http::api_key`; nothing formats it.
- **Tests never touch the real OS keyring or the network.** Inject the lookup
  or point the client at a local `wiremock` server.
- **No `unwrap`, `expect` or `panic!` outside tests** (clippy denies them), and
  no `unsafe` (forbidden), so tests inject the environment instead of setting it.
- **Doc comments on every public item** (`missing_docs` is on; only the
  generated module is exempt).
- **Published to crates.io** from this repository: a dependency on a sibling crate
  carries both `path` and `version`, so a release publishes the siblings first.

## Commands

```bash
cargo test -p llm-wire-anthropic
cargo clippy -p llm-wire-anthropic --all-targets -- -D warnings
cargo fmt
cargo insta review   # after an intentional change to the request bytes or events
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
