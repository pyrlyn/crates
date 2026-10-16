//! The turn state machine: `Agent::submit` takes a `Submission` and drives
//! the turn it starts as `step`s, one provider call and its tool batch
//! each, until `TurnDone`. I/O happens only through the seams in
//! [`crate::traits`]; the agent owns history, ordering and cancellation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use llm_wire::{CallId, Content, Message, Provider, Role, StopReason};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::dispatch;
use crate::ids::{ItemId, TurnId};
use crate::stream::{self, Consumed};
use crate::traits::{Approvals, Context, EventSink, Next, Tools};
use crate::types::{Attachment, Decision, Event, ItemKind, Level, LoopError, State, Submission};

/// Limits for one agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Provider calls allowed per user turn (cox: `core.max_turns`); the
    /// turn ends `MaxTurns` when they are used up.
    pub max_steps: u32,
    /// Most `Concurrency::Parallel` calls running at once; `0` means 1.
    pub parallel_tools: u32,
    /// How long an approval may wait before it counts as a deny; `None`
    /// waits until answered or interrupted.
    pub approval_timeout: Option<Duration>,
    /// Most bytes of a tool's `visible` output the model sees; the rest is
    /// cut at a char boundary and replaced by a one-line trailer.
    pub tool_output_cap: usize,
}

impl Default for Config {
    /// cox's defaults; the output cap is a backstop well above cox's own
    /// 8 KiB truncation.
    fn default() -> Self {
        Self {
            max_steps: 200,
            parallel_tools: 4,
            approval_timeout: None,
            tool_output_cap: 64 * 1024,
        }
    }
}

/// What the agent is built from.
pub struct Parts {
    /// The model.
    pub provider: Arc<dyn Provider>,
    /// The tool executor.
    pub tools: Arc<dyn Tools>,
    /// The approval gate.
    pub approvals: Arc<dyn Approvals>,
    /// Request assembly and the hooks around a provider call.
    pub context: Arc<dyn Context>,
    /// Where events go.
    pub sink: Arc<dyn EventSink>,
}

pub(crate) struct Shared {
    pub(crate) parts: Parts,
    pub(crate) config: Config,
    history: Mutex<Vec<Message>>,
    /// Held for a whole turn, so turns run one at a time; the count is the
    /// last turn's `seq`.
    turns: Mutex<u32>,
    state: StdMutex<State>,
    /// Calls parked in `AwaitingApproval`, answered by `Submission::Approve`.
    pending: StdMutex<HashMap<CallId, oneshot::Sender<Decision>>>,
    cancel: StdMutex<CancellationToken>,
}

impl Shared {
    pub(crate) async fn emit(&self, event: Event) -> Result<(), LoopError> {
        self.parts.sink.emit(event).await
    }

    pub(crate) fn cancel_token(&self) -> CancellationToken {
        self.cancel
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(crate) fn state(&self) -> State {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn set_state(&self, state: State) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }

    pub(crate) fn park(&self, call: CallId) -> oneshot::Receiver<Decision> {
        let (tx, rx) = oneshot::channel();
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.insert(call, tx);
        rx
    }

    pub(crate) fn unpark(&self, call: CallId) -> Option<oneshot::Sender<Decision>> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.remove(&call)
    }
}

enum Step {
    Continue,
    Done,
}

/// The agent: one conversation's turn loop. Cheap to clone; clones share
/// the conversation, so one task can run a turn while another submits
/// `Approve` or `Interrupt`.
#[derive(Clone)]
pub struct Agent {
    shared: Arc<Shared>,
}

impl Agent {
    /// An agent with empty history.
    pub fn new(parts: Parts, config: Config) -> Self {
        Self::with_history(parts, config, Vec::new())
    }

    /// An agent that continues `history` (a resumed conversation).
    pub fn with_history(parts: Parts, config: Config, history: Vec<Message>) -> Self {
        Self {
            shared: Arc::new(Shared {
                parts,
                config,
                history: Mutex::new(history),
                turns: Mutex::new(0),
                state: StdMutex::new(State::Idle),
                pending: StdMutex::new(HashMap::new()),
                cancel: StdMutex::new(CancellationToken::new()),
            }),
        }
    }

