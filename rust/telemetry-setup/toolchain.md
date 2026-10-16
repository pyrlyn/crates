# Toolchain

| Program | How to install | Why here | Source |
| --- | --- | --- | --- |
| rustc | mise | Build | https://github.com/rust-lang/rust |
| just | mise | Test recipe | https://github.com/casey/just |
| ketch | see its README | Installs swarfr | https://github.com/pyrlyn/ketch |
| swarfr | ketch | Lossless cleanup of target/ after tests | https://github.com/listepo/swarfr |

## cargo

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| tracing | local | https://github.com/tokio-rs/tracing | Subscriber and global default |
| tracing-subscriber | local | https://github.com/tokio-rs/tracing | JSON and human formatters, `EnvFilter`, registry |
| tracing-appender | local | https://github.com/tokio-rs/tracing | Daily rotation and the non-blocking file writer |
| regex | local | https://github.com/rust-lang/regex | Credential shapes and secret-named pairs |
| serde_json | local | https://github.com/serde-rs/json | Structural masking of JSON log records |
| thiserror | local | https://github.com/dtolnay/thiserror | `Error` |
| opentelemetry | local | https://github.com/open-telemetry/opentelemetry-rust | `otlp` feature: span attributes and tracer |
| opentelemetry_sdk | local | https://github.com/open-telemetry/opentelemetry-rust | `otlp` feature: tracer provider and the masking span processor |
| opentelemetry-otlp | local | https://github.com/open-telemetry/opentelemetry-rust | `otlp` feature: OTLP/HTTP span exporter |
| tracing-opentelemetry | local | https://github.com/tokio-rs/tracing-opentelemetry | `otlp` feature: tracing spans to OpenTelemetry |
| tempfile | local | https://github.com/Stebalien/tempfile | Log directories in tests |

## ketch

| Package | Where | Source | Why here |
| --- | --- | --- | --- |
| swarfr | global | https://github.com/listepo/swarfr | Lossless cleanup of target/ after tests |
