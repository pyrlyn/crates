# Ideas

- Move the `Scripted` provider here behind `test-util`, with the usage estimator injected (a closure or a small trait) instead of the coding agent's `tokens::estimate`.
- `ProviderId` still names the coding agent's backends (`Jev`, `External`). A neutral id (a string newtype) would suit aulo better, but changes the serialized form, so it needs a coding agent migration.
- `Tier`, `Job` and `Effort` are the coding agent routing labels carried by `Request` only because `Request` is the coding agent's shape; aulo may want a `Request` without them.
