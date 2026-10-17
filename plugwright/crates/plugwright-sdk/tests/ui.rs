//! `#[plugin]` diagnostics as `rustc` prints them; `.stderr` files are the expected output.
//! Regenerate after an intended change with `TRYBUILD=overwrite cargo test -p plugwright-sdk --test ui`.

#[test]
fn plugin_diagnostics() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/plugin/pass.rs");
    t.compile_fail("tests/ui/plugin/fail/*.rs");
}
