//! Compile-fail tests: misuse must be rejected by the compiler with the
//! message pinned in `tests/ui/*.stderr` (regenerate with
//! `TRYBUILD=overwrite cargo test --test compile_fail` after a toolchain bump).

#[test]
fn misuse_is_a_compile_error_with_a_fix_hint() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
