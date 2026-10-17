# llm-wire-anthropic

The Anthropic Messages wire for LLM agent loops, over `llm-wire` and `llm-http`.

- `request::build_body` turns a `llm_wire::Request` into the JSON body for
  `POST /v1/messages`: `cache_control` breakpoints (at most four, on the block
  the index names), `output_config.effort`, adaptive `thinking` for the models
  that take it, and thinking blocks replayed only on the model that produced
  them. Pure, so every rule is a snapshot test.
- `stream::AnthropicStream` feeds SSE frames one at a time and returns the
  `ProviderEvent`s each produces, with usage accumulated across
  `message_start` and `message_delta`.
- `AnthropicProvider` implements `llm_wire::Provider`: headers (version, key,
  workspace id, beta), credential lookup, retry with backoff and streaming.
- The wire types are generated at build time with typify from the vendored
  OpenAPI spec in `schema/`.

```rust
use llm_wire_anthropic::{AnthropicProvider, CacheTtl};
use llm_http::Transport;

fn client() -> Result<AnthropicProvider, llm_wire::ProviderError> {
    let transport = Transport {
        base_url: "https://api.anthropic.com".into(),
        api_key_env: "ANTHROPIC_API_KEY".into(),
        timeout_s: 60,
        max_retries: 4,
    };
    AnthropicProvider::new(&transport, "my-app", "MY_APP_KEYRING", CacheTtl::FiveMinutes, false, 200_000)
}
```

Licensed `GPL-3.0-or-later OR LicenseRef-Royalty-Free`; see the repository's
`LICENSE` and `LICENSE-ROYALTY-FREE.md`.