    /// The conversation so far.
    pub async fn history(&self) -> Vec<Message> {
        self.shared.history.lock().await.clone()
    }

    /// Where the running turn is.
    pub fn state(&self) -> State {
        self.shared.state()
    }

    /// Cancels the running turn's provider stream, tools and approval
    /// wait. Synchronous so a barge-in or kill switch acts at once.
    pub fn interrupt(&self) {
        self.shared.cancel_token().cancel();
    }

    /// Feeds one submission into the state machine. A `UserTurn` returns
    /// once its `TurnDone` is emitted; the others return at once.
    pub async fn submit(&self, sub: Submission) -> Result<(), LoopError> {
        match sub {
            Submission::UserTurn { text, attachments } => self.run_turn(text, attachments).await,
            Submission::Interrupt => {
                self.interrupt();
                Ok(())
            }
            Submission::Approve { call_id, decision } => match self.shared.unpark(call_id) {
                Some(tx) => {
                    let _ = tx.send(decision);
                    Ok(())
                }
                None => {
                    self.shared
                        .emit(Event::Notice {
                            level: Level::Warn,
                            text: format!("no approval pending for call {call_id}"),
                        })
                        .await
                }
            },
        }
    }

    async fn run_turn(&self, text: String, attachments: Vec<Attachment>) -> Result<(), LoopError> {
        let shared = &self.shared;
        let mut turns = shared.turns.lock().await;
        // A fresh token: the last turn's interrupt may have left the old
        // one cancelled.
        *shared.cancel.lock().unwrap_or_else(|e| e.into_inner()) = CancellationToken::new();
        *turns += 1;
        let seq = *turns;
        let turn = TurnId::new();
        let user_item = ItemId::new();
        shared.set_state(State::Assembling);
        let content = shared.parts.context.user_content(&text, &attachments);
        shared.history.lock().await.push(Message {
            role: Role::User,
            content,
        });
        shared.emit(Event::TurnStarted { turn, seq }).await?;
        shared
            .emit(Event::ItemStarted {
                item: user_item,
                kind: ItemKind::UserMessage { text, attachments },
            })
            .await?;
        shared.emit(Event::ItemDone { item: user_item }).await?;
        let mut steps = 0;
        let mut retried = false;
        loop {
            match self.step(turn, &mut steps, &mut retried).await? {
                Step::Continue => {}
                Step::Done => return Ok(()),
            }
        }
    }

