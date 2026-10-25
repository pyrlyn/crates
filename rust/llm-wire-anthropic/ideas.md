# Ideas

- The coding agent re-vendors its copy of the OpenAPI snapshot with its `agent-vendor anthropic-spec` script. Move that script (or a small Rust equivalent) next to `schema/` so the spec is re-vendored from this crate, and the coding agent stops carrying the file.
- `llm_wire::supports_adaptive_thinking` is a model-id prefix list that goes stale with each model family. Let the caller pass the answer from its own model catalog (`Capabilities::adaptive_thinking`) instead of keeping a list in `llm-wire`.
- `count_tokens` returns `Unsupported` because the coding agent has not implemented it either; `POST /v1/messages/count_tokens` takes the same body as `build_body`.
