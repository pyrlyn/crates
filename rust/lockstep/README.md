# lockstep

Locks that cannot deadlock: one global acquisition order, or lock-free
structures. **Skeleton only; nothing is implemented yet.** See `plan.md`.

## How it works

Every lock has a level. A thread may only take a lock above every level it
already holds, so no cycle of waiting threads can form. The order is checked
at compile time where possible and at run time in debug builds. Where no lock
is needed, lock-free queue and stack types are offered instead.

The guarantee holds only if all locking goes through this crate. One lock
taken outside it can break the order.

## Enable

Planned: add `lockstep` as a dependency, replace `Mutex`/`RwLock` with
`OrderedMutex`/`OrderedRwLock`, and enable the clippy `disallowed-types`
config so other locks are rejected.

## Disable

Planned: the runtime order detector is on in debug builds and can be turned
off or switched to log-only through a feature flag. Compile-time checks
cannot be disabled.

## Test

```sh
cargo test -p lockstep
```

Planned: loom model tests and stress tests (see `plan.md`).

## Limitations

- Locks outside the crate are not covered.
- Not published (`publish = false`).
