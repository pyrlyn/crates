# crates-packages

[![ci](https://github.com/pyrlyn/crates-packages/actions/workflows/ci.yml/badge.svg)](https://github.com/pyrlyn/crates-packages/actions/workflows/ci.yml) [![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=listepo_crates-packages&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=listepo_crates-packages) [![Coverage](https://sonarcloud.io/api/project_badges/measure?project=listepo_crates-packages&metric=coverage)](https://sonarcloud.io/component_measures?id=listepo_crates-packages&metric=coverage) [![Tests](https://img.shields.io/sonar/tests/listepo_crates-packages?server=https%3A%2F%2Fsonarcloud.io&compact_message)](https://sonarcloud.io/component_measures?id=listepo_crates-packages&metric=tests)

Small, focused Rust crates shared by `ketch` and `rtok`.

| Crate | What it does |
| --- | --- |
| [`agent-ext`](agent-ext) | Agent extension points: `SKILL.md` skills with a deferred skill tool, and Claude-Code-style shell hooks that fail open (not published yet) |
| [`agent-host-config`](agent-host-config) | Register and unregister an MCP server in an agent host's JSON config file, changing only our entry |
| [`agent-loop`](agent-loop) | Neutral agent turn state machine: provider stream, tool dispatch, approvals, interrupt |
| [`abi-drift`](abi-drift) | Drift tests for generated bindings: regenerate a cbindgen header or csbindgen C# file, diff it against the committed copy, bless to rewrite |
| [`cargo-changed-packages`](cargo-changed-packages) | Workspace packages a set of changed paths affects, including reverse dependencies |
| [`change-preview`](change-preview) | Preview what a command would change: diff or `--stat` of edits, size and file count of removals, totals |
| [`config-schema`](config-schema) | Schema-checked TOML config: stale JSON Schema check, figment layering with per-key provenance, comment-preserving key edits (not published yet) |
| [`file-backup`](file-backup) | Copy a file to `<name>.bak-<unix-seconds>` beside it before replacing it |
| [`gettext-catalog`](gettext-catalog) | gettext `.po` catalogs for an application's own strings: plural rules, named placeholders, language negotiation and per-message fallback |
| [`git-changed-paths`](git-changed-paths) | Paths a git working tree changed relative to a base ref |
| [`llm-catalog`](llm-catalog) | Model catalog for LLM agents: context windows, efforts, capabilities and prices merged from built-in rows, host config and a user price file |
| [`llm-http`](llm-http) | HTTP plumbing for LLM provider wires: credential lookup, auth headers, error mapping, retry with backoff, SSE framing |
| [`llm-wire`](llm-wire) | Provider contract and neutral request, event and tool types for LLM backends |
| [`llm-wire-anthropic`](llm-wire-anthropic) | The Anthropic Messages wire for LLM agent loops: request translation (cache breakpoints, thinking, effort), SSE stream parser, streaming client |
| [`llm-wire-openai`](llm-wire-openai) | OpenAI Chat Completions and Responses wires, also for every OpenAI-compatible endpoint |
| [`mcp-client-host`](mcp-client-host) | MCP client host over rmcp: `.mcp.json` discovery, fail-open connect, OAuth with an injectable token store, elicitation, deferred tools |
| [`keychain-secret`](keychain-secret) | Application secrets from an environment variable or the OS keychain, never a config file, with a test guard against the real keychain |
| [`path-gates`](path-gates) | Map changed paths to named gates by glob rules from a TOML config |
| [`perm-rules`](perm-rules) | Permission rules for tool calls: `Tool(subject)` grammar, deny/allow/ask decision order, risk fallback |
| [`proc-sandbox`](proc-sandbox) | Confine a child process: Seatbelt on macOS, bubblewrap or Landlock plus seccomp on Linux, and a path guard for workspace roots |
| [`scoped-check`](scoped-check) | Run only the check commands a change touches (binary) |
| [`telemetry-setup`](telemetry-setup) | `tracing` setup: rotating JSON log, stderr log, optional OTLP traces, secrets masked before they leave the process |
| [`sqlite-change-feed`](sqlite-change-feed) | Change feed for a SQLite database shared between processes: `PRAGMA data_version` through Diesel |
| [`wasm-plugin-host`](wasm-plugin-host) | Run WebAssembly plugins with extism: a worker thread per plugin, memory cap, per-call deadlines, control lane before events |

## Commands

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

## Release

- Pushing to `main` runs CI (commits lint on PRs, fmt, clippy, unit tests,
  publish dry-run).
- A release is `gh workflow run bump.yml` (Actions → Bump and release), the
  only way to a release and the only thing that creates a
  `<crate>-v<version>` tag; `-f package=<crate>` picks the crate. It runs `release-plz update` (version and
  changelog from the conventional commits), opens a pull request with that one
  commit, rebase-merges it once the required checks are green, tags the commit
  that landed on main, creates a draft release and dispatches `release.yml`
  on the tag: the gate again, crates.io, then the draft is published.
  Anything failing before the merge closes the pull request: no tag, no
  release. `-f dry-run=true` opens the pull request, waits for its checks and
  closes it.
- A crate is released after the sibling crates it depends on: `llm-wire` before
  `llm-http`, `llm-wire-openai`, `llm-wire-anthropic`, `llm-catalog` and
  `agent-loop`; `file-backup` before `agent-host-config`; `git-changed-paths` and
  `cargo-changed-packages` before `scoped-check` (which also needs `path-gates`). Those dependency
  lines carry both `path` and `version`, so `cargo publish` fails while a
  sibling is not on crates.io yet.
- On crates.io: `file-backup`. `change-preview` and `wasm-plugin-host` are publish-ready (CI runs their
  publish dry-run) and go out with their first bump. The other crates are not published yet.

Secrets: `CARGO_REGISTRY_TOKEN` (crates.io API token) and
`RELEASE_PLZ_TOKEN` (a PAT that can trigger workflows; bump opens the
version pull request with it so CI runs on it).

## Secrets

Two secrets must exist in the repo settings
(`Settings → Secrets and variables → Actions → New repository secret`):

| Secret | Where to get it | Used by |
| --- | --- | --- |
| `CARGO_REGISTRY_TOKEN` | crates.io → Account Settings → API Tokens → New Token (needs `publish-new` and `publish-update` scopes; restrict it to the crates listed above). Add with `gh secret set CARGO_REGISTRY_TOKEN --repo pyrlyn/crates-packages` | `release.yml`, job `crates-io`: `cargo publish -p <crate>` |
| `CARGO_REGISTRY_TOKEN` | crates.io → Account Settings → API Tokens → New Token (needs `publish-new` and `publish-update` scopes; restrict it to the `file-backup`, `change-preview`, `wasm-plugin-host`, `telemetry-setup`, `gettext-catalog`, `sqlite-change-feed`, `abi-drift` and `keychain-secret` crates; add each new crate before its first release). Add with `gh secret set CARGO_REGISTRY_TOKEN --repo pyrlyn/crates-packages` | `release.yml`, job `crates-io`: `cargo publish -p <crate>` |
| `RELEASE_PLZ_TOKEN` | A fine-grained PAT (or GitHub App token) with **Contents** and **Pull requests** read/write on this repo — see https://release-plz.dev/docs/github/token. The default `GITHUB_TOKEN` cannot trigger `release.yml` from the release PR it opens, so without this the release PR would land without CI. Add with `gh secret set RELEASE_PLZ_TOKEN --repo pyrlyn/crates-packages` | `bump.yml`: pushes the version branch and opens the version PR |

Without `CARGO_REGISTRY_TOKEN` the release stops at the `crates-io` job with
a loud error and publishes nothing — the tag is never created.
Without `RELEASE_PLZ_TOKEN` bump falls back to GITHUB_TOKEN, which cannot
open the pull request (workflow permissions), so it fails before any tag.

## License

Licensed under the [GNU General Public License v3.0 or later](LICENSE)
(`GPL-3.0-or-later`).

The crates shared with cox and aulo (`agent-ext`, `agent-host-config`,
`agent-loop`, `config-schema`, `llm-*`, `mcp-client-host`, `perm-rules`,
`proc-sandbox`, `shell-classify`, `speech-capture`, `text-sanitize`) are
offered under `GPL-3.0-or-later OR LicenseRef-Royalty-Free`: the second option
is [`LICENSE-ROYALTY-FREE.md`](LICENSE-ROYALTY-FREE.md). Each package ships
both texts (`LICENSE` and `LICENSE-ROYALTY-FREE.md` are symlinks to the
repository root files).
