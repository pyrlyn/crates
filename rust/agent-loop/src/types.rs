//! What crosses the loop's boundary: `Submission`s in, `Event`s out, and
//! the tool-call and approval shapes both carry. Each type is the neutral
//! subset of its namesake in cox's `cox-protocol`, with the same serde tags
//! and field names, so a cox rollout line for one of these events reads back
//! here and a surface maps one enum onto the other variant by variant.

use std::path::PathBuf;

use llm_wire::{ArchiveRef, CallId, ProviderError, Risk, StopReason, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{ItemId, TurnId};

/// A file, image or other blob attached to a `UserTurn`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    /// Display name (usually the original filename).
    pub name: String,
    /// MIME type, e.g. `"image/png"`.
    pub media_type: String,
    /// Base64-encoded bytes.
    pub data_b64: String,
}

/// A request into the loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Submission {
    /// Start a turn with user text; runs until the turn's `TurnDone`.
    UserTurn {
        /// The submitted text.
        text: String,
        /// Attached files/images.
        #[serde(default)]
        attachments: Vec<Attachment>,
    },
    /// Answer a pending `ApprovalRequired`.
    Approve {
        /// The call being decided.
        call_id: CallId,
        /// The decision.
        decision: Decision,
    },
    /// Cancel the running turn: the provider stream, the tools and any
    /// approval wait share one cancellation token.
    Interrupt,
}

/// What a transcript item is; carries what the item needs to be replayed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemKind {
    /// The user's turn text plus any attachments.
    UserMessage {
        /// The submitted text.
        text: String,
        /// Attached files/images, if any.
        attachments: Vec<Attachment>,
    },
    /// The assistant's visible reply text.
    AssistantMessage {
        /// The accumulated text (from `TextDelta`s); empty when started.
        text: String,
    },
    /// An extended-thinking block.
    Thinking {
        /// The accumulated thinking text; empty when started.
        text: String,
        /// The provider's signature for replaying the block, if it needs one.
        signature: Option<String>,
    },
}

/// A request from the model to run a tool, rated by the tool executor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Correlates `ToolCallRequested` through `ToolCallDone`.
    pub id: CallId,
    /// The tool's registered name.
    pub name: String,
    /// The model-supplied arguments. Untrusted: the model wrote them.
    pub input: Value,
    /// The call's risk classification, which the approval gate judges.
    pub risk: Risk,
    /// What approval rules match on: a path, command line, URL or name.
    pub subject: String,
    /// The simple commands a shell `subject` splits into; `None` when the
    /// subject is one unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segments: Option<Segments>,
}

/// A compound command line as an approval gate judges it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segments {
    /// Every simple command in source order.
    pub commands: Vec<String>,
    /// The split cannot vouch for the whole line, so no prefix rule or
    /// grant may allow it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub opaque: bool,
}

/// A unified diff for one file, for edit-shaped tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    /// The file the diff applies to.
    pub path: PathBuf,
    /// Unified diff text.
    pub unified: String,
}

/// The outcome of a finished tool call, as the model and surfaces see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// Whether the call succeeded.
    pub ok: bool,
    /// What the model sees. Untrusted tool output, capped by the loop at
    /// `Config::tool_output_cap`.
    pub visible: String,
    /// Where the untruncated output lives, if the executor archived it.
    pub archive: Option<ArchiveRef>,
    /// Size of the full (pre-truncation) output, in bytes.
    pub bytes: u64,
    /// Wall-clock time the call took; measured by the loop.
    pub duration_ms: u64,
    /// A unified diff, for edit-shaped tools.
    pub diff: Option<Diff>,
    /// The tool's machine-readable payload, for surfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<Box<Value>>,
}

impl ToolResult {
    /// A failed result whose only content is `msg`: an unknown tool, a
    /// denial, a call that never ran.
    pub fn failed(msg: &str) -> Self {
        Self {
            ok: false,
            visible: msg.into(),
            archive: None,
            bytes: msg.len() as u64,
            duration_ms: 0,
            diff: None,
            structured: None,
        }
    }
}

/// An approval decision, for both `Submission::Approve` and `ApprovalDecided`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Decision {
    /// Run the call once.
    Allow,
    /// Run this call and let the gate grant similar calls from now on.
    AllowForSession,
    /// Refuse the call.
    Deny {
        /// Shown to the model in the tool result.
        reason: String,
    },
    /// Run the call with edited input; it goes back through the gate.
    Edit {
        /// The replacement input.
        input: Value,
    },
}

/// Who decided an approval (`ApprovalDecided::by`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecidedBy {
    /// The user answered an `ApprovalRequired` prompt.
    User,
    /// A rule matched.
    Rule,
    /// A grant from earlier in the session matched.
    Session,
    /// A mode or policy decided without a rule (or an approval timed out).
    Policy,
    /// A hook decided.
    Hook,
}

/// Why an `ApprovalRequired` was raised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Why {
    /// An `ask` rule matched.
    RuleAsk {
        /// The rule that matched.
        rule: String,
    },
    /// No rule matched; the call's risk requires asking.
    Risk {
        /// The call's risk.
        risk: Risk,
    },
    /// A sandbox denied the call and the policy asks on failure.
    SandboxDenied {
        /// The sandbox's denial detail.
        detail: String,
    },
    /// The active policy forces asking regardless of risk.
    Policy {
        /// The policy's name (cox: `untrusted`, `on-request`, ...).
        policy: String,
    },
}

