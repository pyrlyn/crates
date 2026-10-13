# llm-wire — completed tasks

### T1. Extract the provider contract from cox-protocol

Copied verbatim from cox's `cox-protocol` (and the pure helpers of `cox-provider-testkit`) so aulo can share the contract; cox is untouched. Moved: the `Provider` trait (`traits`); `ProviderError` (`errors`); `CallId` and `ArchiveId` (`ids`); `Request`, `ProviderEvent`, `Caps`, `ToolSpec`, `Usage`, `Risk` and what they reach: `Concurrency`, `Tier`, `Job`, `Effort`, `Thinking`, `StopReason`, `ModelId`, `ProviderId`, `SystemBlock`, `Role`, `Content`, `Message`, `ArchiveRef` (`types`); with their unit tests. Behind feature `test-util`: scenario parsing and `events_for` (`test_util::scripted`) and cassette hashing, redaction, writing and the near-miss hint (`test_util::replay`). The `Scripted` and `Replay` provider structs stay in cox's `cox-provider`: `Scripted` estimates usage with cox's `tokens::estimate` and `Replay` feeds a cassette through its Anthropic stream parser, so moving them would pull those into this crate. Check: `cargo test -p llm-wire --all-features` (21 tests), `cargo clippy -p llm-wire --all-targets --all-features -- -D warnings` and `cargo fmt --all --check` green.

### T3. One ProviderModel, Capabilities and effort rule for the llm crates

Aulo follow-up to T1.3, T1.4 and T1.5, which each carried their own copy of cox's model entry and effort decision. Moved into this crate: `model` (`ProviderModel`, the catalog's former `ModelEntry` under cox's name, with serde and JSON Schema; `Capabilities` with `declared_by`) and `effort` (`Api`, `WireEffort`, `effort_for`, `supports_adaptive_thinking` and its prefix list), with their tests. `llm-catalog` and `llm-wire-openai` now read these types, `llm-wire-anthropic` and `llm-wire-openai` call `effort_for`, and `llm_wire_openai::Capabilities::chat_effort` is gone (`effort_for(Api::Chat, ..)` replaces it). `ulid_id!` is exported so `agent-loop` shares it (T1.15). No new dependencies. Check: `cargo test -p llm-wire --all-features` (24 tests), clippy and `cargo fmt --all --check` green.

### T4. Builds for wasm32-unknown-unknown

Mailune's browser build uses `llm-wire`. ULID ids draw on getrandom 0.4, which has no backend on wasm32-unknown-unknown until its `wasm_js` feature is on, so a target-specific dependency turns it on there; ulid already links wasm-bindgen on that target through web-time, which is the exception getrandom's README allows a library. Check: `cargo build -p llm-wire --target wasm32-unknown-unknown` and its clippy `-D warnings` green, in CI too; native tests unchanged.
