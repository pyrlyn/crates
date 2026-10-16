# llm-http

HTTP plumbing for LLM provider wires, on top of `reqwest`.

- `http`: `client_with_timeout` (bounded connect timeout, idle read timeout),
  `resolve_key` (env var first, then the platform keyring; a switch variable
  turns the keyring off for tests and dev runs), `bearer` and `api_key`
  (sensitive auth headers), and `map_http_error` (a non-2xx response to a
  `llm_wire::ProviderError`).
- `retry`: `stream_with_retry` runs one provider attempt with exponential
  backoff and jitter, honours `retry-after`, and retries only before the
  first event reached the caller. `stream_with_retry_with` is the same loop
  with the backoff sleep passed in, for a host without tokio's timer (it is
  the only one on wasm32-unknown-unknown) and for tests that must not wait.
- `Transport`: the section every wire's constructor takes (base URL, key env
  var, idle timeout, max retries).
- `sse`: `sse_stream` turns a byte stream into `(event, data)` frames and
  `parse_sse_str` parses a whole body for fixtures.

```rust
use llm_http::http::{bearer, resolve_key};

fn auth_header() -> Result<reqwest::header::HeaderValue, llm_wire::ProviderError> {
    let key = resolve_key("OPENAI_API_KEY", "my-app", "openai", "MY_APP_KEYRING")?;
    bearer(&key)
}
```

## Features

`keyring`, `reqwest` and `sse` are on by default, so a plain dependency gets them; `test-util` is off.

| Feature | Gates | Pulls in |
| --- | --- | --- |
| `keyring` | `http::resolve_key` (env var, then the platform keyring) | `keyring` |
| `reqwest` | `http::client_with_timeout` | `reqwest` |
| `sse` | the `sse` module (`sse_stream`, `parse_sse_str`) | `eventsource-stream`, `bytes`, `futures` |
| `test-util` | the `cassette` module: record and replay HTTP bodies with secrets masked (off by default) | `serde`, `llm-wire/test-util` |

A host that keeps secrets in its own store and sends requests through its own
HTTP stack sets `default-features = false` and builds without `keyring`,
`reqwest` or `eventsource-stream`. It keeps `resolve_key_with` (the lookup
injected), `bearer`, `api_key`, `map_http_error` (on `http` crate types,
the ones reqwest re-exports), the whole `retry` module and `Transport`. That
build also compiles for wasm32-unknown-unknown, where the backoff jitter reads
the JS clock through `web-time`.

```toml
llm-http = { version = "0.1", default-features = false }
```

Licensed `GPL-3.0-or-later OR LicenseRef-Royalty-Free`; see the repository's
`LICENSE` and `LICENSE-ROYALTY-FREE.md`.