    /// One provider call and its tool batch.
    async fn step(
        &self,
        turn: TurnId,
        steps: &mut u32,
        retried: &mut bool,
    ) -> Result<Step, LoopError> {
        let shared = &self.shared;
        let cancel = shared.cancel_token();
        if cancel.is_cancelled() {
            return self.interrupted(turn).await;
        }
        if *steps >= shared.config.max_steps {
            return self.finish(turn, StopReason::MaxTurns).await;
        }
        shared.set_state(State::Assembling);
        *steps += 1;
        let next = {
            let mut history = shared.history.lock().await;
            shared
                .parts
                .context
                .request(turn, *steps, &mut history)
                .await
        };
        let req = match next {
            Next::Send(req) => req,
            Next::Stop(stop) => return self.finish(turn, stop).await,
        };
        let assistant_item = ItemId::new();
        shared.set_state(State::Streaming);
        let (ptx, mut prx) = mpsc::channel(64);
        let provider = shared.parts.provider.clone();
        let provider_cancel = cancel.clone();
        let mut join =
            tokio::spawn(async move { provider.stream(req, ptx, provider_cancel).await });
        let sink = shared.parts.sink.as_ref();
        let streamed = match stream::consume(sink, &mut prx, assistant_item, &cancel).await? {
            Consumed::Done(streamed) => streamed,
            Consumed::Cancelled => {
                // Never wait on a provider that ignores its token: an
                // interrupt must end the turn now.
                join.abort();
                return self.interrupted(turn).await;
            }
            Consumed::Failed(error) => {
                let _ = join.await;
                return self.failed(turn, error).await;
            }
        };
        let usage = match (&mut join).await {
            Ok(Ok(usage)) => usage,
            Ok(Err(_)) if cancel.is_cancelled() => return self.interrupted(turn).await,
            Ok(Err(error)) => {
                if !*retried && shared.parts.context.retryable(&error) {
                    shared
                        .emit(Event::ItemDone {
                            item: assistant_item,
                        })
                        .await?;
                    *retried = true;
                    let mut history = shared.history.lock().await;
                    if shared.parts.context.recover(&error, &mut history).await {
                        return Ok(Step::Continue);
                    }
                }
                return self.failed(turn, error).await;
            }
            Err(_) => return self.interrupted(turn).await,
        };
        let usage = streamed.usage.unwrap_or(usage);
        shared.parts.context.on_usage(turn, &usage).await?;
        shared.emit(Event::Usage { turn, usage }).await?;
        shared
            .emit(Event::ItemDone {
                item: assistant_item,
            })
            .await?;

        if streamed.calls.is_empty() {
            if !streamed.text.is_empty() {
                shared.history.lock().await.push(Message {
                    role: Role::Assistant,
                    content: vec![Content::Text {
                        text: streamed.text,
                    }],
                });
            }
            return self.finish(turn, StopReason::EndTurn).await;
        }

        shared
            .history
            .lock()
            .await
            .push(assistant_message(&streamed));
        shared.set_state(State::RunningTools);
        let results =
            dispatch::run_batch(shared, turn, streamed.calls, &streamed.signatures).await?;
        // Pushed even when interrupted: every `ToolUse` in history keeps
        // its `ToolResult`, or the next request would be malformed.
        let mut msg = dispatch::results_message(results);
        shared.parts.context.on_results(&mut msg).await;
        shared.history.lock().await.push(msg);
        if cancel.is_cancelled() {
            return self.interrupted(turn).await;
        }
        Ok(Step::Continue)
    }

    async fn failed(
        &self,
        turn: TurnId,
        error: llm_wire::ProviderError,
    ) -> Result<Step, LoopError> {
        self.shared
            .emit(Event::Error {
                error: LoopError::Provider { error },
                fatal: false,
            })
            .await?;
        self.finish(turn, StopReason::Error).await
    }

    async fn interrupted(&self, turn: TurnId) -> Result<Step, LoopError> {
        self.shared.set_state(State::Interrupted);
        self.finish(turn, StopReason::Interrupted).await
    }

    async fn finish(&self, turn: TurnId, stop: StopReason) -> Result<Step, LoopError> {
        self.shared.set_state(State::Finishing);
        let emitted = self.shared.emit(Event::TurnDone { turn, stop }).await;
        self.shared.set_state(State::Idle);
        emitted.map(|()| Step::Done)
    }
}

