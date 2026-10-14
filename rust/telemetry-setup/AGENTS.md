# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The `tracing` setup applications repeat: a daily-rotated JSON log file, an
optional human log on stderr, a filter from config or an environment variable,
and OTLP trace export behind the `otlp` feature. Every line and every exported
span is masked for secrets first. Extracted from aulo's `aulo-telemetry`;
Mailune's F5 and cox's `cox-telemetry` are the next consumers. It knows nothing
about any one application: the name, the filter variable and extra credential
shapes are settings.

## Rules

- Masking happens on the finished line, in `Scrubbed`, before any sink or
  worker thread sees the bytes. Do not add a sink that bypasses it. The OTLP
  path masks span attributes in its own processor because the OpenTelemetry
  layer records fields itself.
- Fail closed: the free `redact` functions mask the whole text if the built-in
  rules do not compile, and `subscriber` refuses to start rather than log
  unmasked.
- Removing or weakening a built-in rule changes what every consumer leaks;
  the tests in `tests/redaction.rs` pin them.
- Dependency versions follow aulo's lock. Bump them only with the creator, and
  the four OpenTelemetry crates together.
- Tests open no socket: the `otlp` test builds the exporter and opens no span.

## Commands

```bash
cargo test
cargo test --features otlp
cargo clippy --all-targets --all-features
cargo fmt
```

`just test` runs both test sets and finishes with a lossless `swarfr` cleanup of the target dir.
