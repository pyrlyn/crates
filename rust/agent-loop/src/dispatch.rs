//! One tool batch: announce every call, gate each through `Approvals` (and
//! the user when the gate asks), then run the approved ones: exclusive
//! calls one at a time first, then parallel calls up to
//! `Config::parallel_tools` at once. Results come back in the order the
//! model asked for them, whatever order they finished in.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use llm_wire::{CallId, Concurrency, Content, Message, Risk, Role};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::agent::Shared;
use crate::ids::{ItemId, TurnId};
use crate::traits::{ToolCx, Verdict};
use crate::types::{
    DecidedBy, Decision, Event, ItemKind, LoopError, State, ToolCall, ToolResult, Why,
};

/// Runs one batch; results are returned, and their `ToolCallDone`s
/// emitted, in the order the calls were requested.
pub(crate) async fn run_batch(
    shared: &Arc<Shared>,
    turn: TurnId,
    calls: Vec<(CallId, String, Value)>,
    signatures: &HashMap<CallId, String>,
) -> Result<Vec<(CallId, ToolResult)>, LoopError> {
    let order: Vec<CallId> = calls.iter().map(|(id, _, _)| *id).collect();
    let calls: Vec<(ToolCall, bool)> = calls
        .into_iter()
        .map(|(id, name, input)| {
            let rating = shared.parts.tools.rate(&name, &input);
            let known = rating.is_some();
            let rating = rating.unwrap_or_else(|| crate::traits::Rating {
                risk: Risk::ReadOnly,
                subject: String::new(),
                segments: None,
            });
            let call = ToolCall {
                id,
                name,
                input,
                risk: rating.risk,
                subject: rating.subject,
                segments: rating.segments,
            };
            (call, known)
        })
        .collect();
    for (call, _) in &calls {
        // The signature's block sits right before its call, the order the
        // assistant message in history keeps.
        if let Some(signature) = signatures.get(&call.id) {
            let item = ItemId::new();
            let kind = ItemKind::Thinking {
                text: String::new(),
                signature: Some(signature.clone()),
            };
            shared.emit(Event::ItemStarted { item, kind }).await?;
            shared.emit(Event::ItemDone { item }).await?;
        }
        shared
            .emit(Event::ToolCallRequested { call: call.clone() })
            .await?;
    }
    // Gate serially, so the user answers one prompt at a time and an
    // `AllowForSession` grant covers the calls behind it in the batch.
    let mut serial = Vec::new();
    let mut parallel = Vec::new();
    let mut done = HashMap::new();
    for (call, known) in calls {
        if !known {
            let result = ToolResult::failed(&format!("unknown tool {}", call.name));
            done.insert(call.id, result);
            continue;
        }
        let id = call.id;
        let call = match gate(shared, call).await? {
            Ok(call) => call,
            Err(result) => {
                done.insert(id, result);
                continue;
            }
        };
        match shared.parts.tools.concurrency(&call) {
            Concurrency::Exclusive => serial.push(call),
            Concurrency::Parallel => parallel.push(call),
        }
    }
    for call in serial {
        let result = run_one(shared, turn, &call).await;
        done.insert(call.id, result);
    }
    let cap = shared.config.parallel_tools.max(1) as usize;
    let mut set = JoinSet::new();
    let mut rest = parallel.into_iter();
    loop {
        while set.len() < cap {
            let Some(call) = rest.next() else { break };
            let shared = shared.clone();
            set.spawn(async move {
                let result = run_one(&shared, turn, &call).await;
                (call.id, result)
            });
        }
        let Some(joined) = set.join_next().await else {
            break;
        };
        match joined {
            Ok((id, result)) => {
                done.insert(id, result);
            }
            Err(_) => return Err(LoopError::Interrupted),
        }
    }
    let results: Vec<(CallId, ToolResult)> = order
        .into_iter()
        .map(|id| {
            let result = done
                .remove(&id)
                .unwrap_or_else(|| ToolResult::failed("tool did not return"));
            (id, result)
        })
        .collect();
    for (id, result) in &results {
        shared
            .emit(Event::ToolCallDone {
                call_id: *id,
                result: result.clone(),
            })
            .await?;
    }
    Ok(results)
}

/// Asks the gate and, when it escalates, the user. Returns the call to run
/// (its input may have been edited) or the failed result the model sees.
/// An allow emits nothing: it is the common case, and `ToolCallRequested`
/// already records the call.
async fn gate(shared: &Shared, call: ToolCall) -> Result<Result<ToolCall, ToolResult>, LoopError> {
    let id = call.id;
    let denied = |reason: &str| ToolResult::failed(&format!("permission denied: {reason}"));
    let approvals = shared.parts.approvals.as_ref();
    let original = call.input.clone();
    let mut call = match approvals.prepare(call).await {
        Ok(call) => call,
        Err(text) => return Ok(Err(ToolResult::failed(&text))),
    };
    if call.input != original {
        let input = std::mem::take(&mut call.input);
        rate(shared, &mut call, input);
    }
    loop {
        let why = match approvals.decide(&call).await {
            Verdict::Allow => return Ok(Ok(call)),
            Verdict::Deny { reason, by } => {
                shared
                    .emit(Event::ApprovalDecided {
                        call_id: id,
                        decision: Decision::Deny {
                            reason: reason.clone(),
                        },
                        by,
                    })
                    .await?;
                return Ok(Err(denied(&reason)));
            }
            Verdict::Ask(why) => why,
        };
        match ask(shared, &call, why).await? {
            Decision::Allow => return Ok(Ok(call)),
            Decision::AllowForSession => {
                approvals.grant(&call).await;
                return Ok(Ok(call));
            }
            Decision::Deny { reason } => return Ok(Err(denied(&reason))),
            // A rewritten input is a new call as far as the rules go: its
            // risk and subject change, so it goes back through `decide`.
            Decision::Edit { input } => rate(shared, &mut call, input),
        }
    }
}

