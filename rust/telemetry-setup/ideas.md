# Ideas

- Export OTLP logs as well as traces, as cox does today, so cox can move onto this crate without losing its log signal.
- Read `OTEL_SERVICE_NAME` and `OTEL_RESOURCE_ATTRIBUTES` like cox, when a consumer needs them.
