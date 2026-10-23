//! The same masking check as `llm-http`'s cassette tests, through this crate's
//! re-export. No socket.

use llm_testkit::cassette::Cassette;

const REQUEST: &str = r#"{"authorization":"Bearer sk-fixturevalue1234","model":"m"}"#;

#[test]
fn a_saved_cassette_keeps_no_token_and_still_replays() {
    let mut cassette = Cassette::new();
    cassette.record(
        REQUEST,
        200,
        r#"{"text":"hello","key":"sk-ant-api03-abcdef123456"}"#,
    );
    let saved = cassette.save();
    assert!(!saved.contains("sk-fixturevalue1234"));
    assert!(!saved.contains("sk-ant-api03-abcdef123456"));
    assert!(saved.contains("redacted"));

    let loaded = Cassette::load(&saved).expect("saved text loads");
    let answer = loaded.replay(REQUEST).expect("same request replays");
    assert_eq!(answer.status, 200);
    assert!(answer.body.contains("hello"));
}
