//! Test support for the loop's own tests: a scripted provider over
//! `llm_wire::test_util` scenarios, stub tools, a risk-based gate, a plain
//! request builder and event helpers. Mirrors cox-core's loop-test harness
//! (`tests/common`) so the ported tests read the same.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use llm_wire::test_util::scripted::{TurnSpec, events_for, parse_scenario};
use llm_wire::{
    Caps, Concurrency, Effort, Job, Message, ModelId, Provider, ProviderError, ProviderEvent,
    ProviderId, Request, Risk, Thinking, Tier, Usage,
};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    Agent, Approvals, Config, Context, Event, Next, Parts, Rating, Submission, ToolCall, ToolCx,
    ToolResult, Tools, TurnId, Verdict, Why,
};

/// One provider call's script.
pub enum Script {
    /// A scenario turn, streamed like cox's `Scripted` does.
    Turn(TurnSpec),
    /// Sends these events, then hangs ignoring its cancel token: the worst
    /// provider an interrupt has to beat.
    Stall(Vec<ProviderEvent>),
    /// Fails before streaming anything.
    Fail(ProviderError),
}

/// A provider that answers each call with the next script.
#[derive(Default)]
pub struct Scripted {
    scripts: Mutex<VecDeque<Script>>,
    pub requests: Mutex<Vec<Request>>,
}

pub fn usage() -> Usage {
    Usage {
        input_tokens: 10,
        output_tokens: 5,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        estimated: true,
        cost_usd: 0.0,
        latency_ms: 0,
    }
}

impl Scripted {
    pub fn from_toml(toml: &str) -> Self {
        let turns = parse_scenario(toml).expect("scenario");
        Self::from_scripts(turns.into_iter().map(Script::Turn).collect())
    }

    pub fn from_scripts(scripts: Vec<Script>) -> Self {
        Self {
            scripts: Mutex::new(scripts.into()),
            requests: Mutex::default(),
        }
    }

    pub fn calls(&self) -> usize {
        self.requests.lock().expect("requests").len()
    }
}

#[async_trait]
impl Provider for Scripted {
    fn id(&self) -> ProviderId {
        ProviderId::Local
    }
    fn capabilities(&self) -> Caps {
        Caps {
            cache: false,
            thinking: false,
            server_tools: false,
            count_tokens: false,
            max_context: 200_000,
        }
    }
    async fn stream(
        &self,
        req: Request,
        sink: mpsc::Sender<ProviderEvent>,
        cancel: CancellationToken,
    ) -> Result<Usage, ProviderError> {
        let model = req.model.clone();
        self.requests.lock().expect("requests").push(req);
        let script = self.scripts.lock().expect("scripts").pop_front();
        let script = script.ok_or_else(|| ProviderError::Unsupported {
            feature: "scripted scenario ran out".into(),
        })?;
        match script {
            Script::Turn(turn) => {
                for event in events_for(&turn, model, usage()) {
                    // The real providers' channel discipline: honour cancel
                    // before each send; bail if the receiver hung up.
                    if cancel.is_cancelled() || sink.send(event).await.is_err() {
                        return Err(ProviderError::Cancelled);
                    }
                }
                match turn.error {
                    Some(message) => Err(ProviderError::BadRequest { message }),
                    None => Ok(usage()),
                }
            }
            Script::Stall(events) => {
                for event in events {
                    let _ = sink.send(event).await;
                }
                std::future::pending().await
            }
            Script::Fail(error) => Err(error),
        }
    }
    async fn count_tokens(&self, _req: &Request) -> Result<u32, ProviderError> {
        Ok(0)
    }
}

fn text(text: &str, ok: bool) -> ToolResult {
    ToolResult {
        ok,
        visible: text.into(),
        archive: None,
        bytes: text.len() as u64,
        duration_ms: 0,
        diff: None,
        structured: None,
    }
}