/// Re-rates `call` for a rewritten `input`: risk, subject and segments
/// change together, or the gate would judge the new input by the old one.
fn rate(shared: &Shared, call: &mut ToolCall, input: Value) {
    if let Some(rating) = shared.parts.tools.rate(&call.name, &input) {
        call.risk = rating.risk;
        call.subject = rating.subject;
        call.segments = rating.segments;
    }
    call.input = input;
}

/// Emits `ApprovalRequired`, parks until `Submission::Approve` answers it,
/// and emits `ApprovalDecided`. An interrupt answers `Deny`, and so does
/// `Config::approval_timeout` running out.
async fn ask(shared: &Shared, call: &ToolCall, why: Why) -> Result<Decision, LoopError> {
    let id = call.id;
    let before = shared.state();
    shared.set_state(State::AwaitingApproval);
    let rx = shared.park(id);
    shared.parts.approvals.asking(call, &why).await;
    shared
        .emit(Event::ApprovalRequired {
            call: call.clone(),
            why,
        })
        .await?;
    let answer = async {
        rx.await.unwrap_or(Decision::Deny {
            reason: "session closed".into(),
        })
    };
    let timed = async {
        match shared.config.approval_timeout {
            Some(limit) => tokio::time::timeout(limit, answer).await.ok(),
            None => Some(answer.await),
        }
    };
    let cancel = shared.cancel_token();
    let (decision, by) = tokio::select! {
        biased;
        _ = cancel.cancelled() => (Decision::Deny { reason: "interrupted".into() }, DecidedBy::User),
        answered = timed => match answered {
            Some(decision) => (decision, DecidedBy::User),
            None => (Decision::Deny { reason: "approval timed out".into() }, DecidedBy::Policy),
        },
    };
    // An unanswered prompt must not stay answerable after it ended.
    shared.unpark(id);
    shared.set_state(before);
    shared
        .emit(Event::ApprovalDecided {
            call_id: id,
            decision: decision.clone(),
            by,
        })
        .await?;
    Ok(decision)
}

/// Runs one approved call, forwarding its streamed output, then caps what
/// the model sees. A call whose turn was interrupted before it started
/// never runs: nothing executes after a kill switch.
async fn run_one(shared: &Shared, turn: TurnId, call: &ToolCall) -> ToolResult {
    let cancel = shared.cancel_token();
    if cancel.is_cancelled() {
        return ToolResult::failed("interrupted before it ran");
    }
    let started = Instant::now();
    let (output, mut out_rx) = mpsc::channel::<String>(32);
    let cx = ToolCx {
        turn,
        cancel,
        output,
    };
    let run = shared.parts.tools.run(call, cx);
    tokio::pin!(run);
    // Output is display-only, so a sink that rejects it never fails the call.
    let mut result = loop {
        tokio::select! {
            biased;
            Some(delta) = out_rx.recv() => {
                let _ = shared.emit(Event::ToolCallOutput { call_id: call.id, delta }).await;
            }
            result = &mut run => break result,
        }
    };
    while let Ok(delta) = out_rx.try_recv() {
        let _ = shared
            .emit(Event::ToolCallOutput {
                call_id: call.id,
                delta,
            })
            .await;
    }
    result.duration_ms = started.elapsed().as_millis() as u64;
    cap(&mut result.visible, shared.config.tool_output_cap);
    result
}

/// Cuts `visible` to at most `limit` bytes on a char boundary and says so.
fn cap(visible: &mut String, limit: usize) {
    if visible.len() <= limit {
        return;
    }
    let total = visible.len();
    let end = visible.floor_char_boundary(limit);
    visible.truncate(end);
    visible.push_str(&format!("\n[output cut: {end} of {total} bytes shown]"));
}

/// The user message that carries a batch's results back to the model.
pub(crate) fn results_message(results: Vec<(CallId, ToolResult)>) -> Message {
    Message {
        role: Role::User,
        content: results
            .into_iter()
            .map(|(id, result)| Content::ToolResult {
                call_id: id,
                content: result.visible,
                is_error: !result.ok,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn cap_cuts_on_a_char_boundary_and_says_so() {
        let mut text = "ab\u{e9}cd".to_string(); // `é` is bytes 2..4
        cap(&mut text, 3);
        assert_eq!(text, "ab\n[output cut: 2 of 6 bytes shown]");
        let mut short = "ok".to_string();
        cap(&mut short, 3);
        assert_eq!(short, "ok");
    }
}