/// Severity of a `Notice`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Informational.
    Info,
    /// Something went wrong but the turn goes on.
    Warn,
}

/// Failures the loop reports or returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoopError {
    /// The turn was cancelled.
    #[error("interrupted")]
    Interrupted,
    /// The provider call failed.
    #[error("provider error: {error}")]
    Provider {
        /// The underlying provider failure.
        error: ProviderError,
    },
    /// The event sink could not take an event (cox: the rollout store).
    #[error("event sink failed: {message}")]
    Sink {
        /// The sink's error text.
        message: String,
    },
}

/// Everything a consumer can observe from the loop. Nothing is emitted
/// after a turn's `TurnDone`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A new turn began.
    TurnStarted {
        /// The turn's id.
        turn: TurnId,
        /// The turn's ordinal, 1-based.
        seq: u32,
    },
    /// A new transcript item began accumulating.
    ItemStarted {
        /// The item's id.
        item: ItemId,
        /// What kind of item this is.
        kind: ItemKind,
    },
    /// Streamed text for an `AssistantMessage` item.
    TextDelta {
        /// The item this delta belongs to.
        item: ItemId,
        /// The next chunk of text.
        text: String,
    },
    /// Streamed text for a `Thinking` item.
    ThinkingDelta {
        /// The item this delta belongs to.
        item: ItemId,
        /// The next chunk of thinking text.
        text: String,
    },
    /// A streamed `Thinking` item stopped growing.
    ThinkingDone {
        /// The `Thinking` item that ended.
        item: ItemId,
        /// Milliseconds from its first to its last delta.
        duration_ms: u64,
    },
    /// The model requested a tool call.
    ToolCallRequested {
        /// The requested call.
        call: ToolCall,
    },
    /// A tool call needs a decision before it can run.
    ApprovalRequired {
        /// The call awaiting a decision.
        call: ToolCall,
        /// Why it needs one.
        why: Why,
    },
    /// An approval was decided (by the user, or by the gate on a deny).
    ApprovalDecided {
        /// The call that was decided.
        call_id: CallId,
        /// The decision.
        decision: Decision,
        /// Who decided it.
        by: DecidedBy,
    },
    /// Streamed output from a running tool, for display only.
    ToolCallOutput {
        /// The call producing output.
        call_id: CallId,
        /// The next chunk of output.
        delta: String,
    },
    /// A tool call finished.
    ToolCallDone {
        /// The call that finished.
        call_id: CallId,
        /// Its result.
        result: ToolResult,
    },
    /// A transcript item finished accumulating.
    ItemDone {
        /// The item that finished.
        item: ItemId,
    },
    /// A provider call's usage.
    Usage {
        /// The turn this usage belongs to.
        turn: TurnId,
        /// The usage.
        usage: Usage,
    },
    /// An informational or warning message, not part of the transcript.
    Notice {
        /// Severity.
        level: Level,
        /// The message.
        text: String,
    },
    /// A turn finished.
    TurnDone {
        /// The turn that finished.
        turn: TurnId,
        /// Why it stopped.
        stop: StopReason,
    },
    /// An error occurred.
    Error {
        /// What went wrong.
        error: LoopError,
        /// Whether the whole session must end, or just the turn.
        fatal: bool,
    },
}

/// Where a turn is, for a status line or presence record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No turn in flight.
    Idle,
    /// Building the next provider request.
    Assembling,
    /// A provider stream is open.
    Streaming,
    /// Tools from the last assistant message are running.
    RunningTools,
    /// Waiting on `Submission::Approve`.
    AwaitingApproval,
    /// Emitting `TurnDone`.
    Finishing,
    /// An interrupt is ending the turn.
    Interrupted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// The serde shape is cox's: a cox rollout line for a shared event
    /// reads back here unchanged.
    #[test]
    fn events_keep_the_cox_wire_shape() {
        let call_id = CallId::new();
        let ev = Event::ApprovalDecided {
            call_id,
            decision: Decision::Deny {
                reason: "no".into(),
            },
            by: DecidedBy::User,
        };
        let json = serde_json::to_value(&ev).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "type": "approval_decided",
                "call_id": call_id.to_string(),
                "decision": {"type": "deny", "reason": "no"},
                "by": "user",
            })
        );
        let why: Why = serde_json::from_value(serde_json::json!({
            "type": "policy", "policy": "untrusted"
        }))
        .expect("cox ApprovalPolicy reads as a string");
        assert_eq!(
            why,
            Why::Policy {
                policy: "untrusted".into()
            }
        );
    }

    /// A cox `UserTurn` carries `confirm_think`; the loop ignores it.
    #[test]
    fn user_turn_reads_a_cox_submission() {
        let sub: Submission = serde_json::from_value(serde_json::json!({
            "type": "user_turn", "text": "hi", "attachments": [], "confirm_think": false
        }))
        .expect("deserialize");
        assert_eq!(
            sub,
            Submission::UserTurn {
                text: "hi".into(),
                attachments: vec![],
            }
        );
    }
}