fn str_field(input: &Value, key: &str) -> String {
    input.get(key).and_then(Value::as_str).unwrap_or("").into()
}

/// The stub tools: `echo` (read-only, parallel, returns `input.text`),
/// `touch` (write, exclusive), `slow` (read-only, runs until cancelled),
/// `chatty` (streams output), `big` (returns 100 KiB), `wait` (read-only,
/// parallel, sleeps so overlap can be measured).
#[derive(Default)]
pub struct Kit {
    /// Tool names in the order their runs started.
    pub started: Mutex<Vec<String>>,
    pub running: AtomicUsize,
    pub peak: AtomicUsize,
}

impl Kit {
    pub fn started(&self) -> Vec<String> {
        self.started.lock().expect("started").clone()
    }
}

#[async_trait]
impl Tools for Kit {
    fn rate(&self, name: &str, input: &Value) -> Option<Rating> {
        let (risk, subject) = match name {
            "echo" => (Risk::ReadOnly, str_field(input, "text")),
            "touch" => (Risk::Write, str_field(input, "path")),
            "slow" | "chatty" | "big" | "wait" => (Risk::ReadOnly, name.into()),
            _ => return None,
        };
        Some(Rating {
            risk,
            subject,
            segments: None,
        })
    }

    fn concurrency(&self, call: &ToolCall) -> Concurrency {
        if call.name == "touch" {
            Concurrency::Exclusive
        } else {
            Concurrency::Parallel
        }
    }

    async fn run(&self, call: &ToolCall, cx: ToolCx) -> ToolResult {
        self.started
            .lock()
            .expect("started")
            .push(call.name.clone());
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        let result = match call.name.as_str() {
            "echo" => text(&call.subject, true),
            "touch" => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                text("touched", true)
            }
            "slow" => loop {
                if cx.cancel.is_cancelled() {
                    break text("cancelled", false);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            },
            "chatty" => {
                for delta in ["one ", "two"] {
                    let _ = cx.output.send(delta.into()).await;
                }
                text("said two things", true)
            }
            "big" => text(&"x".repeat(100 * 1024), true),
            _ => {
                tokio::time::sleep(Duration::from_millis(30)).await;
                text("waited", true)
            }
        };
        self.running.fetch_sub(1, Ordering::SeqCst);
        result
    }
}

/// A gate like cox's default mode: read-only calls run, anything else
/// asks; `deny` names tools a rule refuses, `ask` names subjects an `ask`
/// rule catches, `AllowForSession` grants `(tool, subject)`.
#[derive(Default)]
pub struct Gate {
    pub deny: Vec<String>,
    pub ask: Vec<String>,
    pub grants: Mutex<Vec<(String, String)>>,
}

#[async_trait]
impl Approvals for Gate {
    async fn prepare(&self, call: ToolCall) -> Result<ToolCall, String> {
        if call.subject == "blocked" {
            return Err("blocked by hook: no".into());
        }
        Ok(call)
    }

    async fn decide(&self, call: &ToolCall) -> Verdict {
        let key = (call.name.clone(), call.subject.clone());
        if self.deny.contains(&call.name) {
            return Verdict::Deny {
                reason: "a rule refuses it".into(),
                by: crate::DecidedBy::Rule,
            };
        }
        if self.ask.contains(&call.subject) {
            return Verdict::Ask(Why::RuleAsk {
                rule: format!("{}({})", call.name, call.subject),
            });
        }
        if self.grants.lock().expect("grants").contains(&key) || call.risk == Risk::ReadOnly {
            return Verdict::Allow;
        }
        Verdict::Ask(Why::Risk { risk: call.risk })
    }

    async fn grant(&self, call: &ToolCall) {
        let key = (call.name.clone(), call.subject.clone());
        self.grants.lock().expect("grants").push(key);
    }
}

/// Sends the whole history every step; `ContextTooLong` is retryable and
/// `recover` always succeeds, counting its calls.
#[derive(Default)]
pub struct Plain {
    pub recovered: AtomicUsize,
}

