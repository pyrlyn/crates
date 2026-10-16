# telemetry-setup

`tracing` setup for an application, with secrets masked before they leave the
process:

- JSON log files in a directory you choose, rotated daily, the oldest beyond
  `max_files` removed;
- an optional human log on stderr, without ANSI codes;
- the filter from config, overridden by an environment variable you name;
- OTLP/HTTP trace export behind the `otlp` feature;
- bearer tokens, provider API keys, forge and chat tokens, cloud keys, JWTs
  and the values of secret-named fields masked in every line and every
  exported span, plus any credential shapes you add.

```rust
use telemetry_setup::{Settings, init};

let mut settings = Settings::new("myapp", "info", "/var/log/myapp").filter_from_env("MYAPP_LOG");
settings.extra_secret_patterns = vec![r"\bmyapp_[0-9a-f]{16,}".into()];
let _guard = init(&settings)?;
tracing::info!(password = "hunter2", "ready"); // stored as password=[REDACTED]
```

Keep the guard alive until the end of `main`; dropping it flushes the file
writer and the exporter. `subscriber` builds the same subscriber without
installing it, for tests. `redact` and `Scrubbed` work on their own for an
application that builds its own layers.

Extracted from aulo's `aulo-telemetry`.

Licensed under either of MIT or Apache-2.0 at your option.
