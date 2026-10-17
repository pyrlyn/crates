# change-preview — completed tasks

### T1. The crate: edits, removals, totals, colour, JSON

Moved out of rtok's `src/render.rs` (`unified_diff`, `paint`) and `src/info.rs` (`human_bytes`) so rtok and ketch print one dry-run format (rtok plan T416). `Preview` collects edits (path, before, after; an unchanged file is dropped) and removals (path, bytes, files) and renders them in `Mode::Diff` (unified diffs, three lines of context, `a/` and `b/` headers) or `Mode::Stat` (`path | n +++--`, bars capped at 40 marks); removals print `- path  size  N files` in both modes; totals follow git's wording and grouping (`1 file changed`, `1 204 311 files`). `paint` colours through owo-colors' `if_supports_color`. Check: `cargo test` (10 tests) and `cargo clippy --all-targets -- -D warnings` green on Rust 1.86.

### T27. Preserve trailing newlines in coloured previews

`paint` now preserves one or more trailing newlines without changing line colouring. The regression test failed before the fix and passed after it; crate tests, clippy and formatting checks passed.

### T28. Reuse parsed line diff in change-preview

`Preview::edit` now shares one parsed `TextDiff` between change counts and unified rendering, while the public `unified_diff` function remains available. The same tests, clippy and formatting checks passed before and after the refactor.
