# keychain-secret

Application secrets from an environment variable or the OS keychain, never a
config file:

- `resolve(env_var, account, store)`: the variable when set and not blank,
  else the store; `None` when neither has one, so the caller decides whether
  that is an error;
- `Keychain`: Keychain on macOS, Credential Manager on Windows, the Secret
  Service on Linux (keyring 4); `os_store(service, switch_var)` returns
  `NoStore` when the switch variable says `off`, so development builds never
  raise a keychain prompt;
- `Secret`: redacted `Debug`, no `Display`, `reveal()` to use it and
  `redacted()` (`…abcd`) for diagnostics;
- `reject_inline_secrets(toml_text, origin)`: refuses a config with a
  secret-named key at any depth (`api_key`, `password`, `*_api_key`, …);
- `MemoryStore` for tests, and `guard::violations(root, calls)` for one test
  in your suite that fails when any test reaches the real keychain.

```rust
use keychain_secret::{os_store, resolve};

let store = os_store("myapp", "MYAPP_KEYCHAIN");
match resolve("OPENAI_API_KEY", "openai", store.as_ref())? {
    Some(found) => client.bearer(found.secret.reveal()),
    None => return Err("set OPENAI_API_KEY or store the key in the keychain".into()),
}
```

Extracted from runa's `runa-cloud` (`secrets.rs`), cox's `cox-provider-http`
key lookup and `no_real_keychain_in_tests` guard, and aulo's token store.

Licensed under either of MIT or Apache-2.0 at your option.
