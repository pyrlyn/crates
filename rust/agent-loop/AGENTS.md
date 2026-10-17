# AGENTS.md

If an `AGENTS.md` or `CLAUDE.md` exists higher in the tree, follow it too. If it
conflicts with this file, ask the creator.

## What this crate is

The neutral turn loop of an LLM agent, extracted from cox's `cox-core`
(`src/session.rs` `run_turn`/`step`/`finish`, `src/turn.rs`) so cox and
aulo's `aulo-agent` share one state machine. `Submission`s go in, `Event`s
come out; the model is any `llm_wire::Provider`.

## Design

**What moved from cox-core.**

- The step loop: one provider call, then its tool batch, until the model
  stops calling tools (`EndTurn`), `Config::max_steps` calls are used up
  (`MaxTurns`), the provider fails (`Error`), the `Context` stops the turn,
  or an interrupt fires (`Interrupted`). Same event order as cox.
- The stream consumer (`consume_provider`): text and thinking deltas, a
  thought's own `Thinking` item closed before the reply's item, tool-use
  accumulation, thought signatures by call id.
- Tool dispatch (`run_signed_tools`): every `ToolCallRequested` first
  (a signed call's empty signed `Thinking` item before it), serial gating,
  exclusive calls one at a time, then parallel ones up to
  `Config::parallel_tools`, results in request order in one message.
- Approvals (`gate`/`ask`): verdict, `ApprovalRequired`, park until
  `Submission::Approve`, `ApprovalDecided`; `AllowForSession` grants; an
  `Edit` goes back through the gate; an interrupt answers `Deny`.
- Interrupt: one `CancellationToken` per turn, shared by the provider
  stream, the tools and the approval wait.
- The neutral subset of `Submission`/`Event`/`ToolCall`/`ToolResult`/
  `Decision`/`Why`, with cox's serde tags.

**Trait seams** (`src/traits.rs`), all held as `Arc<dyn _>` in `Parts`:

| Seam | Does | cox implements it with |
| --- | --- | --- |
| `llm_wire::Provider` | Streams one model call | its wires |
| `Tools` | Rates a call (risk, subject, segments), says if it may run in parallel, runs it | its tool registry, checkpoints, sandbox retry, archive, dedup, truncation, images |
| `Approvals` | `prepare` (hooks, risk advice), `decide` (rules, mode, grants), `grant`, `asking` | `cox-permission`, the `PreToolUse`/`PermissionRequest` hooks, advisors |
| `Context` | User message content, request assembly (may rewrite history), one `recover` per turn, `on_usage`, `on_results` | prompts, routing, budget, compaction, microcompaction, ledger, cache diagnostics, tool images |
| `EventSink` | Takes every event | the rollout, redaction, telemetry, notifications, surfaces |

**What stays in cox:** routing and tiers, budget, compaction, repo map,
prompts and skills, memory, checkpoints and rewind, subagents and tasks,
hooks, sandbox, archive and dedup, telemetry spans, the full `Event` enum.

**Deliberate differences from cox-core** (proposed for cox in T1.16):

1. An interrupt mid-stream ends the turn `Interrupted` at once, without
   waiting on the provider; cox waits for the provider's `Cancelled` error
   and ends `Error`.
2. A call that has not started when the interrupt fires never runs.
3. An interrupted tool round still records its results in history, so no
   `ToolUse` is left without a `ToolResult`.
4. `Config::approval_timeout` (a deny by `Policy`) and `Config::tool_output_cap`
   (a hard cap on what the model sees of untrusted tool output) are new.

## Rules

- **Behaviour-neutral with cox.** Keep cox's event order and serde shapes for
  the types here; a change goes into cox too.
- **No tool runs before an allow**, and none starts after an interrupt.
- **No `unwrap`, `expect` or `panic!` outside tests** (clippy denies them).
- **Doc comments on every public item** (`missing_docs` is on).
- **Published to crates.io** from this repository: a dependency on a sibling crate
  carries both `path` and `version`, so a release publishes the siblings first.

## Commands

```bash
cargo test -p agent-loop
cargo clippy -p agent-loop --all-targets -- -D warnings
cargo fmt
```

`just test` runs the tests and finishes with a lossless `swarfr` cleanup of the target dir.
