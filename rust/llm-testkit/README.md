# llm-testkit

Re-exports `llm_http::cassette` from published `llm-http` 0.1.0 (feature `test-util`).

A host that sends requests through its own HTTP stack records the request body,
the status and the response body. Secrets are masked before anything is stored,
and replay matches the masked request. No socket is opened. The implementation
stays in `llm-http`; this crate only gives that module one name to depend on.

```rust
use llm_testkit::cassette::Cassette;

let request = r#"{"authorization":"Bearer sk-fixturevalue1234"}"#;
let mut cassette = Cassette::new();
cassette.record(request, 200, r#"{"key":"sk-ant-api03-abcdef123456"}"#);
let saved = cassette.save();
assert!(!saved.contains("sk-fixturevalue1234"));
assert!(saved.contains("redacted"));
```

```toml
llm-testkit = "0.1"
```

`llm-http` is required with `default-features = false` and `test-util`, so this
crate does not pull `reqwest` or `keyring`.

Licensed `GPL-3.0-or-later OR LicenseRef-Royalty-Free`; see the repository's
`LICENSE` and `LICENSE-ROYALTY-FREE.md`.
