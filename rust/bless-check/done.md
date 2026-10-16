# bless-check — completed tasks

### T1. Check or bless a committed generated file

Extracted from the drift checks that Scull (`crates/scull-ffi/tests/bindings.rs`, `tools/scull-ucd-gen` `--check`), rtok, cox, ketch, aulo and weft each wrote by hand. `check_or_bless(path, rendered, mode)` compares with CRLF read as LF and, by `Mode` (`Check`, `CreateMissing`, `Bless`), fails with `Error::Missing` / `Error::Differs` (first differing line, an excerpt of each side) or writes the file, creating its directories; a fresh file is never rewritten. `assert_fresh` panics with the error and a regeneration hint; `Mode::from_env` reads a `*_BLESS` switch. Std only. Scull switched to it. Check: `cargo test -p bless-check` (12 tests) and clippy green; builds on Rust 1.86.
