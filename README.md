# crates

[![ci](https://github.com/pyrlyn/crates/actions/workflows/ci.yml/badge.svg)](https://github.com/pyrlyn/crates/actions/workflows/ci.yml) [![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=listepo_crates-packages&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=listepo_crates-packages) [![Coverage](https://sonarcloud.io/api/project_badges/measure?project=listepo_crates-packages&metric=coverage)](https://sonarcloud.io/component_measures?id=listepo_crates-packages&metric=coverage) [![Tests](https://img.shields.io/sonar/tests/listepo_crates-packages?server=https%3A%2F%2Fsonarcloud.io&compact_message)](https://sonarcloud.io/component_measures?id=listepo_crates-packages&metric=tests)

Small, focused Rust crates shared by the creator's projects (`ketch`, `rtok`, `scull` and others).

| Crate | What it does |
| --- | --- |
| [`agent-ext`](rust/agent-ext) | Agent extension points: `SKILL.md` skills with a deferred skill tool, and Claude-Code-style shell hooks that fail open (not published yet) |
| [`agent-host-config`](rust/agent-host-config) | Register and unregister an MCP server in an agent host's JSON config file, changing only our entry |
| [`agent-loop`](rust/agent-loop) | Neutral agent turn state machine: provider stream, tool dispatch, approvals, interrupt |
| [`abi-drift`](rust/abi-drift) | Drift tests for generated bindings: regenerate a cbindgen header or csbindgen C# file, diff it against the committed copy, bless to rewrite |
| [`app-home`](rust/app-home) | Resolve the user home, `$APP_HOME` or `~/.app`, and XDG config/data/cache/state dirs the same way on every OS |
| [`cargo-changed-packages`](rust/cargo-changed-packages) | Workspace packages a set of changed paths affects, including reverse dependencies |
| [`change-preview`](rust/change-preview) | Preview what a command would change: diff or `--stat` of edits, size and file count of removals, totals |
| [`operation-icons`](rust/operation-icons) | Emoji icons for the verb of a status line and the fixed-width gutter they sit in |
| [`owner-listener`](rust/owner-listener) | Owner-only Unix socket and Windows named pipe for a local gRPC agent, with tonic `Connected` peer info and login-service install (not published) |
| [`config-schema`](rust/config-schema) | Schema-checked TOML config: stale JSON Schema check, figment layering with per-key provenance, comment-preserving key edits (not published yet) |
| [`d2-render`](rust/d2-render) | Render D2 diagrams to SVG/PNG in pure Rust, with an optional d2 CLI backend, structured diagnostics and freshness checks |
| [`fs-kit`](rust/fs-kit) | One entry point for file and directory work: `\\?\`-free paths, atomic writes, tree usage, copy and removal, backups |
| [`file-backup`](rust/file-backup) | Copy a file to `<name>.bak-<unix-seconds>` beside it before replacing it |
| [`gettext-catalog`](rust/gettext-catalog) | gettext `.po` catalogs for an application's own strings: plural rules, named placeholders, language negotiation and per-message fallback |
| [`git-changed-paths`](rust/git-changed-paths) | Paths a git working tree changed relative to a base ref |
| [`llm-catalog`](rust/llm-catalog) | Model catalog for LLM agents: context windows, efforts, capabilities and prices merged from built-in rows, host config and a user price file |
| [`llm-http`](rust/llm-http) | HTTP plumbing for LLM provider wires: credential lookup, auth headers, error mapping, retry with backoff, SSE framing |
| [`llm-wire`](rust/llm-wire) | Provider contract and neutral request, event and tool types for LLM backends |
| [`llm-wire-anthropic`](rust/llm-wire-anthropic) | The Anthropic Messages wire for LLM agent loops: request translation (cache breakpoints, thinking, effort), SSE stream parser, streaming client |
| [`llm-wire-openai`](rust/llm-wire-openai) | OpenAI Chat Completions and Responses wires, also for every OpenAI-compatible endpoint |
| [`mcp-client-host`](rust/mcp-client-host) | MCP client host over rmcp: `.mcp.json` discovery, fail-open connect, OAuth with an injectable token store, elicitation, deferred tools |
| [`keychain-secret`](rust/keychain-secret) | Application secrets from an environment variable or the OS keychain, never a config file, with a test guard against the real keychain |
| [`injecta`](rust/injecta) | Compile-time dependency injection: one `container!` declaration, derive `Injectable`, no runtime registry |
| [`injecta-macros`](rust/injecta-macros) | Procedural macros behind `injecta` (`container!`, `#[derive(Injectable)]`, `#[injectable]`) |
| [`path-gates`](rust/path-gates) | Map changed paths to named gates by glob rules from a TOML config |
| [`plugwright`](rust/plugwright) | Plugin manager for Rust hosts: native (in-process or isolated) and Wasm (wasmtime, wasmi, wasmer) plugins from one manifest, cached builds, `#[plugin]`/`#[interface]` macros (crates under `rust/plugwright/crates`; `plugwright-wasmer` stays outside the workspace and unpublished) |
| [`perm-rules`](rust/perm-rules) | Permission rules for tool calls: `Tool(subject)` grammar, deny/allow/ask decision order, risk fallback |
| [`sccache-r2`](rust/sccache-r2) | Wire sccache to a Cloudflare R2 bucket from env vars: validate, export (GitHub Actions or shell), check the bucket, print stats; off without the secrets |
| [`proc-sandbox`](rust/proc-sandbox) | Confine a child process: Seatbelt on macOS, bubblewrap or Landlock plus seccomp on Linux, and a path guard for workspace roots |
| [`rgba-png`](rust/rgba-png) | RGBA8 frames and PNG files: encode, decode any PNG to RGBA under an allocation cap, per-pixel comparison with a threshold, diff images (not published) |
| [`scoped-check`](rust/scoped-check) | Run only the check commands a change touches (binary) |
| [`telemetry-setup`](rust/telemetry-setup) | `tracing` setup: rotating JSON log, stderr log, optional OTLP traces, secrets masked before they leave the process |
| [`sqlite-change-feed`](rust/sqlite-change-feed) | Change feed for a SQLite database shared between processes: `PRAGMA data_version` through Diesel |
| [`sqlitex-graph`](rust/sqlx/sqlitex-graph) | Code-symbol store in SQLite with a native reference/call graph in CSR segments: lock-free readers, delta writes with background merge, `graph_impact`/`graph_callers`/`graph_callees` table-valued functions, also as a loadable extension |
| [`wasm-plugin-host`](rust/wasm-plugin-host) | Run WebAssembly plugins with extism: a worker thread per plugin, memory cap, per-call deadlines, control lane before events |
| [`workspace-graph`](rust/workspace-graph) | Load a workspace's direct dependency graph and assert rules over it in tests (not published) |
| [`atomic-replace`](atomic-replace) | Replace a file atomically: sibling temp file, kept permissions and symlinks, optional fsync, rename |
| [`cargo-changed-packages`](cargo-changed-packages) | Workspace packages a set of changed paths affects, including reverse dependencies |
| [`change-preview`](change-preview) | Preview what a command would change: diff or `--stat` of edits, size and file count of removals, totals |
| [`file-backup`](file-backup) | Copy a file to `<name>.bak-<unix-seconds>` beside it before replacing it |
| [`git-changed-paths`](git-changed-paths) | Paths a git working tree changed relative to a base ref |
| [`path-gates`](path-gates) | Map changed paths to named gates by glob rules from a TOML config |
| [`scoped-check`](scoped-check) | Run only the check commands a change touches (binary) |

## Commands

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

## Release

Any workspace crate without `publish = false` can be released, one crate per
run, each with its own version, `rust/<crate>/CHANGELOG.md` and
`<crate>-v<version>` tag.

- Pushing to `main` runs CI (commits lint on PRs, fmt, clippy, unit tests, and
  publish dry-runs for the release-ready crates).
- A release is `gh workflow run bump.yml -f package=<crate>` (Actions → Bump and
  release), the only way to a release and the only thing that creates a
  `<crate>-v<version>` tag. First it checks that the crate is a workspace
  member, is publishable and that the workspace crates it depends on are already
  on crates.io. It runs `release-plz update` (version and changelog from the
  conventional commits), opens a pull request with that one commit, rebase-merges
  it once the required checks are green, tags the commit that landed on main,
  creates a draft release and dispatches `release.yml` on the tag: the tag must
  name a publishable crate and the version in its `Cargo.toml`, then the gate
  runs again, `cargo publish -p <crate> --locked` goes to crates.io, and the
  draft is published. Anything failing before the merge closes the pull
  request: no tag, no release. `-f dry-run=true` opens the pull request, waits
  for its checks and closes it.
- A crate that has never been tagged is released at the version in its
  `Cargo.toml` (`0.1.0`), with a new changelog. A crate that must not be
  published says `publish = false`.

### Selective publishing

`publish-crates.yml` publishes with [cargo-workspaces](https://github.com/pksunkara/cargo-workspaces)
(`cargo:cargo-workspaces` in `mise.toml`). `cargo ws publish --publish-as-is`
uploads, in dependency order, only the crates whose `Cargo.toml` version is not
on crates.io yet; an unchanged (not bumped) crate is skipped. To release a
change, bump the crate in a pull request (`cargo ws version`, or by hand) and
merge it.

- Pull request: lists changed crates (`cargo ws changed --since origin/main`) in
  the job summary and runs `cargo ws publish --dry-run --no-verify`. Nothing is
  uploaded.
- Push to `main`: the same listing against the previous commit, then the
  publish. It is real only when the repository variable `CRATES_PUBLISH` is
  `true` (otherwise a dry run); `CARGO_REGISTRY_TOKEN` is read from the
  environment. Manual run: Actions → publish-crates, `dry-run: false`.
- `cargo ws changed` diffs against the last git tag, and the existing
  `<crate>-v<version>` tags predate most crates, so without `--since` it lists
  nearly everything. Use it as a report; the registry decides what is published.
  Tags are still created only by `bump.yml`.

### Release order

The shared bump workflow currently runs bare `release-plz update`, which can
update other crates in the same commit. It must use `release-plz update -p
<crate>` before releasing from this multi-crate workspace.

A crate is released only after every workspace crate it depends on (normal,
build, and dev-dependencies that carry a version) is on crates.io at the version
in this repository: crates.io resolves the dependencies when a crate is
published. Both workflows check it and stop before a pull request or a publish.
The rest of the order is free, so release the crates others wait for first.

plugwright's crates, a step at a time (any order inside a step; the steps follow
from their normal, build and versioned dev-dependencies):

1. `plugwright-abi`, `plugwright-macros`, `plugwright-packages`, `plugwright-sandbox`
2. `plugwright-core`, `plugwright-sdk`
3. `plugwright-build`, `plugwright-extism`, `plugwright-native`, `plugwright-trust`, `plugwright-wasm`
4. `plugwright-isolate`, `plugwright-wasmi`, `plugwright-wasmtime`
5. `plugwright-loader`, `plugwright-runner`
6. `plugwright`
7. `cargo-plugwright`

That is 18 new crates: past the burst of 5 below, a new crate every 10 minutes.

crates.io limits how fast new crates appear: a burst of 5 new crates, then one
every 10 minutes; new versions of existing crates have a burst of 30 and one per
minute (`default_burst` and `default_rate_seconds` in
[`src/rate_limiter.rs`](https://github.com/rust-lang/crates.io/blob/29692b85b6e7506f5cf504ad5e2eb05ba36a8e05/src/rate_limiter.rs),
read 2026-10-07; the registry operator can change the defaults or raise them for
one account, which this file cannot show). A first release past the burst fails
at `crates-io` with an HTTP 429: re-run `release.yml` for the same tag once the
window has passed.

Secrets: `CARGO_REGISTRY_TOKEN` (crates.io API token) and
`RELEASE_PLZ_TOKEN` (a PAT that can trigger workflows; bump opens the
version pull request with it so CI runs on it).

## Secrets

Two secrets must exist in the repo settings
(`Settings → Secrets and variables → Actions → New repository secret`):

| Secret | Where to get it | Used by |
| --- | --- | --- |
| `CARGO_REGISTRY_TOKEN` | crates.io → Account Settings → API Tokens → New Token (needs `publish-new` and `publish-update` scopes; if you restrict it by crate, use name patterns such as `llm-*` or list every crate released from here, since a new crate has no name to pick yet). Add with `gh secret set CARGO_REGISTRY_TOKEN --repo pyrlyn/crates` | `release.yml`, job `crates-io`: `cargo publish -p <crate>` |
| `RELEASE_PLZ_TOKEN` | A fine-grained PAT (or GitHub App token) with **Contents** and **Pull requests** read/write on this repo — see https://release-plz.dev/docs/github/token. The default `GITHUB_TOKEN` cannot trigger `release.yml` from the release PR it opens, so without this the release PR would land without CI. Add with `gh secret set RELEASE_PLZ_TOKEN --repo pyrlyn/crates` | `bump.yml`: pushes the version branch and opens the version PR |

Without `CARGO_REGISTRY_TOKEN` the release stops at the `crates-io` job with
a loud error and publishes nothing; the tag and the draft release already exist,
so re-run `release.yml` for that tag once the secret is set.
Without `RELEASE_PLZ_TOKEN` bump falls back to GITHUB_TOKEN, which cannot
open the pull request (workflow permissions), so it fails before any tag.

## License

Licensed under the [GNU General Public License v3.0 or later](LICENSE)
(`GPL-3.0-or-later`).

The crates shared with cox and aulo (`agent-ext`, `agent-host-config`,
`agent-loop`, `config-schema`, `llm-*`, `mcp-client-host`, `owner-listener`, `perm-rules`,
`proc-sandbox`, `shell-classify`, `speech-capture`, `text-sanitize`) are
offered under `GPL-3.0-or-later OR LicenseRef-Royalty-Free`: the second option
is [`LICENSE-ROYALTY-FREE.md`](LICENSE-ROYALTY-FREE.md). Each package ships
both texts (`LICENSE` and `LICENSE-ROYALTY-FREE.md` are symlinks to the
repository root files).
