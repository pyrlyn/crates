//! One provider stream, turned into events: forwards `ProviderEvent`s as
//! `Event`s and collects the tool-use blocks for the batch that follows.
//! Separate from the turn loop so a step stays a state transition.
//!
//! Reasoning streams into its own `Thinking` item, and the reply's
//! `AssistantMessage` item starts only once the thought is over (before the
//! first other provider event, or at the end), so every surface lists the
//! thought ahead of the reply it led to.

use std::collections::HashMap;
use std::time::Instant;

use llm_wire::{CallId, ProviderError, ProviderEvent, Usage};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::ids::ItemId;
use crate::traits::EventSink;
use crate::types::{Event, ItemKind, LoopError};

/// What one stream produced.
#[derive(Debug, Default)]
pub(crate) struct Streamed {
    pub text: String,
    pub calls: Vec<(CallId, String, Value)>,
    /// Thought signatures by call id; opaque, for replay only.
    pub signatures: HashMap<CallId, String>,
    pub usage: Option<Usage>,
}

/// How a stream ended.
#[derive(Debug)]
pub(crate) enum Consumed {
    /// The provider closed the channel.
    Done(Streamed),
    /// The provider streamed an `Error`.
    Failed(ProviderError),
    /// `cancel` fired; whatever was still buffered is dropped.
    Cancelled,
}

struct Acc {
    id: CallId,
    name: String,
    input: String,
}

/// The `Thinking` item still growing, with when its first and latest
/// deltas arrived: `ThinkingDone` reports the gap.
struct Thought {
    item: ItemId,
    first: Instant,
    last: Instant,
}

async fn end_thought(sink: &dyn EventSink, thought: Thought) -> Result<(), LoopError> {
    let duration_ms = thought.last.duration_since(thought.first).as_millis() as u64;
    sink.emit(Event::ThinkingDone {
        item: thought.item,
        duration_ms,
    })
    .await?;
    sink.emit(Event::ItemDone { item: thought.item }).await
}

async fn start_reply(sink: &dyn EventSink, item: ItemId) -> Result<(), LoopError> {
    let kind = ItemKind::AssistantMessage {
        text: String::new(),
    };
    sink.emit(Event::ItemStarted { item, kind }).await
}

