# Vendored API specs

## `anthropic-openapi.json`

Anthropic's OpenAPI 3.1 spec, the snapshot the Stainless SDK generator last
published for the Anthropic SDKs. `build.rs` generates the Anthropic wire types
(`src/wire.rs`) from it with typify; nothing else reads it. The file is
copied byte for byte from cox's `cox-provider-anthropic/schema/`.

| | |
| --- | --- |
| Source | https://storage.googleapis.com/stainless-sdk-openapi-specs/anthropic/anthropic-465bff21a179090915396565d1ae8f705cf8596e2ec920eb121072f25b8a7d68.yml |
| Downloaded | 2026-09-25 |
| sha256 | `1bb7c7a0a4a9bd1e342ecaba06c4abd1c94f53779ea970e971de6e093d6d1ad2` |

The file is JSON despite the `.yml` in the URL. The URL is a snapshot, not a
maintained pointer: `anthropic-sdk-python` stopped linking a spec URL in
`.stats.yml` on 2026-09-03 (commit `f9b0cf28`). A newer snapshot URL, when one
is published, goes in `SNAPSHOT_URL` in
`scripts/vendor/src/cox_vendor/anthropic_spec.py`.

This file is a vendored, non-package-manager artifact: no hand `curl`. cox's
saved script `scripts/vendor` (`cox-vendor anthropic-spec`, with `--check` to
report staleness) re-vendors its copy; this crate has no such script yet, so
a re-vendor here copies cox's file and updates the "Downloaded" and "sha256"
rows by hand.

A changed spec regenerates the types on the next build. A field cox sets that
the spec renamed or made required is a compile error in `request.rs`; the
request snapshots catch any change to the bytes cox sends.
