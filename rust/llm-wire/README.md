# llm-wire

The provider contract for LLM agent loops.

- `Provider`: the one trait a model backend implements. `stream` takes a
  `Request` and forwards `ProviderEvent`s on a channel until the call ends or
  is cancelled; `capabilities` tells the loop what the backend supports.
- `Request`, `Message`, `Content`, `ToolSpec`: what the loop sends, in a
  neutral shape a wire crate translates to its own format.
- `ProviderEvent`, `Usage`, `StopReason`, `ProviderError`: what comes back.
- `Risk`, `Concurrency`, `Tier`, `Job`, `Effort`: the labels a request and a
  tool carry.

- `ProviderModel`, `Capabilities`: a configured model entry and what it
  supports, shared by the wires and the catalog.
- `effort_for`, `Api`, `WireEffort`, `supports_adaptive_thinking`: the one
  rule for which effort a wire sends.
- `ulid_id!`: declares a ULID newtype id; `CallId` and `ArchiveId` use it.

Everything serializes with serde and derives `JsonSchema`.

Feature `test-util` adds `test_util::scripted` (a TOML scenario format and the
events a turn expands to) and `test_util::replay` (cassette hashing, secret
redaction and writing) for building `Provider` test doubles.

```rust
use llm_wire::{Provider, ProviderId};

fn is_local(provider: &dyn Provider) -> bool {
    provider.id() == ProviderId::Local
}
```

Licensed `GPL-3.0-or-later OR LicenseRef-Royalty-Free`; see the repository's
`LICENSE` and `LICENSE-ROYALTY-FREE.md`.
