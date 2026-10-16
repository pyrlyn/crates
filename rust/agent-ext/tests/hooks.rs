// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! Shell hooks speak Claude Code's protocol and always fail open —
//! a crash, a bad exit or a timeout is a `Failed` outcome, never an error.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use agent_ext::hooks::{
    Hook, HookConfig, HookEvent, HookOutcome, HooksConfig, ShellHooks, with_context,
};
use serde_json::json;

fn hooks(event: &str, matcher: Option<&str>, command: &str, timeout_s: Option<u32>) -> ShellHooks {
    let mut events = HashMap::new();
    events.insert(
        event.to_string(),
        vec![HookConfig {
            matcher: matcher.map(str::to_string),
            command: command.to_string(),
            timeout_s,
        }],
    );
    let config = HooksConfig {
        events,
        ..HooksConfig::default()
    };
    ShellHooks::new(&config, PathBuf::from("."))
}

fn bash_call() -> serde_json::Value {
    json!({"session_id": "s", "cwd": ".", "hook_event_name": "PreToolUse", "tool_name": "bash", "tool_input": {"command": "git status"}})
}

#[tokio::test]
async fn hooks_pre_tool_use_exit_2_blocks_bash() {
    let h = hooks(
        "PreToolUse",
        Some("bash"),
        "echo 'not on my watch' >&2; exit 2",
        None,
    );
    let out = h
        .run(HookEvent::PreToolUse, bash_call(), Duration::from_secs(5))
        .await;
    assert_eq!(
        out,
        HookOutcome::Block {
            reason: "not on my watch".into()
        }
    );
    // The matcher keeps the same hook away from other tools.
    let mut other = bash_call();
    other["tool_name"] = json!("read");
    let out = h
        .run(HookEvent::PreToolUse, other, Duration::from_secs(5))
        .await;
    assert_eq!(out, HookOutcome::Continue);
}

#[tokio::test]
async fn hooks_crashing_hook_is_skipped_not_fatal() {
    let h = hooks("PreToolUse", None, "exit 7", None);
    let out = h
        .run(HookEvent::PreToolUse, bash_call(), Duration::from_secs(5))
        .await;
    assert!(
        matches!(out, HookOutcome::Failed { ref error } if error.starts_with("exit 7")),
        "{out:?}"
    );
    let h = hooks("PreToolUse", None, "sleep 30", Some(1));
    let out = h
        .run(HookEvent::PreToolUse, bash_call(), Duration::from_secs(5))
        .await;
    assert!(
        matches!(out, HookOutcome::Failed { ref error } if error.contains("timed out")),
        "{out:?}"
    );
    let h = hooks("PreToolUse", None, "/definitely/not/here", None);
    let out = h
        .run(HookEvent::PreToolUse, bash_call(), Duration::from_secs(5))
        .await;
    assert!(matches!(out, HookOutcome::Failed { .. }), "{out:?}");
}

// Needs `sh` and `cat`, which a Windows runner does not have on PATH.
#[cfg(unix)]
#[tokio::test]
async fn hooks_updated_input_is_applied() {
    // The hook reads the payload it was given and rewrites the command.
    let dir = tempfile::tempdir().unwrap();
    let seen = dir.path().join("payload.json");
    let cmd = format!(
        "cat > {} && echo '{{\"hookSpecificOutput\":{{\"hookEventName\":\"PreToolUse\",\"updatedInput\":{{\"command\":\"rtk git status\"}}}}}}'",
        seen.display()
    );
    let h = hooks("PreToolUse", Some("bash|edit"), &cmd, None);
    let out = h
        .run(HookEvent::PreToolUse, bash_call(), Duration::from_secs(5))
        .await;
    assert_eq!(
        out,
        HookOutcome::Modify {
            input: json!({"command": "rtk git status"})
        }
    );
    let payload: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(seen).unwrap()).unwrap();
    assert_eq!(payload["hook_event_name"], "PreToolUse");
    assert_eq!(payload["tool_input"]["command"], "git status");
}

#[tokio::test]
async fn hooks_unconfigured_event_and_plain_stdout_continue() {
    let h = hooks("PostToolUse", None, "echo looks fine", None);
    let out = h
        .run(HookEvent::PreToolUse, bash_call(), Duration::from_secs(5))
        .await;
    assert_eq!(out, HookOutcome::Continue);
    let out = h
        .run(HookEvent::PostToolUse, bash_call(), Duration::from_secs(5))
        .await;
    assert_eq!(out, HookOutcome::Continue);
}

#[test]
fn hooks_context_keeps_a_rewritten_prompt_and_joins_earlier_context() {
    assert_eq!(
        with_context(HookOutcome::Continue, ""),
        HookOutcome::Continue
    );
    assert_eq!(
        with_context(HookOutcome::Continue, "c"),
        HookOutcome::Modify {
            input: json!({"additional_context": "c"})
        }
    );
    assert_eq!(
        with_context(
            HookOutcome::Modify {
                input: json!("new prompt")
            },
            "c"
        ),
        HookOutcome::Modify {
            input: json!({"prompt": "new prompt", "additional_context": "c"})
        }
    );
    assert_eq!(
        with_context(
            HookOutcome::Modify {
                input: json!({"additional_context": "a"})
            },
            "b"
        ),
        HookOutcome::Modify {
            input: json!({"additional_context": "a\nb"})
        }
    );
}

#[test]
fn hooks_config_has_a_schema_and_reads_toml_shaped_events() {
    let schema = serde_json::to_value(schemars::schema_for!(HooksConfig)).unwrap();
    assert!(schema["properties"]["timeout_s"].is_object(), "{schema}");
    let config: HooksConfig = serde_json::from_value(json!({
        "timeout_s": 5,
        "PreToolUse": [{ "matcher": "bash", "command": "true" }],
    }))
    .unwrap();
    assert_eq!(config.timeout_s, 5);
    assert_eq!(config.events["PreToolUse"][0].command, "true");
    assert!(serde_json::from_value::<HookConfig>(json!({ "command": "x", "bogus": 1 })).is_err());
}
