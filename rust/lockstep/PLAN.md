# Lockstep plan

## 0. Research

- Study existing Rust approaches: `parking_lot` `deadlock_detection`, what
  the borrow checker can and cannot guarantee about locks, lock-free libraries
  such as crossbeam; check whether lock-ordering crates (e.g. `lock_ordering`,
  `happylock`) exist and how they work.
- Study other languages: Go channels, Erlang actors and message passing,
  Haskell STM.
- Decide which approach fits Lockstep: global lock order, lock-free, or both.
- Deliverable: `docs/research.md` with the decision.

## 1. Lock acquisition API with a global order

Levels (ranks); compile-time checking via typestate or const generics where
possible; runtime check in debug builds.

## 2. Mutex and RwLock wrappers with order checking

`OrderedMutex`, `OrderedRwLock` and their guards.

## 3. Lock-free queue and stack

## 4. Detection of locks outside the crate, or warnings

Clippy `disallowed-types` for `std::sync::Mutex`/`RwLock` and `parking_lot`;
debug-mode checks.

## 5. Built-in runtime deadlock / order-violation detector

Fires when acquisition order is violated. On by default in debug builds,
behind a feature flag otherwise; configurable panic or log.

## 6. Tests for absence of deadlocks and order correctness

loom model tests and stress tests.

## 7. Deadlock scenario tests

Classic cases that deadlock without Lockstep: AB/BA with two mutexes, dining
philosophers, a lock cycle across three threads, RwLock upgrade. Run each with
`std`/`parking_lot` and a timeout to prove the deadlock, then the same with
Lockstep to prove it is impossible (compile error or runtime rejection).

## 8. Documentation with examples
