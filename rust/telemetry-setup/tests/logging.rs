// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later

//! The subscriber end to end: files, filters, masking and the OTLP switch, as `aulo-telemetry`
//! pinned them.

use std::fs;
use std::path::Path;

use telemetry_setup::{Error, Settings, subscriber};

const APP: &str = "demo";

fn settings(dir: &Path, level: &str) -> Settings {
    let mut settings = Settings::new(APP, level, dir);
    settings.stderr = false;
    settings
}

/// Runs `emit` under a fresh subscriber and returns the log file's text once flushed.
fn logged(settings: &Settings, emit: impl FnOnce()) -> String {
    let (subscriber, guard) = subscriber(settings).unwrap();
    tracing::subscriber::with_default(subscriber, emit);
    drop(guard);
    let mut text = String::new();
    for entry in fs::read_dir(&settings.log_dir).unwrap() {
        text.push_str(&fs::read_to_string(entry.unwrap().path()).unwrap());
    }
    text
}

#[test]
fn writes_a_rotating_json_file_named_by_app_and_date() {
    let dir = tempfile::tempdir().unwrap();
    let log_dir = dir.path().join("nested").join("logs");
    let text = logged(&settings(&log_dir, "info"), || {
        tracing::info!(turn = 3, "hello")
    });

    let names: Vec<_> = fs::read_dir(&log_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(
        names[0].starts_with("demo.") && names[0].ends_with(".log"),
        "{names:?}"
    );

    let record: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(record["fields"]["message"], "hello");
    assert_eq!(record["fields"]["turn"], 3);
    assert_eq!(record["level"], "INFO");
}

#[test]
fn secrets_never_reach_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let text = logged(&settings(dir.path(), "info"), || {
        let span = tracing::info_span!("call", api_key = "sk-live-0123456789abcdefXYZ");
        let _entered = span.enter();
        tracing::info!(
            password = "hunter2",
            user = "ada",
            "got Bearer abcdef0123456789"
        );
        tracing::warn!(note = "key sk-ant-api03-abcdefghijklmnop");
    });
    for leaked in ["hunter2", "abcdef0123456789", "sk-live", "sk-ant-api03"] {
        assert!(!text.contains(leaked), "{leaked} leaked: {text}");
    }
    assert!(text.contains("ada"), "{text}");
    assert!(text.contains("[REDACTED]"), "{text}");
}

#[test]
fn extra_patterns_reach_the_file_writer() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = settings(dir.path(), "info");
    settings.extra_secret_patterns = vec![r"\bdemo_[0-9a-f]{16,}".into()];
    let text = logged(&settings, || {
        tracing::info!(note = "using demo_0123456789abcdef0123", "call");
    });
    assert!(!text.contains("demo_0123456789abcdef"), "{text}");
    assert!(text.contains("[REDACTED]"), "{text}");
}

#[test]
fn config_level_filters_events() {
    let dir = tempfile::tempdir().unwrap();
    let text = logged(&settings(dir.path(), "warn"), || {
        tracing::info!("quiet");
        tracing::warn!("loud");
    });
    assert!(text.contains("loud") && !text.contains("quiet"), "{text}");
}

#[test]
fn override_beats_the_config_level() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = settings(dir.path(), "error");
    settings.filter_override = Some("debug".into());
    let text = logged(&settings, || tracing::debug!("detail"));
    assert!(text.contains("detail"), "{text}");
}

#[test]
fn an_unset_filter_variable_leaves_the_level_in_charge() {
    let settings = Settings::new(APP, "info", "logs")
        .filter_from_env("TELEMETRY_SETUP_TEST_VARIABLE_THAT_IS_NEVER_SET");
    assert_eq!(settings.filter_override, None);
}

#[test]
fn invalid_filter_is_an_error_naming_the_directive() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = settings(dir.path(), "info");
    settings.filter_override = Some("demo=notalevel".into());
    match subscriber(&settings) {
        Err(Error::Filter { directive, .. }) => assert_eq!(directive, "demo=notalevel"),
        other => panic!("expected a filter error, got {:?}", other.err()),
    }
}

#[test]
fn an_app_name_that_is_not_a_plain_file_name_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    for app in ["", "../escape", "a/b", ".hidden", "a b"] {
        let mut settings = settings(dir.path(), "info");
        settings.app = app.into();
        match subscriber(&settings) {
            Err(Error::AppName(name)) => assert_eq!(name, app),
            other => panic!(
                "expected an app name error for {app:?}, got {:?}",
                other.err()
            ),
        }
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[cfg(not(feature = "otlp"))]
#[test]
fn otlp_without_the_feature_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = settings(dir.path(), "info");
    settings.otlp_endpoint = Some("http://localhost:4318/v1/traces".into());
    assert!(matches!(subscriber(&settings), Err(Error::OtlpUnavailable)));
}

#[cfg(feature = "otlp")]
#[test]
fn otlp_layer_builds_and_shuts_down_with_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = settings(dir.path(), "info");
    settings.otlp_endpoint = Some("http://127.0.0.1:4318/v1/traces".into());
    // No span is opened, so the exporter has nothing to send and the test opens no socket.
    let text = logged(&settings, || {
        tracing::info!(token = "secret-value", "outside any span");
    });
    assert!(
        text.contains("outside any span") && !text.contains("secret-value"),
        "{text}"
    );
}