#[async_trait]
impl Context for Plain {
    async fn request(&self, _turn: TurnId, _step: u32, history: &mut Vec<Message>) -> Next {
        Next::Send(Request {
            tier: Tier::Code,
            job: Job::Main,
            model: ModelId("scripted".into()),
            system: vec![],
            tools: vec![],
            messages: history.clone(),
            effort: Effort::High,
            max_tokens: 1024,
            thinking: Thinking::Off,
            cache_breakpoints: vec![],
            stop_sequences: vec![],
        })
    }

    fn retryable(&self, error: &ProviderError) -> bool {
        matches!(error, ProviderError::ContextTooLong { .. })
    }

    async fn recover(&self, _error: &ProviderError, _history: &mut Vec<Message>) -> bool {
        self.recovered.fetch_add(1, Ordering::SeqCst);
        true
    }
}

pub struct Harness {
    pub agent: Agent,
    pub rx: mpsc::Receiver<Event>,
    pub provider: Arc<Scripted>,
    pub kit: Arc<Kit>,
    pub context: Arc<Plain>,
}

pub fn harness(provider: Scripted, gate: Gate, config: Config) -> Harness {
    let (tx, rx) = mpsc::channel(256);
    let provider = Arc::new(provider);
    let kit = Arc::new(Kit::default());
    let context = Arc::new(Plain::default());
    let parts = Parts {
        provider: provider.clone(),
        tools: kit.clone(),
        approvals: Arc::new(gate),
        context: context.clone(),
        sink: Arc::new(tx),
    };
    Harness {
        agent: Agent::new(parts, config),
        rx,
        provider,
        kit,
        context,
    }
}

/// Spawns a user turn so the test can read events (or submit `Interrupt`
/// / `Approve`) while it runs.
pub fn spawn_turn(
    agent: &Agent,
    text: &str,
) -> tokio::task::JoinHandle<Result<(), crate::LoopError>> {
    let agent = agent.clone();
    let text = text.to_owned();
    tokio::spawn(async move {
        agent
            .submit(Submission::UserTurn {
                text,
                attachments: vec![],
            })
            .await
    })
}

/// Collects events until `pred` matches one (inclusive).
pub async fn until(rx: &mut mpsc::Receiver<Event>, pred: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut events = Vec::new();
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("event timeout")
            .expect("event stream closed");
        let hit = pred(&ev);
        events.push(ev);
        if hit {
            return events;
        }
    }
}

/// Collects events up to and including `TurnDone`.
pub async fn drain(rx: &mut mpsc::Receiver<Event>) -> Vec<Event> {
    until(rx, |e| matches!(e, Event::TurnDone { .. })).await
}

/// One user turn over `toml`, drained.
pub async fn run(toml: &str, gate: Gate, config: Config) -> (Vec<Event>, Harness) {
    let mut h = harness(Scripted::from_toml(toml), gate, config);
    let running = spawn_turn(&h.agent, "go");
    let events = drain(&mut h.rx).await;
    running.await.expect("join").expect("turn");
    (events, h)
}

/// `(ok, visible)` of every `ToolCallDone`, in order.
pub fn tool_results(events: &[Event]) -> Vec<(bool, String)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ToolCallDone { result, .. } => Some((result.ok, result.visible.clone())),
            _ => None,
        })
        .collect()
}

/// Each event's serde tag, with an item's kind or a turn's stop reason in
/// parentheses: the shape cox-core's golden snapshots pin.
pub fn shape(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|e| {
            let v = serde_json::to_value(e).expect("event json");
            let tag = v["type"].as_str().unwrap_or("").to_string();
            match (v["kind"]["type"].as_str(), v["stop"]["type"].as_str()) {
                (Some(kind), _) => format!("{tag}({kind})"),
                (_, Some(stop)) => format!("{tag}({stop})"),
                _ => tag,
            }
        })
        .collect()
}
