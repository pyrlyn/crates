//! Every mode against a fresh, a different and a missing file in a scratch directory.

use std::fs;

use bless_check::{Error, Mode, Outcome, assert_fresh, check_or_bless};
use tempfile::TempDir;

fn scratch(committed: Option<&str>) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("schema.json");
    if let Some(text) = committed {
        fs::write(&path, text).unwrap();
    }
    (dir, path)
}

#[test]
fn a_matching_file_is_fresh_in_every_mode_and_never_rewritten() {
    for mode in [Mode::Check, Mode::CreateMissing, Mode::Bless] {
        let (_dir, path) = scratch(Some("a\nb\n"));
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            check_or_bless(&path, "a\nb\n", mode).unwrap(),
            Outcome::Fresh
        );
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }
}

#[test]
fn crlf_in_either_side_reads_as_lf() {
    let (_dir, path) = scratch(Some("a\r\nb\r\n"));
    assert_eq!(
        check_or_bless(&path, "a\nb\n", Mode::Check).unwrap(),
        Outcome::Fresh
    );
    let (_dir, path) = scratch(Some("a\nb\n"));
    assert_eq!(
        check_or_bless(&path, b"a\r\nb\r\n", Mode::Check).unwrap(),
        Outcome::Fresh
    );
}

#[test]
fn check_reports_the_first_differing_line() {
    let (_dir, path) = scratch(Some("a\nold\nc\n"));
    let err = check_or_bless(&path, "a\nnew\nc\n", Mode::Check).unwrap_err();
    assert!(err.is_stale());
    match &err {
        Error::Differs {
            line,
            committed,
            rendered,
            ..
        } => {
            assert_eq!(*line, 2);
            assert_eq!(committed.as_deref(), Some("old"));
            assert_eq!(rendered.as_deref(), Some("new"));
        }
        other => panic!("{other:?}"),
    }
    let text = err.to_string();
    assert!(text.contains("is stale: line 2 differs"), "{text}");
    assert!(text.contains("committed: \"old\""), "{text}");
}

#[test]
fn a_missing_trailing_newline_is_a_difference_past_the_end() {
    let (_dir, path) = scratch(Some("a"));
    match check_or_bless(&path, "a\n", Mode::Check).unwrap_err() {
        Error::Differs {
            line,
            committed,
            rendered,
            ..
        } => {
            assert_eq!(line, 2);
            assert_eq!(committed, None);
            assert_eq!(rendered.as_deref(), Some(""));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_long_line_is_cut_in_the_report() {
    let (_dir, path) = scratch(Some(&"x".repeat(1000)));
    let text = check_or_bless(&path, "y", Mode::Check)
        .unwrap_err()
        .to_string();
    assert!(text.len() < 500, "{text}");
    assert!(text.contains('…'));
}

#[test]
fn check_fails_on_a_missing_file_and_writes_nothing() {
    let (_dir, path) = scratch(None);
    let err = check_or_bless(&path, "a\n", Mode::Check).unwrap_err();
    assert!(matches!(err, Error::Missing { .. }));
    assert!(err.to_string().ends_with("schema.json is missing"));
    assert!(!path.exists());
}

#[test]
fn create_missing_writes_a_missing_file_but_fails_on_a_different_one() {
    let (dir, _) = scratch(None);
    let nested = dir.path().join("docs/new/schema.json");
    assert_eq!(
        check_or_bless(&nested, "a\n", Mode::CreateMissing).unwrap(),
        Outcome::Written
    );
    assert_eq!(fs::read_to_string(&nested).unwrap(), "a\n");

    let (_dir, path) = scratch(Some("old\n"));
    assert!(
        check_or_bless(&path, "new\n", Mode::CreateMissing)
            .unwrap_err()
            .is_stale()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "old\n");
}

#[test]
fn bless_rewrites_a_different_file_and_creates_a_missing_one() {
    let (_dir, path) = scratch(Some("old\n"));
    assert_eq!(
        check_or_bless(&path, "new\n", Mode::Bless).unwrap(),
        Outcome::Written
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");

    let (_dir, path) = scratch(None);
    assert_eq!(
        check_or_bless(&path, "new\n", Mode::Bless).unwrap(),
        Outcome::Written
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
}

#[test]
fn an_unreadable_path_is_an_io_error_not_a_stale_file() {
    let (dir, _) = scratch(None);
    let err = check_or_bless(dir.path(), "a\n", Mode::Check).unwrap_err();
    assert!(!err.is_stale(), "{err:?}");
    assert!(
        matches!(
            err,
            Error::Io {
                action: "reading",
                ..
            }
        ),
        "{err:?}"
    );
    assert!(std::error::Error::source(&err).is_some());
}

#[test]
fn from_env_blesses_only_when_the_variable_is_set() {
    // A name no other test or host sets; reading it is all this test does.
    assert_eq!(
        Mode::from_env("BLESS_CHECK_TEST_UNSET_VARIABLE"),
        Mode::Check
    );
    assert_eq!(Mode::from_env("PATH"), Mode::Bless);
}

#[test]
#[should_panic(expected = "run `just gen`")]
fn assert_fresh_panics_with_the_hint() {
    let (_dir, path) = scratch(Some("old\n"));
    assert_fresh(&path, "new\n", Mode::Check, "run `just gen`");
}

#[test]
fn assert_fresh_passes_a_fresh_file() {
    let (_dir, path) = scratch(Some("same\n"));
    assert_fresh(&path, "same\n", Mode::Check, "unused");
}
