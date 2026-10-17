# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The OpenAI Chat Completions and Responses wires: for each, a pure request
translator, a pure SSE to `ProviderEvent` state machine and a thin client that
implements `llm_wire::Provider` over `llm-http`. They also serve every
OpenAI-compatible endpoint (Ollama, LM Studio, vLLM, OpenRouter, DeepSeek,
Gemini's compatibility endpoint, xAI, runa). Extracted from cox's
`cox-provider-openai` so cox and aulo share one copy.

## Rules

- **Behaviour-neutral with cox.** Request bodies, event mapping, error mapping
  and retry behaviour match cox's; the insta snapshots pin the bodies and
  events byte for byte. Change one only together with cox, never alone.
- **Provider output is untrusted input.** A malformed, truncated or unknown
  frame is a `ProviderError::Parse` or ignored, never a panic; tool calls
  are emitted whole and only once they have a name; keys travel only in a
  sensitive `Authorization` header and never in errors. Keep the tests that
  prove it.
- **Tests never touch the network.** Streams come from recorded fixtures in
  `fixtures/`; the client tests talk to a local `wiremock` server only.
- **No `unwrap`, `expect` or `panic!` outside tests** (clippy denies them), and
  no `unsafe` (forbidden).
- **Doc comments on every public item** (`missing_docs` is on).
- **Published to crates.io** from this repository: a dependency on a sibling crate
  carries both `path` and `version`, so a release publishes the siblings first.

## Commands

```bash
cargo test -p llm-wire-openai
cargo clippy -p llm-wire-openai --all-targets -- -D warnings
cargo fmt
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
