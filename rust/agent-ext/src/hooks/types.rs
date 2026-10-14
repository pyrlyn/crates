// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! The host-neutral shapes the hook runner speaks: the event, the verdict,
//! the `[hooks]` config and the `Hook` trait a hook source implements.
//! Defined here so the runner needs no host crate; a host maps them to its
//! own protocol types (same names, same serde shape).

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which hook trigger point fired (Claude Code hook protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Before the user's text is pushed onto history; may block or rewrite it.
    UserPromptSubmit,
    /// Before a tool call runs; may `Block` or `Modify` its input.
    PreToolUse,
    /// After a tool call succeeds.
    PostToolUse,
    /// After a tool call fails.
    PostToolUseFailure,
    /// After a turn finishes normally.
    Stop,
    /// Before compaction runs; may `Block` it.
    PreCompact,
    /// After compaction runs.
    PostCompact,
    /// A session opened.
    SessionStart,
    /// A session closed.
    SessionEnd,
    /// The engine escalated a call to the user.
    PermissionRequest,
    /// A subagent started.
    SubagentStart,
    /// A subagent finished.
    SubagentStop,
    /// A notice was shown.
    Notification,
}

impl HookEvent {
    /// Claude Code's name for the event: the config key and `hook_event_name`.
    pub fn name(self) -> &'static str {
        match self {
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::Stop => "Stop",
            Self::PreCompact => "PreCompact",
            Self::PostCompact => "PostCompact",
            Self::SessionStart => "SessionStart",
            Self::SessionEnd => "SessionEnd",
            Self::PermissionRequest => "PermissionRequest",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::Notification => "Notification",
        }
    }
}

/// A hook runner's verdict for one hook invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookOutcome {
    /// Proceed unchanged.
    Continue,
    /// Stop the action the hook gated.
    Block {
        /// Shown to the user/model as the reason.
        reason: String,
    },
    /// Proceed with different input (e.g. a rewritten prompt or tool input).
    Modify {
        /// The replacement input.
        input: Value,
    },
    /// The hook itself failed to run; fail-open: the host skips it with a warning.
    Failed {
        /// What went wrong (timeout, non-zero exit, bad matcher).
        error: String,
    },
}

/// One `[[hooks.<Event>]]` entry.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct HookConfig {
    /// Tool/subject matcher (Claude Code hook matcher syntax), if any.
    pub matcher: Option<String>,
    /// The command to run.
    pub command: String,
    /// Per-hook timeout override, in seconds; falls back to `hooks.timeout_s`.
    pub timeout_s: Option<u32>,
}

/// `[hooks]`: the fixed `timeout_s`/`fail_open` keys plus every
/// `[[hooks.<Event>]]` table, captured generically since the event name is
/// the TOML key (`PreToolUse`, `PostToolUse`, ...) rather than a fixed field.
///
/// `deny_unknown_fields` is intentionally *not* set here: the flattened
/// `events` map is exactly what would otherwise be "unknown fields", so the
/// two are mutually exclusive for this one struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HooksConfig {
    /// Max seconds a hook process may run before it's treated as failed.
    pub timeout_s: u32,
    /// Whether a broken hook is skipped (warned) instead of fatal.
    pub fail_open: bool,
    /// `--no-hooks` sets this to `false`; not a default-config key.
    pub enabled: bool,
    /// Every `[[hooks.<Event>]]` array, keyed by event name.
    #[serde(flatten)]
    pub events: HashMap<String, Vec<HookConfig>>,
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            timeout_s: 60,
            fail_open: true,
            enabled: true,
            events: HashMap::new(),
        }
    }
}

/// A hook source (shell hooks, a host's plugin hooks, or a chain of them):
/// reports its verdict for one hook event. Never returns a `Result` — a
/// broken hook is always a `HookOutcome::Failed`, never a panic or a fatal
/// error ("fail open on extensions").
#[async_trait]
pub trait Hook: Send + Sync {
    /// Whether an observe-only trigger (`SessionStart`, `Notification`)
    /// should be dispatched to this source at all. The default is the
    /// `[hooks]` config check; a source configured elsewhere (a plugin's
    /// granted hooks) answers for itself, or it would never run.
    fn interested(&self, event: HookEvent, config: &HooksConfig) -> bool {
        config.events.contains_key(event.name())
    }

    /// Runs the hook for `event` with `payload`, giving up after `timeout`.
    async fn run(&self, event: HookEvent, payload: Value, timeout: Duration) -> HookOutcome;
}
