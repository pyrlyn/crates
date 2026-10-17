# Ideas

- Graph export (`App::graph()` as DOT/Mermaid) from the generated `PROVIDERS` plus per-entry dependency lists.
- `Lazy<T>` dependency wrapper that resolves on first use, for expensive optional services.
- `Vec<Arc<dyn Trait>>` multi-bindings collected from several entries (`singleton Arc<dyn Plugin> += ...`).
- Framework adapters: axum `FromRequestParts` for scopes, a tower layer that opens a `Request` scope.
- A `cargo xtask` that turns compile errors from `container!` into a one-line agent hint.
