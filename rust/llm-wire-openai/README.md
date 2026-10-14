# llm-wire-openai

The OpenAI wires for LLM agent loops, on top of `llm-wire` and `llm-http`.

- `chat`: Chat Completions (`POST /v1/chat/completions`), the subset every
  OpenAI-compatible server speaks (Ollama, LM Studio, vLLM, llama.cpp,
  OpenRouter, DeepSeek, Gemini's compatibility endpoint, xAI). Streams text,
  reasoning content and parallel tool calls, and replays a Gemini tool-call
  thought signature.
- `responses`: the Responses API (`POST /v1/responses`) OpenAI's own models
  use, with request and event types from `async-openai`'s `response-types`
  (no second HTTP client).
- The section and model fields the wires read are `llm_http::Transport`,
  `llm_wire::ProviderModel` and `llm_wire::Capabilities`; the Chat wire's
  effort decision is `llm_wire::effort_for`.

Each wire exposes `build_body` (request translation), a state machine that
turns SSE frames into `ProviderEvent`s, and a provider that implements
`llm_wire::Provider` with retry before the first event.

```rust
use llm_wire_openai::chat::OpenAiChatProvider;
use llm_http::Transport;

fn ollama() -> Result<OpenAiChatProvider, llm_wire::ProviderError> {
    let transport = Transport {
        base_url: "http://localhost:11434/v1".into(),
        api_key_env: String::new(),
        timeout_s: 120,
        max_retries: 4,
    };
    OpenAiChatProvider::new(&transport, None, vec![], 32_768)
}
```

Licensed `GPL-3.0-or-later OR LicenseRef-Royalty-Free`; see the repository's
`LICENSE` and `LICENSE-ROYALTY-FREE.md`.
