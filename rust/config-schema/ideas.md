# Ideas

- aulo-config adopts it: its `schema_json`, `Named`, provenance walk and `drop_null` are the origin of those parts here, and its commented `default.toml` is what `reveal` serves.
- A `show` helper that renders `Layers::leaves` as `key = value  # layer` lines, with a caller-supplied redaction list (the token CLI redacts `otel.headers`, the coding agent prints sources).
- A legacy-key fold hook for `Layers` (the token CLI's `LegacyFold`), if a second app needs one.
