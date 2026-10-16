//! The seams the loop drives instead of doing the work itself: a tool
//! executor (`Tools`), an approval gate (`Approvals`), the session-specific
//! parts of a turn (`Context`) and where events go (`EventSink`). The model
//! is `llm_wire::Provider`. The loop owns ordering, concurrency, approvals
//! and cancellation; everything domain-specific lives behind these traits.

use async_trait::async_trait;
use llm_wire::{Concurrency, Content, Message, ProviderError, Request, Risk, StopReason, Usage};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::ids::TurnId;
use crate::types::{Attachment, DecidedBy, Event, LoopError, Segments, ToolCall, ToolResult, Why};

/// How the executor rates a call's input: what the approval gate judges.
#[derive(Debug, Clone, PartialEq)]
pub struct Rating {
    /// The call's risk for this input.
    pub risk: Risk,
    /// What approval rules match on.
    pub subject: String,
    /// The simple commands a shell subject splits into, if any.
    pub segments: Option<Segments>,
}

/// What a running tool gets from the loop.
#[derive(Debug, Clone)]
pub struct ToolCx {
    /// The turn the call belongs to.
    pub turn: TurnId,
    /// Fires on interrupt; a tool must return promptly once it does.
    pub cancel: CancellationToken,
    /// Streamed output for display (`ToolCallOutput`). The loop forwards
    /// it until `Tools::run` returns; later sends are dropped.
    pub output: mpsc::Sender<String>,
}

/// The tool executor: rates calls and runs them.
#[async_trait]
pub trait Tools: Send + Sync {
    /// Rates `input` for the tool `name`; `None` for a tool it does not
    /// know, which the loop answers with a failed result and never runs.
    fn rate(&self, name: &str, input: &Value) -> Option<Rating>;
    /// Whether an approved call may run alongside others in its batch.
    /// Asked after approval, so it sees the final input and risk.
    fn concurrency(&self, call: &ToolCall) -> Concurrency;
    /// Runs an approved call. Its output is untrusted; the loop caps
    /// `visible` and measures `duration_ms`.
    async fn run(&self, call: &ToolCall, cx: ToolCx) -> ToolResult;
}

/// The engine's verdict on a call before anyone is asked.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Run it.
    Allow,
    /// Refuse it; the loop emits `ApprovalDecided` with `by`.
    Deny {
        /// Shown to the model in the tool result.
        reason: String,
        /// Who decided.
        by: DecidedBy,
    },
    /// Ask the user (`ApprovalRequired`).
    Ask(Why),
}

/// The approval gate. Policy lives here, outside the model; the loop runs
/// no tool before the gate (or the user) allows it.
#[async_trait]
pub trait Approvals: Send + Sync {
    /// Runs once per call before `decide` (cox: the `PreToolUse` hook and
    /// risk advice). Returns the call to judge, possibly with new input,
    /// or `Err` with the text the model sees instead of a result.
    async fn prepare(&self, call: ToolCall) -> Result<ToolCall, String> {
        Ok(call)
    }
    /// The verdict for `call` under the current rules, mode and grants.
    async fn decide(&self, call: &ToolCall) -> Verdict;
    /// The user answered `AllowForSession` for `call`.
    async fn grant(&self, call: &ToolCall) {
        let _ = call;
    }
    /// Called just before `ApprovalRequired` is emitted for `call`.
    async fn asking(&self, call: &ToolCall, why: &Why) {
        let _ = (call, why);
    }
}

/// What to do with the next provider call.
#[derive(Debug, Clone, PartialEq)]
pub enum Next {
    /// Send this request.
    Send(Request),
    /// End the turn with this reason instead (a budget cap, a request too
    /// big to send, a config error).
    Stop(StopReason),
}

/// The session-specific parts of a turn: request assembly and the hooks
/// around a provider call (cox: prompts, routing, compaction, ledger).
#[async_trait]
pub trait Context: Send + Sync {
    /// The user message's content for a new turn. By default the text,
    /// then each attachment as an image block.
    fn user_content(&self, text: &str, attachments: &[Attachment]) -> Vec<Content> {
        let mut content = vec![Content::Text { text: text.into() }];
        content.extend(attachments.iter().map(|a| Content::Image {
            media_type: a.media_type.clone(),
            data_b64: a.data_b64.clone(),
        }));
        content
    }
    /// Builds the request for provider call number `step` (1-based) of
    /// `turn`. May rewrite `history` first (compaction).
    async fn request(&self, turn: TurnId, step: u32, history: &mut Vec<Message>) -> Next;
    /// Whether `error` is worth one `recover` per turn (cox: a context
    /// that is too long).
    fn retryable(&self, error: &ProviderError) -> bool {
        let _ = error;
        false
    }
    /// Tries to make the next request succeed after a `retryable` error;
    /// `true` retries the call.
    async fn recover(&self, error: &ProviderError, history: &mut Vec<Message>) -> bool {
        let _ = (error, history);
        false
    }
    /// A provider call's usage, before its `Usage` event (cox: the ledger).
    async fn on_usage(&self, turn: TurnId, usage: &Usage) -> Result<(), LoopError> {
        let _ = (turn, usage);
        Ok(())
    }
    /// The tool results message, before it joins history.
    async fn on_results(&self, results: &mut Message) {
        let _ = results;
    }
}

/// Where the loop's events go.
#[async_trait]
pub trait EventSink: Send + Sync {
    /// Takes one event. An error ends the turn with `LoopError::Sink`.
    async fn emit(&self, event: Event) -> Result<(), LoopError>;
}

#[async_trait]
impl EventSink for mpsc::Sender<Event> {
    /// A receiver that went away is not an error: a surface that left must
    /// not fail the turn.
    async fn emit(&self, event: Event) -> Result<(), LoopError> {
        let _ = self.send(event).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The loop holds every seam as `Arc<dyn _>`.
    #[test]
    fn seams_are_object_safe() {
        fn assert_object_safe<T: ?Sized>() {}
        assert_object_safe::<dyn Tools>();
        assert_object_safe::<dyn Approvals>();
        assert_object_safe::<dyn Context>();
        assert_object_safe::<dyn EventSink>();
    }
}
