use std::collections::BTreeSet;
use std::path::PathBuf;

use path_gates::{Error, GateSpec, Rules, Unmatched};

const BASE: &str = r#"
[[gate]]
name = "rust"
paths = ["**/*.rs", "**/Cargo.toml", "Cargo.lock", "rust-toolchain.toml"]

[[gate]]
name = "docs"
paths = ["**/*.md"]
"#;

fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn rules(extra: &str) -> Rules {
    Rules::from_toml(&format!("{BASE}\n{extra}")).expect("valid config")
}

#[test]
fn matching_path_selects_its_gate() {
    let s = rules("").select(["src/lib.rs"]);
    assert_eq!(s.gates, names(&["rust"]));
    assert!(s.unmatched.is_empty());
    assert!(!s.all);
}

#[test]
fn path_claimed_by_several_gates_selects_all_of_them() {
    let rules = Rules::from_toml(
        r#"
[[gate]]
name = "a"
paths = ["src/**"]
[[gate]]
name = "b"
paths = ["**/*.rs"]
"#,
    )
    .unwrap();
    assert_eq!(rules.select(["src/x.rs"]).gates, names(&["a", "b"]));
}

#[test]
fn double_star_prefix_matches_the_root_too() {
    let r = rules("");
    assert_eq!(r.select(["README.md"]).gates, names(&["docs"]));
    assert_eq!(r.select(["a/b/README.md"]).gates, names(&["docs"]));
    assert_eq!(r.select(["Cargo.toml"]).gates, names(&["rust"]));
}

#[test]
fn single_star_does_not_cross_directories() {
    let r =
        Rules::from_toml("unmatched = \"ignore\"\n[[gate]]\nname = \"top\"\npaths = [\"*.md\"]")
            .unwrap();
    assert_eq!(r.select(["a.md"]).gates, names(&["top"]));
    assert!(r.select(["dir/a.md"]).gates.is_empty());
}

#[test]
fn unmatched_path_selects_every_gate_by_default() {
    let s = rules("").select(["src/lib.rs", "data.bin"]);
    assert!(s.all);
    assert_eq!(s.gates, names(&["rust", "docs"]));
    assert_eq!(s.unmatched, vec![PathBuf::from("data.bin")]);
}

#[test]
fn unmatched_all_is_the_explicit_default_too() {
    let s = Rules::from_toml(&format!("unmatched = \"all\"\n{BASE}"))
        .unwrap()
        .select(["data.bin"]);
    assert!(s.all);
    assert_eq!(s.gates, names(&["rust", "docs"]));
}

#[test]
fn unmatched_ignore_reports_but_does_not_select() {
    let r = Rules::from_toml(&format!("unmatched = \"ignore\"\n{BASE}")).unwrap();
    let s = r.select(["docs/a.md", "data.bin"]);
    assert!(!s.all);
    assert_eq!(s.gates, names(&["docs"]));
    assert_eq!(s.unmatched, vec![PathBuf::from("data.bin")]);
}

#[test]
fn always_gate_is_selected_whenever_anything_changed() {
    let r = Rules::from_toml(&format!(
        "unmatched = \"ignore\"\n{BASE}\n[[gate]]\nname = \"lint\"\nalways = true\n"
    ))
    .unwrap();
    assert_eq!(r.select(["x.bin"]).gates, names(&["lint"]));
    assert_eq!(r.select(["a.md"]).gates, names(&["docs", "lint"]));
}

#[test]
fn empty_input_selects_nothing_even_with_always() {
    let r = Rules::from_toml(&format!(
        "{BASE}\n[[gate]]\nname = \"lint\"\nalways = true\n"
    ))
    .unwrap();
    let s = r.select(Vec::<PathBuf>::new());
    assert!(s.gates.is_empty() && s.unmatched.is_empty() && !s.all);
}

#[test]
fn invalid_glob_is_a_config_error() {
    let err = Rules::from_toml("[[gate]]\nname = \"x\"\npaths = [\"[abc\"]")
        .err()
        .unwrap();
    assert!(matches!(err, Error::InvalidGlob { ref gate, .. } if gate == "x"));
}

#[test]
fn duplicate_gate_name_is_a_config_error() {
    let err = Rules::from_toml("[[gate]]\nname = \"x\"\n[[gate]]\nname = \"x\"")
        .err()
        .unwrap();
    assert!(matches!(err, Error::DuplicateGate(ref n) if n == "x"));
}

#[test]
fn malformed_toml_and_wrong_types_are_config_errors() {
    assert!(matches!(
        Rules::from_toml("[[gate").err().unwrap(),
        Error::Config(_)
    ));
    assert!(matches!(
        Rules::from_toml("[[gate]]\nname = 1").err().unwrap(),
        Error::Config(_)
    ));
}

#[test]
fn unknown_keys_are_ignored_at_both_levels() {
    let r = Rules::from_toml(
        "base = \"origin/main\"\n[[gate]]\nname = \"rust\"\npaths = [\"**/*.rs\"]\nrun = \"cargo test\"\n",
    )
    .unwrap();
    assert_eq!(r.select(["a.rs"]).gates, names(&["rust"]));
}

#[test]
fn new_builds_rules_without_toml() {
    let spec = |name: &str, path: &str, always| GateSpec {
        name: name.to_owned(),
        paths: vec![path.to_owned()],
        always,
    };
    let r = Rules::new(
        vec![spec("rust", "**/*.rs", false), spec("lint", "none", true)],
        Unmatched::Ignore,
    )
    .unwrap();
    assert_eq!(r.select(["a.rs"]).gates, names(&["rust", "lint"]));
    let dup = Rules::new(
        vec![spec("a", "x", false), spec("a", "y", false)],
        Unmatched::All,
    );
    assert!(matches!(dup.err().unwrap(), Error::DuplicateGate(_)));
    let bad = Rules::new(vec![spec("a", "[x", false)], Unmatched::All);
    assert!(matches!(bad.err().unwrap(), Error::InvalidGlob { .. }));
}

#[test]
fn windows_separators_and_dot_prefix_are_normalised() {
    let r = rules("");
    let s = r.select(["crates\\core\\src\\lib.rs", ".\\Cargo.lock", "./docs/a.md"]);
    assert_eq!(s.gates, names(&["rust", "docs"]));
    assert!(!s.all);
    let s = r.select(["assets\\logo.png"]);
    assert_eq!(s.unmatched, vec![PathBuf::from("assets/logo.png")]);
}

#[test]
fn repeated_dot_and_empty_segments_are_collapsed() {
    let r =
        Rules::from_toml("unmatched = \"ignore\"\n[[gate]]\nname = \"top\"\npaths = [\"*.rs\"]")
            .unwrap();
    for path in ["././foo.rs", ".//foo.rs", "foo/../bar.rs"] {
        let s = r.select([path]);
        assert_eq!(s.gates, names(&["top"]), "{path}");
        assert!(s.unmatched.is_empty(), "{path}");
    }
}

#[test]
fn gate_names_keep_config_order() {
    let r = rules("");
    assert_eq!(r.gate_names().collect::<Vec<_>>(), ["rust", "docs"]);
}

#[test]
fn config_without_gates_flags_everything_as_unmatched() {
    let r = Rules::from_toml("").unwrap();
    let s = r.select(["a.rs"]);
    assert!(s.all);
    assert!(s.gates.is_empty());
}