/// The assistant message for a tool round: its text, then each call, a
/// signed call preceded by its empty signed `Thinking` block (the order
/// `dispatch::run_batch` emits them in).
fn assistant_message(streamed: &stream::Streamed) -> Message {
    let mut content = Vec::new();
    if !streamed.text.is_empty() {
        content.push(Content::Text {
            text: streamed.text.clone(),
        });
    }
    for (id, name, input) in &streamed.calls {
        if let Some(signature) = streamed.signatures.get(id) {
            content.push(Content::Thinking {
                text: String::new(),
                signature: Some(signature.clone()),
            });
        }
        content.push(Content::ToolUse {
            id: *id,
            name: name.clone(),
            input: input.clone(),
        });
    }
    Message {
        role: Role::Assistant,
        content,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use llm_wire::{ProviderError, ProviderEvent};
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::testkit::{
        Gate, Script, Scripted, drain, harness, run, shape, spawn_turn, tool_results, until,
    };
    use crate::types::{DecidedBy, Why};

    const PREFIX: [&str; 3] = ["turn_started", "item_started(user_message)", "item_done"];

    /// One provider call that answers `text` and nothing else, as cox's
    /// golden snapshots show it (their `context_breakdown` stays in cox).
    const REPLY: [&str; 4] = [
        "item_started(assistant_message)",
        "text_delta",
        "usage",
        "item_done",
    ];

    fn expect(parts: &[&[&str]]) -> Vec<String> {
        parts.concat().into_iter().map(String::from).collect()
    }

    const ONE_TOOL: &str = r#"
[[turn]]
text = "echoing"
tool_calls = [{ name = "echo", input = { text = "hi" } }]

[[turn]]
text = "done"
"#;

    const WRITE: &str = r#"
[[turn]]
text = "writing"
tool_calls = [{ name = "touch", input = { path = "a" } }]

[[turn]]
text = "done"
"#;

    fn approval_call(events: &[Event]) -> CallId {
        events
            .iter()
            .find_map(|e| match e {
                Event::ApprovalRequired { call, .. } => Some(call.id),
                _ => None,
            })
            .expect("an approval prompt")
    }

    /// Runs `toml` until its first `ApprovalRequired`, answers it with
    /// `decision`, then drains the turn.
    async fn answered(toml: &str, gate: Gate, decision: Decision) -> (Vec<Event>, usize) {
        let mut h = harness(Scripted::from_toml(toml), gate, Config::default());
        let running = spawn_turn(&h.agent, "write");
        let mut events = until(&mut h.rx, |e| matches!(e, Event::ApprovalRequired { .. })).await;
        assert_eq!(h.agent.state(), State::AwaitingApproval);
        let call_id = approval_call(&events);
        h.agent
            .submit(Submission::Approve { call_id, decision })
            .await
            .expect("approve");
        running.await.expect("join").expect("turn");
        events.extend(drain(&mut h.rx).await);
        let touched = h.kit.started().iter().filter(|n| *n == "touch").count();
        (events, touched)
    }

    #[tokio::test]
    async fn text_only_turn_streams_one_reply_and_ends() {
        let (events, h) = run(
            "[[turn]]\ntext = \"hello\"\n",
            Gate::default(),
            Config::default(),
        )
        .await;
        assert_eq!(
            shape(&events),
            expect(&[&PREFIX, &REPLY, &["turn_done(end_turn)"]])
        );
        let history = h.agent.history().await;
        assert_eq!(history.len(), 2);
        assert_eq!(
            history[1].content,
            vec![Content::Text {
                text: "hello".into()
            }]
        );
        assert_eq!(h.agent.state(), State::Idle);
    }

    #[tokio::test]
    async fn one_tool_turn_matches_cox_event_order() {
        let (events, h) = run(ONE_TOOL, Gate::default(), Config::default()).await;
        let tools = ["tool_call_requested", "tool_call_done"];
        assert_eq!(
            shape(&events),
            expect(&[&PREFIX, &REPLY, &tools, &REPLY, &["turn_done(end_turn)"]])
        );
        assert_eq!(tool_results(&events), [(true, "hi".to_string())]);
        assert_eq!(h.provider.calls(), 2);
        // user, assistant tool_use, user tool_result, assistant text
        let roles: Vec<Role> = h.agent.history().await.iter().map(|m| m.role).collect();
        assert_eq!(
            roles,
            [Role::User, Role::Assistant, Role::User, Role::Assistant]
        );
        let second = &h.provider.requests.lock().expect("requests")[1];
        assert!(matches!(
            second.messages[2].content[..],
            [Content::ToolResult {
                is_error: false,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn parallel_results_return_in_request_order_in_one_message() {
        let toml = r#"
[[turn]]
tool_calls = [
  { name = "echo", input = { text = "a" } },
  { name = "echo", input = { text = "b" } },
  { name = "echo", input = { text = "c" } },
]

[[turn]]
text = "all three"
"#;
        let (events, h) = run(toml, Gate::default(), Config::default()).await;
        let done: Vec<_> = tool_results(&events).into_iter().map(|r| r.1).collect();
        assert_eq!(done, ["a", "b", "c"]);
        let history = h.agent.history().await;
        let with_results: Vec<_> = history
            .iter()
            .filter(|m| {
                m.content
                    .iter()
                    .any(|c| matches!(c, Content::ToolResult { .. }))
            })
            .collect();
        assert_eq!(with_results.len(), 1);
        assert_eq!(with_results[0].content.len(), 3);
    }

    #[tokio::test]
    async fn parallel_calls_overlap_up_to_the_cap() {
        let toml = "[[turn]]\ntool_calls = [\n".to_string()
            + &"  { name = \"wait\", input = {} },\n".repeat(5)
            + "]\n\n[[turn]]\ntext = \"ok\"\n";
        let config = Config {
            parallel_tools: 2,
            ..Config::default()
        };
        let (events, h) = run(&toml, Gate::default(), config).await;
        assert_eq!(tool_results(&events).len(), 5);
        assert_eq!(h.kit.peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn exclusive_calls_run_alone_before_the_parallel_ones() {
        let toml = r#"
[[turn]]
tool_calls = [
  { name = "wait", input = {} },
  { name = "touch", input = { path = "a" } },
  { name = "wait", input = {} },
]

[[turn]]
text = "ok"
"#;
        let gate = Gate {
            grants: std::sync::Mutex::new(vec![("touch".into(), "a".into())]),
            ..Gate::default()
        };
        let (events, h) = run(toml, gate, Config::default()).await;
        assert_eq!(h.kit.started(), ["touch", "wait", "wait"]);
        let done: Vec<_> = tool_results(&events).into_iter().map(|r| r.1).collect();
        assert_eq!(done, ["waited", "touched", "waited"]);
    }

    #[tokio::test]
    async fn max_steps_ends_the_turn_after_the_last_allowed_call() {
        let toml = "[[turn]]\ntool_calls = [{ name = \"echo\", input = { text = \"1\" } }]\n\n[[turn]]\ntext = \"should not be reached\"\n";
        let config = Config {
            max_steps: 1,
            ..Config::default()
        };
        let (events, h) = run(toml, Gate::default(), config).await;
        let reply = ["item_started(assistant_message)", "usage", "item_done"];
        let tools = ["tool_call_requested", "tool_call_done"];
        assert_eq!(
            shape(&events),
            expect(&[&PREFIX, &reply, &tools, &["turn_done(max_turns)"]])
        );
        assert_eq!(h.provider.calls(), 1);
    }

    #[tokio::test]
    async fn provider_error_ends_the_turn_with_an_error_event() {
        let toml = "[[turn]]\ntext = \"partial\"\nerror = \"boom\"\n";
        let (events, h) = run(toml, Gate::default(), Config::default()).await;
        let streamed = ["item_started(assistant_message)", "text_delta", "error"];
        assert_eq!(
            shape(&events),
            expect(&[&PREFIX, &streamed, &["turn_done(error)"]])
        );
        // A failed reply never joins history.
        assert_eq!(h.agent.history().await.len(), 1);
    }

    #[tokio::test]
    async fn retryable_provider_error_recovers_once() {
        let too_long = || ProviderError::ContextTooLong { max: 1, got: 2 };
        let provider =
            Scripted::from_scripts(vec![Script::Fail(too_long()), Script::Fail(too_long())]);
        let mut h = harness(provider, Gate::default(), Config::default());
        let running = spawn_turn(&h.agent, "go");
        let events = drain(&mut h.rx).await;
        running.await.expect("join").expect("turn");
        assert_eq!(h.context.recovered.load(Ordering::SeqCst), 1);
        assert_eq!(h.provider.calls(), 2);
        let first = ["item_started(assistant_message)", "item_done"];
        let second = ["item_started(assistant_message)", "error"];
        assert_eq!(
            shape(&events),
            expect(&[&PREFIX, &first, &second, &["turn_done(error)"]])
        );
    }

    #[tokio::test]
    async fn unknown_tool_fails_without_running() {
        let toml =
            "[[turn]]\ntool_calls = [{ name = \"nope\", input = {} }]\n\n[[turn]]\ntext = \"ok\"\n";
        let (events, h) = run(toml, Gate::default(), Config::default()).await;
        assert_eq!(
            tool_results(&events),
            [(false, "unknown tool nope".to_string())]
        );
        assert!(h.kit.started().is_empty());
    }

    #[tokio::test]
    async fn tool_output_streams_before_its_result() {
        let toml = "[[turn]]\ntool_calls = [{ name = \"chatty\", input = {} }]\n\n[[turn]]\ntext = \"ok\"\n";
        let (events, _) = run(toml, Gate::default(), Config::default()).await;
        let tail: Vec<_> = shape(&events)
            .into_iter()
            .skip_while(|t| t != "tool_call_requested")
            .take(4)
            .collect();
        assert_eq!(
            tail,
            [
                "tool_call_requested",
                "tool_call_output",
                "tool_call_output",
                "tool_call_done"
            ]
        );
    }

    #[tokio::test]
    async fn untrusted_tool_output_is_capped_for_the_model() {
        let toml =
            "[[turn]]\ntool_calls = [{ name = \"big\", input = {} }]\n\n[[turn]]\ntext = \"ok\"\n";
        let config = Config {
            tool_output_cap: 1024,
            ..Config::default()
        };
        let (events, h) = run(toml, Gate::default(), config).await;
        let (ok, visible) = &tool_results(&events)[0];
        assert!(ok);
        assert!(visible.starts_with(&"x".repeat(1024)));
        assert!(visible.ends_with("[output cut: 1024 of 102400 bytes shown]"));
        let sent = &h.provider.requests.lock().expect("requests")[1];
        assert!(matches!(
            &sent.messages[2].content[..],
            [Content::ToolResult { content, .. }] if content == visible
        ));
    }

    #[tokio::test]
    async fn approval_allow_runs_the_call() {
        let (events, touched) = answered(WRITE, Gate::default(), Decision::Allow).await;
        let tools = [
            "tool_call_requested",
            "approval_required",
            "approval_decided",
            "tool_call_done",
        ];
        assert_eq!(
            shape(&events),
            expect(&[&PREFIX, &REPLY, &tools, &REPLY, &["turn_done(end_turn)"]])
        );
        assert_eq!(tool_results(&events), [(true, "touched".to_string())]);
        assert_eq!(touched, 1);
        assert!(events.iter().any(|e| matches!(
            e,
            Event::ApprovalRequired {
                why: Why::Risk {
                    risk: llm_wire::Risk::Write
                },
                ..
            }
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            Event::ApprovalDecided {
                decision: Decision::Allow,
                by: DecidedBy::User,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn approval_deny_never_runs_the_call() {
        let deny = Decision::Deny {
            reason: "no".into(),
        };
        let (events, touched) = answered(WRITE, Gate::default(), deny).await;
        assert_eq!(touched, 0);
        assert_eq!(
            tool_results(&events),
            [(false, "permission denied: no".to_string())]
        );
        assert!(matches!(
            events.last(),
            Some(Event::TurnDone {
                stop: StopReason::EndTurn,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn a_rule_deny_is_decided_without_asking() {
        let gate = Gate {
            deny: vec!["touch".into()],
            ..Gate::default()
        };
        let (events, h) = run(WRITE, gate, Config::default()).await;
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::ApprovalRequired { .. }))
        );
        assert!(events.iter().any(|e| matches!(
            e,
            Event::ApprovalDecided {
                by: DecidedBy::Rule,
                ..
            }
        )));
        assert_eq!(
            tool_results(&events),
            [(false, "permission denied: a rule refuses it".to_string())]
        );
        assert!(h.kit.started().is_empty());
    }

    #[tokio::test]
    async fn prepare_can_refuse_before_the_rules_run() {
        let toml = "[[turn]]\ntool_calls = [{ name = \"echo\", input = { text = \"blocked\" } }]\n\n[[turn]]\ntext = \"ok\"\n";
        let (events, h) = run(toml, Gate::default(), Config::default()).await;
        assert_eq!(
            tool_results(&events),
            [(false, "blocked by hook: no".to_string())]
        );
        assert!(h.kit.started().is_empty());
    }

    #[tokio::test]
    async fn allow_for_session_covers_the_next_call() {
        let toml = r#"
[[turn]]
tool_calls = [
  { name = "touch", input = { path = "a" } },
  { name = "touch", input = { path = "a" } },
]

[[turn]]
text = "done"
"#;
        let (events, touched) = answered(toml, Gate::default(), Decision::AllowForSession).await;
        let prompts = events
            .iter()
            .filter(|e| matches!(e, Event::ApprovalRequired { .. }))
            .count();
        assert_eq!(prompts, 1);
        assert_eq!(touched, 2);
    }

    #[tokio::test]
    async fn edited_input_goes_back_through_the_rules() {
        let gate = Gate {
            ask: vec!["hi".into()],
            ..Gate::default()
        };
        let edit = Decision::Edit {
            input: serde_json::json!({"text": "bye"}),
        };
        let (events, _) = answered(ONE_TOOL, gate, edit).await;
        // `echo(bye)` matches no ask rule and is read-only, so the edited
        // call runs without a second prompt.
        let prompts = events
            .iter()
            .filter(|e| matches!(e, Event::ApprovalRequired { .. }))
            .count();
        assert_eq!(prompts, 1);
        assert_eq!(tool_results(&events), [(true, "bye".to_string())]);
    }

    #[tokio::test]
    async fn approval_timeout_denies_by_policy() {
        let config = Config {
            approval_timeout: Some(Duration::from_millis(20)),
            ..Config::default()
        };
        let (events, h) = run(WRITE, Gate::default(), config).await;
        assert!(events.iter().any(|e| matches!(
            e,
            Event::ApprovalDecided {
                by: DecidedBy::Policy,
                ..
            }
        )));
        assert_eq!(
            tool_results(&events),
            [(false, "permission denied: approval timed out".to_string())]
        );
        assert!(h.kit.started().is_empty());
    }

    #[tokio::test]
    async fn approve_with_nothing_pending_is_a_notice() {
        let mut h = harness(Scripted::default(), Gate::default(), Config::default());
        let call_id = CallId::new();
        h.agent
            .submit(Submission::Approve {
                call_id,
                decision: Decision::Allow,
            })
            .await
            .expect("submit");
        assert_eq!(
            h.rx.recv().await,
            Some(Event::Notice {
                level: Level::Warn,
                text: format!("no approval pending for call {call_id}"),
            })
        );
    }

    #[tokio::test]
    async fn interrupt_mid_tool_ends_the_turn_interrupted() {
        let toml = "[[turn]]\ntool_calls = [{ name = \"slow\", input = {} }]\n";
        let mut h = harness(
            Scripted::from_toml(toml),
            Gate::default(),
            Config::default(),
        );
        let running = spawn_turn(&h.agent, "interrupt");
        let mut events = until(&mut h.rx, |e| matches!(e, Event::ToolCallRequested { .. })).await;
        h.agent
            .submit(Submission::Interrupt)
            .await
            .expect("interrupt");
        running.await.expect("join").expect("turn");
        events.extend(drain(&mut h.rx).await);
        let reply = ["item_started(assistant_message)", "usage", "item_done"];
        let tools = ["tool_call_requested", "tool_call_done"];
        assert_eq!(
            shape(&events),
            expect(&[&PREFIX, &reply, &tools, &["turn_done(interrupted)"]])
        );
        assert_eq!(tool_results(&events), [(false, "cancelled".to_string())]);
        // The interrupted call still has its result in history.
        let history = h.agent.history().await;
        assert!(matches!(
            history.last().map(|m| &m.content[..]),
            Some([Content::ToolResult { is_error: true, .. }])
        ));
        assert!(h.rx.try_recv().is_err(), "nothing after TurnDone");
    }

    /// A voice barge-in: the reply is cut mid-stream, even by a provider
    /// that never honours its cancel token.
    #[tokio::test]
    async fn interrupt_mid_stream_ends_the_turn_at_once() {
        let provider =
            Scripted::from_scripts(vec![Script::Stall(vec![ProviderEvent::TextDelta {
                text: "Sure, the weather in".into(),
            }])]);
        let mut h = harness(provider, Gate::default(), Config::default());
        let running = spawn_turn(&h.agent, "weather?");
        let mut events = until(&mut h.rx, |e| matches!(e, Event::TextDelta { .. })).await;
        assert_eq!(h.agent.state(), State::Streaming);
        let asked = Instant::now();
        h.agent.interrupt();
        tokio::time::timeout(Duration::from_secs(1), running)
            .await
            .expect("the turn ends without waiting on the provider")
            .expect("join")
            .expect("turn");
        assert!(asked.elapsed() < Duration::from_millis(200));
        events.extend(drain(&mut h.rx).await);
        let streamed = ["item_started(assistant_message)", "text_delta"];
        assert_eq!(
            shape(&events),
            expect(&[&PREFIX, &streamed, &["turn_done(interrupted)"]])
        );
        // The cut reply never joins history; the next turn starts clean.
        assert_eq!(h.agent.history().await.len(), 1);
    }

    #[tokio::test]
    async fn interrupt_while_awaiting_approval_denies_and_runs_nothing() {
        let mut h = harness(
            Scripted::from_toml(WRITE),
            Gate::default(),
            Config::default(),
        );
        let running = spawn_turn(&h.agent, "write");
        let mut events = until(&mut h.rx, |e| matches!(e, Event::ApprovalRequired { .. })).await;
        let call_id = approval_call(&events);
        h.agent.interrupt();
        running.await.expect("join").expect("turn");
        events.extend(drain(&mut h.rx).await);
        assert!(events.iter().any(|e| matches!(
            e,
            Event::ApprovalDecided { decision: Decision::Deny { reason }, .. } if reason == "interrupted"
        )));
        assert!(h.kit.started().is_empty());
        assert!(matches!(
            events.last(),
            Some(Event::TurnDone {
                stop: StopReason::Interrupted,
                ..
            })
        ));
        // The prompt ended with the turn; a late answer finds nothing.
        h.agent
            .submit(Submission::Approve {
                call_id,
                decision: Decision::Allow,
            })
            .await
            .expect("late approve");
        assert!(matches!(h.rx.recv().await, Some(Event::Notice { .. })));
    }

    #[tokio::test]
    async fn no_call_starts_after_an_interrupt() {
        let toml = r#"
[[turn]]
tool_calls = [
  { name = "touch", input = { path = "a" } },
  { name = "touch", input = { path = "b" } },
]
"#;
        let gate = Gate {
            grants: std::sync::Mutex::new(vec![
                ("touch".into(), "a".into()),
                ("touch".into(), "b".into()),
            ]),
            ..Gate::default()
        };
        let mut h = harness(Scripted::from_toml(toml), gate, Config::default());
        let running = spawn_turn(&h.agent, "two writes");
        let _ = until(&mut h.rx, |e| matches!(e, Event::ToolCallRequested { .. })).await;
        h.agent.interrupt();
        running.await.expect("join").expect("turn");
        let events = drain(&mut h.rx).await;
        assert!(h.kit.started().len() <= 1, "{:?}", h.kit.started());
        assert!(
            tool_results(&events)
                .iter()
                .any(|(ok, text)| !ok && text == "interrupted before it ran")
        );
    }

    #[tokio::test]
    async fn the_next_turn_runs_after_an_interrupted_one() {
        let toml = "[[turn]]\ntool_calls = [{ name = \"slow\", input = {} }]\n\n[[turn]]\ntext = \"back\"\n";
        let mut h = harness(
            Scripted::from_toml(toml),
            Gate::default(),
            Config::default(),
        );
        let running = spawn_turn(&h.agent, "first");
        let _ = until(&mut h.rx, |e| matches!(e, Event::ToolCallRequested { .. })).await;
        h.agent.interrupt();
        running.await.expect("join").expect("turn");
        let _ = drain(&mut h.rx).await;
        let running = spawn_turn(&h.agent, "second");
        let events = drain(&mut h.rx).await;
        running.await.expect("join").expect("turn");
        assert!(matches!(
            events.first(),
            Some(Event::TurnStarted { seq: 2, .. })
        ));
        assert!(matches!(
            events.last(),
            Some(Event::TurnDone {
                stop: StopReason::EndTurn,
                ..
            })
        ));
    }
}