/// Reads `rx` until the provider closes it, streams an error, or `cancel`
/// fires. Cancel wins over buffered events, so an interrupt (a voice
/// barge-in) stops the reply at once, whatever the provider does next.
pub(crate) async fn consume(
    sink: &dyn EventSink,
    rx: &mut mpsc::Receiver<ProviderEvent>,
    assistant_item: ItemId,
    cancel: &CancellationToken,
) -> Result<Consumed, LoopError> {
    use ProviderEvent as P;
    let mut out = Streamed::default();
    let mut current: Option<Acc> = None;
    let mut thought: Option<Thought> = None;
    let mut replying = false;
    loop {
        let ev = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(Consumed::Cancelled),
            ev = rx.recv() => ev,
        };
        let Some(ev) = ev else { break };
        if !matches!(ev, P::ThinkingDelta { .. }) {
            if let Some(done) = thought.take() {
                end_thought(sink, done).await?;
            }
            if !replying {
                replying = true;
                start_reply(sink, assistant_item).await?;
            }
        }
        match ev {
            P::TextDelta { text } => {
                out.text.push_str(&text);
                sink.emit(Event::TextDelta {
                    item: assistant_item,
                    text,
                })
                .await?;
            }
            P::ThinkingDelta { text } => {
                let now = Instant::now();
                let item = match thought.as_mut() {
                    Some(open) => {
                        open.last = now;
                        open.item
                    }
                    None => {
                        let item = ItemId::new();
                        let kind = ItemKind::Thinking {
                            text: String::new(),
                            signature: None,
                        };
                        sink.emit(Event::ItemStarted { item, kind }).await?;
                        thought = Some(Thought {
                            item,
                            first: now,
                            last: now,
                        });
                        item
                    }
                };
                sink.emit(Event::ThinkingDelta { item, text }).await?;
            }
            P::ToolUseStart { id, name } => {
                current = Some(Acc {
                    id,
                    name,
                    input: String::new(),
                });
            }
            P::ToolUseSignature { signature } => {
                if let Some(acc) = current.as_ref() {
                    out.signatures.insert(acc.id, signature);
                }
            }
            P::ToolUseInputDelta { text } => {
                if let Some(acc) = current.as_mut() {
                    acc.input.push_str(&text);
                }
            }
            P::ToolUseEnd => {
                if let Some(acc) = current.take() {
                    // Malformed JSON from the model is still a call: the
                    // tool sees `null` and fails it, the turn goes on.
                    let input = serde_json::from_str(&acc.input).unwrap_or(Value::Null);
                    out.calls.push((acc.id, acc.name, input));
                }
            }
            P::Usage { usage } => out.usage = Some(usage),
            P::Error { error } => return Ok(Consumed::Failed(error)),
            P::MessageStart { .. } | P::Stop { .. } | P::Retrying { .. } => {}
        }
    }
    if let Some(done) = thought {
        end_thought(sink, done).await?;
    }
    if !replying {
        start_reply(sink, assistant_item).await?;
    }
    Ok(Consumed::Done(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    async fn run(events: Vec<ProviderEvent>, item: ItemId) -> (Consumed, Vec<Event>) {
        let (tx, mut rx) = mpsc::channel(16);
        for ev in events {
            tx.send(ev).await.expect("send");
        }
        drop(tx);
        let (etx, mut erx) = mpsc::channel(64);
        let consumed = consume(&etx, &mut rx, item, &CancellationToken::new())
            .await
            .expect("stream");
        drop(etx);
        let mut seen = Vec::new();
        while let Some(ev) = erx.recv().await {
            seen.push(ev);
        }
        (consumed, seen)
    }

    /// A signature streamed between a call's start and end is kept under
    /// that call's id, and the call itself still commits.
    #[tokio::test]
    async fn consume_keeps_signature_by_call_id() {
        let signed = CallId::new();
        let unsigned = CallId::new();
        let events = vec![
            ProviderEvent::ToolUseStart {
                id: signed,
                name: "read".into(),
            },
            ProviderEvent::ToolUseSignature {
                signature: "sig-1".into(),
            },
            ProviderEvent::ToolUseInputDelta {
                text: r#"{"path":"a.rs"}"#.into(),
            },
            ProviderEvent::ToolUseEnd,
            ProviderEvent::ToolUseStart {
                id: unsigned,
                name: "read".into(),
            },
            ProviderEvent::ToolUseEnd,
        ];
        let (Consumed::Done(streamed), _) = run(events, ItemId::new()).await else {
            panic!("stream ends normally");
        };
        assert_eq!(streamed.calls.len(), 2);
        assert_eq!(streamed.calls[0].2, serde_json::json!({"path": "a.rs"}));
        assert_eq!(streamed.calls[1].2, Value::Null);
        assert_eq!(
            streamed.signatures,
            HashMap::from([(signed, "sig-1".to_string())])
        );
    }

    /// Reasoning streams into its own `Thinking` item, which closes with
    /// its duration before the reply's item starts.
    #[tokio::test]
    async fn streamed_thought_is_its_own_item_closed_before_the_reply() {
        let assistant = ItemId::new();
        let events = vec![
            ProviderEvent::ThinkingDelta {
                text: "weigh ".into(),
            },
            ProviderEvent::ThinkingDelta {
                text: "options".into(),
            },
            ProviderEvent::TextDelta { text: "ok".into() },
        ];
        let (_, seen) = run(events, assistant).await;
        let Some(Event::ItemStarted {
            item: thought,
            kind: ItemKind::Thinking { .. },
        }) = seen.first().cloned()
        else {
            panic!("thought item first: {seen:?}");
        };
        assert!(matches!(
            &seen[1..],
            [
                Event::ThinkingDelta { item: a, .. },
                Event::ThinkingDelta { item: b, .. },
                Event::ThinkingDone { item: c, .. },
                Event::ItemDone { item: d },
                Event::ItemStarted { item: e, kind: ItemKind::AssistantMessage { .. } },
                Event::TextDelta { item: f, .. },
            ] if [a, b, c, d] == [&thought; 4] && [e, f] == [&assistant; 2]
        ));
    }

    /// An empty stream still opens the reply item, so `ItemDone` always
    /// has a start to close.
    #[tokio::test]
    async fn empty_stream_still_opens_the_reply() {
        let assistant = ItemId::new();
        let (_, seen) = run(vec![], assistant).await;
        assert_eq!(
            seen,
            vec![Event::ItemStarted {
                item: assistant,
                kind: ItemKind::AssistantMessage {
                    text: String::new()
                },
            }]
        );
    }

    /// Cancel wins over events still waiting in the channel.
    #[tokio::test]
    async fn cancel_drops_buffered_events() {
        let (tx, mut rx) = mpsc::channel(4);
        tx.send(ProviderEvent::TextDelta {
            text: "late".into(),
        })
        .await
        .expect("send");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let (etx, mut erx) = mpsc::channel(4);
        let consumed = consume(&etx, &mut rx, ItemId::new(), &cancel)
            .await
            .expect("stream");
        assert!(matches!(consumed, Consumed::Cancelled));
        drop(etx);
        assert_eq!(erx.recv().await, None);
    }
}
