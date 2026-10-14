# agent-loop

A neutral LLM agent turn loop: submissions in, events out.

`Agent::submit(Submission::UserTurn { .. })` runs one turn: a provider call,
then the tool calls it asked for, again and again until the model answers
without tools, the step limit is hit or the turn is interrupted. Every step
is reported on an `EventSink` as `Event`s (text deltas, tool calls,
approvals, usage, `TurnDone`).

- Tools run through a `Tools` executor. Exclusive calls run one at a time,
  parallel calls up to `Config::parallel_tools` at once, and results come
  back in the order the model asked for them.
- Every call passes an `Approvals` gate first. The gate allows, denies or
  asks; asking emits `ApprovalRequired` and waits for `Submission::Approve`.
  No tool runs before an allow.
- `Agent::interrupt` (or `Submission::Interrupt`) cancels the provider
  stream, the running tools and any approval wait at once, so a voice
  barge-in or a kill switch ends the turn mid-reply.
- Tool output is untrusted: what the model sees is capped at
  `Config::tool_output_cap`.
- A `Context` builds each request from the history and hooks into the
  turn (usage, results, one recovery after a retryable provider error).

The model is any `llm_wire::Provider`.

```rust
use agent_loop::{Agent, Config, Parts, Submission};

async fn ask(parts: Parts) -> Result<(), agent_loop::LoopError> {
    let agent = Agent::new(parts, Config::default());
    agent
        .submit(Submission::UserTurn { text: "hi".into(), attachments: vec![] })
        .await
}
```

Extracted from cox's `cox-core`; see `AGENTS.md` for what moved and what
stays in cox.

Licensed `GPL-3.0-or-later OR LicenseRef-Royalty-Free`; see the repository's
`LICENSE` and `LICENSE-ROYALTY-FREE.md`.
