//! Each rule is proved on a synthetic graph that breaks it; `load` is proved on
//! this workspace, whose members and edges are known.

use workspace_graph::{Graph, Kind};

fn graph(rows: &[(&str, &[&str])]) -> Graph {
    rows.iter()
        .map(|(name, deps)| (*name, deps.iter().copied()))
        .collect()
}

fn messages(result: Result<(), workspace_graph::Violations>) -> Vec<String> {
    result
        .err()
        .map(|v| v.messages().to_vec())
        .unwrap_or_default()
}

#[test]
fn load_reads_this_workspace_with_the_chosen_kinds() {
    let normal = Graph::load(env!("CARGO_MANIFEST_DIR"), &[Kind::Normal]).unwrap();
    assert!(normal.members().any(|m| m == "workspace-graph"));
    normal.assert_exact("workspace-graph", &["cargo_metadata", "thiserror"]);
    let scoped = normal.deps("scoped-check").unwrap();
    assert!(scoped.contains("path-gates"));
    assert!(
        !scoped.contains("assert_cmd"),
        "dev-dependencies are not normal"
    );

    let dev = Graph::load(env!("CARGO_MANIFEST_DIR"), &[Kind::Dev]).unwrap();
    let scoped = dev.deps("scoped-check").unwrap();
    assert!(scoped.contains("assert_cmd"));
    assert!(!scoped.contains("path-gates"));
}

#[test]
fn load_reports_a_directory_outside_any_workspace() {
    let dir = std::env::temp_dir().join("workspace-graph-no-such-dir");
    let err = Graph::load(&dir, &[Kind::Normal]).unwrap_err();
    assert!(
        err.to_string().starts_with("cargo metadata failed"),
        "{err}"
    );
}

#[test]
fn workspace_only_drops_external_edges() {
    let g = graph(&[("app", &["core", "serde"]), ("core", &["thiserror"])]);
    let inner = g.workspace_only();
    assert_eq!(inner.deps("app").unwrap().len(), 1);
    assert!(inner.deps("core").unwrap().is_empty());
    assert_eq!(inner.members().collect::<Vec<_>>(), ["app", "core"]);
}

#[test]
fn only_dependents_names_each_outsider() {
    let g = graph(&[
        ("store", &["diesel"]),
        ("agent", &["diesel", "serde"]),
        ("ui", &["libsqlite3-sys"]),
    ]);
    assert_eq!(
        messages(g.check_only_dependents(&["diesel", "libsqlite3-sys"], &["store", "planned"])),
        [
            "agent must not depend on diesel; only store, planned may",
            "ui must not depend on libsqlite3-sys; only store, planned may",
        ]
    );
    assert_eq!(
        messages(g.check_only_dependents(&["store"], &[])),
        Vec::<String>::new()
    );
    let g = graph(&[("a", &["ffi"]), ("ffi", &[])]);
    assert_eq!(
        messages(g.check_only_dependents(&["ffi"], &[])),
        ["a must not depend on ffi; nothing may"]
    );
}

#[test]
fn forbidden_names_each_banned_edge_and_an_unknown_member() {
    let g = graph(&[("term", &["pty", "grid", "ffi"])]);
    assert_eq!(
        messages(g.check_forbidden("term", &["ffi", "pty"])),
        ["term must not depend on ffi", "term must not depend on pty"]
    );
    assert!(g.check_forbidden("term", &["io"]).is_ok());
    assert_eq!(
        messages(g.check_forbidden("ghost", &["pty"])),
        ["ghost is not a workspace member"]
    );
}

#[test]
fn exact_reports_extra_and_missing_dependencies() {
    let g = graph(&[("core", &["serde", "tokio"]), ("leaf", &[])]);
    assert!(g.check_exact("core", &["tokio", "serde"]).is_ok());
    assert!(g.check_exact("leaf", &[]).is_ok());
    assert_eq!(
        messages(g.check_exact("core", &["serde", "thiserror"])),
        [
            "core depends on tokio, which is not expected",
            "core does not depend on thiserror, which is expected",
        ]
    );
    assert_eq!(
        messages(g.check_exact("ghost", &[])),
        ["ghost is not a workspace member"]
    );
}

#[test]
fn layers_allow_down_and_sideways_and_report_up_and_unplaced() {
    let g = graph(&[
        ("unicode", &[]),
        ("parser", &["unicode", "grid", "memchr"]),
        ("grid", &["unicode", "term"]),
        ("term", &["parser"]),
        ("stray", &[]),
    ]);
    let layers: &[&[&str]] = &[&["unicode"], &["parser", "grid", "planned"], &["term"]];
    assert_eq!(
        messages(g.check_layers(layers)),
        [
            "grid (layer 1) must not depend on term (layer 2)",
            "stray is in no layer",
        ]
    );
}

#[test]
fn violations_display_one_per_line() {
    let g = graph(&[("a", &["x", "y"])]);
    let err = g.check_forbidden("a", &["x", "y"]).unwrap_err();
    assert_eq!(
        err.to_string(),
        "a must not depend on x\na must not depend on y"
    );
}

#[test]
#[should_panic(expected = "a must not depend on x")]
fn assert_forms_panic_with_the_messages() {
    graph(&[("a", &["x"])]).assert_forbidden("a", &["x"]);
}

#[test]
fn assert_forms_pass_a_clean_graph() {
    let g = graph(&[("core", &[]), ("app", &["core"])]);
    g.assert_exact("app", &["core"]);
    g.assert_forbidden("core", &["app"]);
    g.assert_only_dependents(&["core"], &["app"]);
    g.assert_layers(&[&["core"], &["app"]]);
}
