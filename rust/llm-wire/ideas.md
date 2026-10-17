# Ideas

- Move the `Scripted` provider here behind `test-util`, with the usage estimator injected (a closure or a small trait) instead of cox's `tokens::estimate`.
- `ProviderId` still names cox's backends (`Jev`, `External`). A neutral id (a string newtype) would suit aulo better, but changes the serialized form, so it needs a cox migration.
- `Tier`, `Job` and `Effort` are cox routing labels carried by `Request` only because `Request` is cox's shape; aulo may want a `Request` without them.
