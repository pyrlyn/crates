//! `#[interface]` diagnostics as `rustc` prints them; `.stderr` files are the expected output.
//! Regenerate after an intended change with `TRYBUILD=overwrite cargo test -p plugwright --test ui`.

#[test]
fn interface_diagnostics() {
    let t = trybuild::TestCases::new();
    t.pass("tests/ui/interface/pass.rs");
    t.compile_fail("tests/ui/interface/fail/*.rs");
}
