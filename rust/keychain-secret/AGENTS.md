# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

Application secrets: environment first, then the OS keychain (keyring 4),
never a config file. `Secret`, `SecretStore` with `Keychain`, `MemoryStore` and
`NoStore`, `resolve`, the keychain switch, `reject_inline_secrets`, and the
`guard` scanner that keeps tests off the real keychain. Extracted from runa
(`runa-cloud/src/secrets.rs`), cox (`cox-provider-http` and
`crates/cox/tests/no_real_keychain_in_tests.rs`) and aulo
(`aulo-server/src/auth/token.rs`); Mailune's C1 is the next consumer.

## Rules

- No test touches the real keychain: keyring binds to the platform store on
  first use, once per process, so tests use `MemoryStore` or `NoStore` and
  never construct `Keychain` or call `os_store`.
  `guard::tests::this_crate_keeps_its_own_tests_off_the_keychain` enforces it.
- A secret never reaches an error message, a `Debug` string or a log line:
  keyring errors keep only their message, because some carry the stored bytes
  in `Debug`.
- Removing a name from `is_secret_key` lets that key back into config files;
  the tests pin the list.
- Dependency versions follow cox's, runa's and aulo's locks. Bump them only with the creator.

## Commands

```bash
cargo test
cargo clippy --all-targets
cargo fmt
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
