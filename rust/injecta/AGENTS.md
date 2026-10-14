# AGENTS.md — injecta

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too; on conflict, ask the creator.

## Using injecta (for agents writing application code)

The full guide with runnable examples is `GUIDE.md` (also in `llms-full.txt`). The short version:

1. Derive `Injectable` on every struct the container should build. Fields are the dependencies.
2. If a type needs a constructor, put `#[injecta::injectable]` on its `impl` and `#[inject]` on exactly one constructor that returns `Self`.
3. Declare one `injecta::container! { pub struct App { .. } }` in the assembly crate (next to `main`), not in library crates.
4. Pick the lifetime per entry:
   - `instance T`: values you already have (config, values that needed `async` or could fail). They become `App::new` arguments.
   - `singleton Arc<T>`: one shared value, created on first resolve.
   - `transient T`: a new value per resolve.
   - `scoped Arc<T>` inside `scope Name { .. }`: one value per scope (request, session).
5. Trait objects and foreign types need a factory: `singleton Arc<dyn Trait> = |c| Arc::new(c.build::<Impl>())`.
6. Get values with `container.resolve::<T>()`. In generic code, bound on `C: Provide<T>`.
7. In tests, replace singletons with `App::new(..).with(fake)`; never add a second container type just for tests.

Hard rules:

- Never store the container inside a service; inject fields.
- Never register two entries of one type; use newtypes.
- Never `unwrap` in an `#[inject]` constructor; build fallible values first and register them as `instance`.
- When the compiler says ``App` has no provider for `X` ``, add an entry for `X` (the note lists the three forms). Do not add `Option`, `Default` or a hand-written `Provide` impl to silence it.
- A `cycle detected when simplifying constant ... DEPTH` error is a dependency cycle between the listed types. Break the cycle in the design (extract the shared part into a third type); do not hide it behind a factory.

## Changing injecta (for agents working on this repository)

- Layout: `injecta` (runtime traits, storage, hooks, introspection; `no_std + alloc` without the `std` feature) and `../injecta-macros` (`container!`, `#[derive(Injectable)]`, `#[injectable]`). `bench/` is a separate, unpublished workspace that compares injecta with other DI crates; it is never a dependency of the library.
- The hot path must stay free of locks, `dyn`, `TypeId` maps and allocation beyond what the user's own types do. Any change to generated `provide` bodies needs a run of `just bench` and the numbers in the PR.
- Every misuse that can be caught at compile time gets a `tests/ui/*.rs` case with a pinned `.stderr` that contains a fix hint. Macro errors use `syn::Error::new(span, ..)` on the offending tokens, and report all errors of one invocation at once.
- Docs have one source: `GUIDE.md`. It is the crate-level rustdoc (`include_str!`), its examples are doctests, and `tests/docs.rs` derives `llms.txt` and `llms-full.txt` from it and this file. After editing either, run `just docs`.
- Before calling work done: `just check` (fmt, clippy with `-D warnings` on all targets and features, nextest, doctests, compile-fail tests, MSRV check).
- Conventional Commits in English. The human is the only author: no `Co-Authored-By` or "Generated with" lines.
