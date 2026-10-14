# sccache-r2

Wire [sccache](https://github.com/mozilla/sccache) to a Cloudflare R2 bucket from
environment variables. It validates the config, prints or exports the env sccache needs,
checks that the bucket answers, and prints stats. When the credentials are missing (a fork
pull request, Dependabot, a repository without the secrets) it clears `RUSTC_WRAPPER` so
the build runs without a cache instead of failing.

```bash
cargo install sccache-r2
```

## Input

| Variable | Meaning |
| --- | --- |
| `R2_ACCESS_KEY_ID` (or `AWS_ACCESS_KEY_ID`) | R2 API token Access Key ID |
| `R2_SECRET_ACCESS_KEY` (or `AWS_SECRET_ACCESS_KEY`) | R2 API token Secret Access Key |
| `SCCACHE_BUCKET` | bucket name, 3-63 of `a-z`, `0-9`, `-` |
| `SCCACHE_ENDPOINT` | `https://<ACCOUNT_ID>.r2.cloudflarestorage.com` (a jurisdiction such as `.eu.` is allowed) |
| `SCCACHE_R2_READ_ONLY` | `true` reads the cache and never writes it (same as `--read-only`) |

Empty values count as missing. All four of the first rows present: the cache is on. Any
missing: it is off. A wrong endpoint or bucket name is an error (exit 1), not "off".

## Output

On: `RUSTC_WRAPPER=sccache`, `CARGO_INCREMENTAL=0`, `SCCACHE_BUCKET`, `SCCACHE_ENDPOINT`,
`SCCACHE_REGION=auto`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, plus
`SCCACHE_S3_RW_MODE=READ_ONLY` when read-only. Off: `RUSTC_WRAPPER=` only.

## Commands

| Command | Does |
| --- | --- |
| `env` | print `NAME=value` lines (credentials included: redirect, never log) |
| `shell` | print `export NAME='value'` lines: `eval "$(sccache-r2 shell)"` |
| `github` | append to `$GITHUB_ENV`, `::add-mask::` the credentials, write `enabled=true\|false` to `$GITHUB_OUTPUT` |
| `check` | restart the sccache server against the bucket (it refuses to start when the bucket is unreachable or the keys are rejected) and print stats; exit 1 when off or failing |
| `stats` | `sccache --show-stats` |

`check` and `stats` need `sccache` on `PATH`; the others do not.

## GitHub Actions

```yaml
- name: sccache configuration
  id: sccache
  env:
    R2_ACCESS_KEY_ID: ${{ secrets.R2_ACCESS_KEY_ID }}
    R2_SECRET_ACCESS_KEY: ${{ secrets.R2_SECRET_ACCESS_KEY }}
    SCCACHE_BUCKET: ${{ vars.SCCACHE_BUCKET }}
    SCCACHE_ENDPOINT: ${{ vars.SCCACHE_ENDPOINT }}
    SCCACHE_R2_READ_ONLY: ${{ github.event_name == 'pull_request' }}
  run: |
    cargo install --locked sccache-r2
    sccache-r2 github
- if: steps.sccache.outputs.enabled == 'true'
  uses: mozilla-actions/sccache-action@v0.0.11
```

## Library

```rust
use sccache_r2::{Config, Outcome};

let outcome = Config::from_env(false)?;
for (name, value) in outcome.env() {
    // export it; never print a value when Outcome::is_secret(name)
}
# Ok::<(), sccache_r2::Error>(())
```

`Config::from_lookup` takes any `Fn(&str) -> Option<String>`, which is how the tests run
without touching the process environment. `Config`'s `Debug` hides the credentials.

## License

MIT
