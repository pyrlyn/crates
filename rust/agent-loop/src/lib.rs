//! A neutral LLM agent turn loop, extracted from cox's `cox-core`.
//! `Submission`s go in, `Event`s come out; one turn is a sequence of
//! provider calls, each followed by its tool batch, until the model stops
//! calling tools, the step limit is hit or the turn is interrupted.
//!
//! - [`Agent`] — the state machine: `submit` a `Submission`, read `Event`s from the sink.
//! - [`ids`] — `TurnId`, `ItemId`.
//! - [`types`] — `Submission`, `Event` and the tool-call, approval and error shapes.
//! - [`traits`] — the seams: `Tools`, `Approvals`, `Context`, `EventSink`.
//!
//! The model behind the loop is any `llm_wire::Provider`.

mod agent;
mod dispatch;
pub mod ids;
mod stream;
pub mod traits;
pub mod types;

#[cfg(test)]
mod testkit;

pub use agent::{Agent, Config, Parts};

pub use ids::{ItemId, TurnId};
pub use traits::{Approvals, Context, EventSink, Next, Rating, ToolCx, Tools, Verdict};
pub use types::{
    Attachment, DecidedBy, Decision, Diff, Event, ItemKind, Level, LoopError, Segments, State,
    Submission, ToolCall, ToolResult, Why,
};
