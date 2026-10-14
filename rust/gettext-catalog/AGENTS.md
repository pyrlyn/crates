# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

gettext catalogs for an application's own strings: `.po` parsing (polib), the
`Plural-Forms` evaluator, named placeholders, language negotiation and the
per-message fallback chain. Extracted from cox's `cox-i18n`; cox and Mailune
(F10) are its consumers. It knows nothing about any one application: the
locales, the default locale and constants such as a product name are passed in.
A process-wide localizer, a `tr!` macro and native-catalog export
(`.strings`, `.resw`) stay in the application.

## Rules

- A missing or broken translation never blanks a string: it falls back message
  by message, and an unknown id renders as itself. Keep that order
  (translation, default `msgstr`, source text, id).
- The plural grammar is GNU gettext's C subset; `src/plural.rs` tests it
  against CLDR for ru and uk. Do not widen it without a test.
- Dependency versions follow cox's lock. Bump them only with the creator.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
