# Ideas

- aulo-config adopts it: its `schema_json`, `Named`, provenance walk and `drop_null` are the origin of those parts here, and its commented `default.toml` is what `reveal` serves.
- A `show` helper that renders `Layers::leaves` as `key = value  # layer` lines, with a caller-supplied redaction list (rtok redacts `otel.headers`, cox prints sources).
- A legacy-key fold hook for `Layers` (rtok's `LegacyFold`), if a second app needs one.
