# abi-drift

Drift tests for generated bindings. Regenerate a C header with cbindgen, a C#
file with csbindgen, or any other generated text inside a test; compare it
with the committed copy; on a mismatch, fail with a unified diff and the
command that regenerates it. With the bless variable set, the test rewrites
the committed copy instead.

```rust
#[test]
fn the_committed_header_is_what_cbindgen_generates() -> Result<(), abi_drift::Error> {
    let dir = env!("CARGO_MANIFEST_DIR");
    let header = abi_drift::cbindgen_header(dir, "cbindgen.toml", "src/lib.rs")?;
    abi_drift::Drift::new(dir)
        .bless_from_env("MYAPP_BLESS")
        .regenerate_with("just bindings")
        .check("include/myapp.h", &header)?;
    Ok(())
}
```

`csbindgen_file(builder, scratch_path)` does the same for a configured
`csbindgen::Builder`. Line endings compare as LF, so a Windows checkout with
CRLF files does not count as drift. A missing committed file reads as empty.
The `cbindgen` and `csbindgen` features are both on by default; turn off the
one you do not use.

Extracted from scull's `scull-ffi` and ketch's `ketch-capi` drift tests.

Licensed under either of MIT or Apache-2.0 at your option.
